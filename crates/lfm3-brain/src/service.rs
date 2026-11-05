//! Bounded request and streaming boundary. This is protocol logic, not an inference backend.
use crate::MODEL_ID;
use serde::{Deserialize, Serialize};
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Instant,
};

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub request_bytes: usize,
    pub context_bytes: usize,
    pub context_tokens: usize,
    pub output_tokens: usize,
    pub output_bytes: usize,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub id: String,
    pub model: String,
    pub messages: Vec<Message>,
    pub max_new_tokens: usize,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Message {
    pub role: String,
    pub content: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    Started {
        id: String,
        model: String,
    },
    Token {
        id: String,
        index: usize,
        text: String,
    },
    Finished {
        id: String,
        tokens: usize,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum Error {
    Invalid(&'static str),
    BackendUnavailable,
    Cancelled,
    Deadline,
    OutputLimit,
    Sink,
}

/// `context_tokens` must use the selected tokenizer. Both calls must fail with
/// `BackendUnavailable` until a real model backend is wired. `stream` invokes
/// the callback once per generated token; cancellation is cooperative between
/// callbacks, so each backend step must itself be bounded.
pub trait Backend {
    fn context_tokens(&self, request: &Request) -> Result<usize, Error>;
    fn stream(
        &self,
        request: &Request,
        token: &mut dyn FnMut(&str) -> Result<(), Error>,
    ) -> Result<(), Error>;
}
pub trait Sink {
    fn send(&mut self, event: Event) -> Result<(), Error>;
}

pub struct Control<'a> {
    pub cancelled: &'a AtomicBool,
    pub deadline: Option<Instant>,
}

fn validate(r: &Request, l: Limits) -> Result<(), Error> {
    if r.id.is_empty()
        || r.id.len() > 128
        || !r
            .id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-.:".contains(&b))
    {
        return Err(Error::Invalid("request id"));
    }
    if r.model != MODEL_ID {
        return Err(Error::Invalid("model id"));
    }
    if r.messages.is_empty()
        || r.messages
            .iter()
            .any(|m| !["system", "user", "assistant"].contains(&m.role.as_str()))
    {
        return Err(Error::Invalid("messages"));
    }
    let context = r
        .messages
        .iter()
        .try_fold(0usize, |n, m| n.checked_add(m.content.len()))
        .ok_or(Error::Invalid("context size"))?;
    if context > l.context_bytes {
        return Err(Error::Invalid("context limit"));
    }
    if r.max_new_tokens == 0 || r.max_new_tokens > l.output_tokens {
        return Err(Error::Invalid("output token limit"));
    }
    Ok(())
}

pub fn parse(input: &[u8], limits: Limits) -> Result<Request, Error> {
    if input.len() > limits.request_bytes {
        return Err(Error::Invalid("request byte limit"));
    }
    let request: Request =
        serde_json::from_slice(input).map_err(|_| Error::Invalid("request JSON"))?;
    validate(&request, limits)?;
    Ok(request)
}

pub fn serve(
    input: &[u8],
    limits: Limits,
    backend: &dyn Backend,
    sink: &mut dyn Sink,
    control: &Control<'_>,
) -> Result<(), Error> {
    let check = |c: &Control<'_>| {
        if c.cancelled.load(Ordering::Relaxed) {
            Err(Error::Cancelled)
        } else if c.deadline.is_some_and(|d| Instant::now() >= d) {
            Err(Error::Deadline)
        } else {
            Ok(())
        }
    };
    check(control)?;
    let request = parse(input, limits)?;
    let context_tokens = backend.context_tokens(&request)?;
    if context_tokens
        .checked_add(request.max_new_tokens)
        .is_none_or(|total| total > limits.context_tokens)
    {
        return Err(Error::Invalid("context token limit"));
    }
    check(control)?;
    sink.send(Event::Started {
        id: request.id.clone(),
        model: request.model.clone(),
    })?;
    let (mut count, mut bytes) = (0usize, 0usize);
    let mut emit = |text: &str| {
        check(control)?;
        let next_count = count.checked_add(1).ok_or(Error::OutputLimit)?;
        let next_bytes = bytes.checked_add(text.len()).ok_or(Error::OutputLimit)?;
        if next_count > request.max_new_tokens || next_bytes > limits.output_bytes {
            return Err(Error::OutputLimit);
        }
        sink.send(Event::Token {
            id: request.id.clone(),
            index: count,
            text: text.to_owned(),
        })?;
        count = next_count;
        bytes = next_bytes;
        Ok(())
    };
    backend.stream(&request, &mut emit)?;
    check(control)?;
    sink.send(Event::Finished {
        id: request.id,
        tokens: count,
    })
}

#[cfg(test)]
#[path = "service_tests.rs"]
mod tests;
