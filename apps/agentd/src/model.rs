//! The model client: OpenAI-compatible `chat/completions` over a raw TCP
//! connection. Plain HTTP only (the homelab gateway and the on-device llama.cpp
//! server are both local), with a hard wall-clock timeout per attempt so a dead
//! brain becomes a journal event instead of a hung request.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::time::Duration;

use serde_json::{json, Value};

pub const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;

/// Why a brain could not answer. The reason goes into the journal verbatim —
/// "can't answer, here's why" is a requirement, not a nice-to-have.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModelError {
    Connect(String),
    Timeout,
    /// Non-2xx HTTP status with a short body excerpt.
    Http(u16, String),
    /// 2xx but not the shape we need.
    BadResponse(String),
    /// A configured brain needs a key we could not read.
    NoKey(String),
}

impl ModelError {
    pub fn reason(&self) -> String {
        match self {
            ModelError::Connect(m) => format!("connect failed: {m}"),
            ModelError::Timeout => "model call timed out".to_string(),
            ModelError::Http(status, body) => format!("HTTP {status}: {body}"),
            ModelError::BadResponse(m) => format!("malformed model response: {m}"),
            ModelError::NoKey(m) => format!("no key: {m}"),
        }
    }
}

/// One tool call as the model asked for it. `arguments` is the raw string the
/// model produced — it is *not* trusted to be JSON until it parses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChatReply {
    pub content: Option<String>,
    pub tool_calls: Vec<ToolCall>,
    pub finish_reason: Option<String>,
    pub model: Option<String>,
    pub usage: Vec<(String, i64)>,
}

/// Split `http://host:port/path`. Only http is supported; anything else is a
/// configuration error, not a runtime surprise.
fn split_url(url: &str) -> Result<(String, u16, String), ModelError> {
    let rest = url
        .strip_prefix("http://")
        .ok_or_else(|| ModelError::Connect(format!("unsupported URL scheme in `{url}` (http only)")))?;
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    let (host, port) = authority
        .rsplit_once(':')
        .ok_or_else(|| ModelError::Connect(format!("URL `{url}` has no port")))?;
    let port: u16 = port
        .parse()
        .map_err(|_| ModelError::Connect(format!("URL `{url}` has a non-numeric port")))?;
    Ok((host.to_string(), port, format!("/{path}")))
}

/// POST one chat request and parse the reply.
pub fn chat(
    url: &str,
    bearer: Option<&str>,
    payload: &Value,
    timeout: Duration,
) -> Result<ChatReply, ModelError> {
    let (host, port, path) = split_url(url)?;
    let addr = (host.as_str(), port)
        .to_socket_addrs()
        .map_err(|e| ModelError::Connect(format!("resolve {host}:{port}: {e}")))?
        .next()
        .ok_or_else(|| ModelError::Connect(format!("{host}:{port} resolved to nothing")))?;
    let addr: SocketAddr = addr;

    let body = payload.to_string();
    let mut stream = TcpStream::connect_timeout(&addr, timeout)
        .map_err(|e| ModelError::Connect(format!("{addr}: {e}")))?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|e| ModelError::Connect(format!("read timeout: {e}")))?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(|e| ModelError::Connect(format!("write timeout: {e}")))?;

    let auth = match bearer {
        Some(k) => format!("Authorization: Bearer {k}\r\n"),
        None => String::new(),
    };
    let head = format!(
        "POST {path} HTTP/1.1\r\nHost: {addr}\r\nUser-Agent: agentd/0.1\r\n\
         Content-Type: application/json\r\n{auth}Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).map_err(|e| ModelError::Connect(format!("write: {e}")))?;
    stream.write_all(body.as_bytes()).map_err(|e| ModelError::Connect(format!("write body: {e}")))?;
    stream.flush().map_err(|e| ModelError::Connect(format!("flush: {e}")))?;

    let raw = match read_capped(&mut stream) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::TimedOut || e.kind() == std::io::ErrorKind::WouldBlock => {
            return Err(ModelError::Timeout)
        }
        Err(e) => return Err(ModelError::Connect(format!("read: {e}"))),
    };
    let text = String::from_utf8_lossy(&raw).to_string();
    let (head_part, body_part) = split_head_body(&text)
        .ok_or_else(|| ModelError::BadResponse("response had no header/body separator".to_string()))?;

    let status_line = head_part.lines().next().unwrap_or("");
    let status = status_line
        .split(' ')
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok())
        .ok_or_else(|| ModelError::BadResponse(format!("unparseable status line `{status_line}`")))?;
    if status >= 400 {
        let excerpt: String = body_part.chars().take(240).collect();
        return Err(ModelError::Http(status, excerpt));
    }

    let value: Value = serde_json::from_str(&body_part)
        .map_err(|e| ModelError::BadResponse(format!("body is not JSON: {e}")))?;
    parse_chat_value(&value)
}

fn read_capped(stream: &mut TcpStream) -> std::io::Result<Vec<u8>> {
    let mut buf = Vec::with_capacity(8 * 1024);
    let mut chunk = [0u8; 8192];
    loop {
        let n = stream.read(&mut chunk)?;
        if n == 0 {
            return Ok(buf);
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.len() > MAX_RESPONSE_BYTES {
            return Ok(buf); // parse will fail as BadResponse; we never buffer unbounded
        }
    }
}

fn split_head_body(text: &str) -> Option<(String, String)> {
    let pos = text.find("\r\n\r\n")?;
    Some((text[..pos].to_string(), text[pos + 4..].to_string()))
}

/// Extract `choices[0].message` (content and/or tool_calls) plus usage.
fn parse_chat_value(value: &Value) -> Result<ChatReply, ModelError> {
    let choices = value
        .get("choices")
        .and_then(Value::as_array)
        .ok_or_else(|| ModelError::BadResponse("response has no `choices` array".to_string()))?;
    let first = choices
        .first()
        .ok_or_else(|| ModelError::BadResponse("`choices` array is empty".to_string()))?;
    let message = first
        .get("message")
        .and_then(Value::as_object)
        .ok_or_else(|| ModelError::BadResponse("choice has no `message` object".to_string()))?;

    let content = message
        .get("content")
        .and_then(Value::as_str)
        .map(|s| s.to_string())
        .filter(|s| !s.trim().is_empty());

    let mut tool_calls = Vec::new();
    if let Some(calls) = message.get("tool_calls").and_then(Value::as_array) {
        for call in calls {
            let id = call.get("id").and_then(Value::as_str).unwrap_or("").to_string();
            let function = call.get("function").unwrap_or(call);
            let name = function.get("name").and_then(Value::as_str).unwrap_or("").to_string();
            if name.is_empty() {
                return Err(ModelError::BadResponse("tool_call has no function name".to_string()));
            }
            let arguments = match function.get("arguments") {
                Some(Value::String(s)) => s.clone(),
                Some(v @ Value::Object(_)) => v.to_string(),
                Some(Value::Null) | None => "{}".to_string(),
                Some(other) => {
                    return Err(ModelError::BadResponse(format!(
                        "tool_call arguments are neither a string nor an object: {}",
                        other
                    )))
                }
            };
            tool_calls.push(ToolCall { id, name, arguments });
        }
    }

    let usage = value
        .get("usage")
        .and_then(Value::as_object)
        .map(|map| {
            map.iter()
                .filter_map(|(k, v)| v.as_i64().map(|n| (k.clone(), n)))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    Ok(ChatReply {
        content,
        tool_calls,
        finish_reason: first.get("finish_reason").and_then(Value::as_str).map(str::to_string),
        model: value.get("model").and_then(Value::as_str).map(str::to_string),
        usage,
    })
}

/// The tool definition shape the OpenAI-compatible API expects.
pub fn tool_definition(name: &str, description: &str, parameters: Value) -> Value {
    json!({"type": "function", "function": {"name": name, "description": description, "parameters": parameters}})
}


// ---------------------------------------------------------------------
// CV3: streaming chat. The same POST, but the caller asks the brain
// for `stream: true` and complete sentences are handed to
// `on_sentence` *while the answer is still arriving* — that callback
// is what lets agentd journal answer chunks inside the run instead
// of after it.
//
// Two sentence sources, because this agent has two answer channels:
// - `Content`: prose content deltas (the answer when the model
//   replies in prose instead of calling tools);
// - `AnswerText`: the `answer` tool's text, decoded incrementally
//   from its streaming arguments by `AnswerTextExtractor` — the
//   channel the system prompt tells the model to finish through.
// Both feed the same sentence chunker discipline: a sentence is
// handed over only when its end is proven.
//
// Transport honesty, in the spirit of the rest of this client:
// - The response head is read line-by-line before anything is
//   trusted; a non-2xx is the same `Http` error as `chat`.
// - SSE bodies arrive `Transfer-Encoding: chunked` from the real
//   servers (LiteLLM/uvicorn, llama.cpp), so a small chunk decoder
//   sits under the line reader; without it a chunk boundary landing
//   mid-line would corrupt the JSON.
// - A stream that ends without `[DONE]` and without a
//   `finish_reason` is a `BadResponse`, never a truncated answer:
//   a half answer must not be spoken as a whole one.
// - A brain that ignores `stream` and answers with a plain JSON body
//   is not an error: the reply parses exactly as `chat` would parse
//   it, `streamed` comes back false, and the caller journals the
//   answer as buffered instead of streamed.

use std::io::{BufRead, BufReader};

use crate::stream::{AnswerTextExtractor, SentenceChunker};

/// Where a streamed sentence came from (see the section comment).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SentenceSource {
    Content,
    AnswerText,
}

/// One streamed call's result: the assembled reply, whether the brain
/// actually streamed (SSE), and the sentences that only became
/// complete at end-of-stream, per source.
pub struct StreamedReply {
    pub reply: ChatReply,
    pub streamed: bool,
    pub content_tail: Vec<String>,
    pub answer_tail: Vec<String>,
}

struct ToolCallAcc {
    id: String,
    name: String,
    arguments: String,
}

/// Accumulates the SSE deltas of one streamed reply. Kept separate
/// from the socket code so the delta grammar is unit-testable.
struct StreamAcc {
    content: String,
    tool_calls: Vec<ToolCallAcc>,
    finish_reason: Option<String>,
    model: Option<String>,
    usage: Vec<(String, i64)>,
    content_chunker: SentenceChunker,
    answer_chunker: SentenceChunker,
    answer_extractor: Option<AnswerTextExtractor>,
}

fn parse_usage(value: &Value) -> Vec<(String, i64)> {
    value
        .get("usage")
        .and_then(Value::as_object)
        .map(|map| {
            map.iter()
                .filter_map(|(k, v)| v.as_i64().map(|n| (k.clone(), n)))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

impl StreamAcc {
    fn new() -> StreamAcc {
        StreamAcc {
            content: String::new(),
            tool_calls: Vec::new(),
            finish_reason: None,
            model: None,
            usage: Vec::new(),
            content_chunker: SentenceChunker::new(),
            answer_chunker: SentenceChunker::new(),
            answer_extractor: None,
        }
    }

    /// Apply one SSE data object. Complete sentences are passed to
    /// `on_sentence` in arrival order, tagged with their source.
    fn apply(
        &mut self,
        value: &Value,
        on_sentence: &mut dyn FnMut(SentenceSource, &str),
    ) {
        if let Some(m) = value.get("model").and_then(Value::as_str) {
            self.model = Some(m.to_string());
        }
        let usage = parse_usage(value);
        if !usage.is_empty() {
            self.usage = usage;
        }
        let Some(choices) = value.get("choices").and_then(Value::as_array) else {
            return; // e.g. a usage-only final object
        };
        let Some(first) = choices.first() else {
            return; // a final object with an empty choices array
        };
        if let Some(fr) = first.get("finish_reason").and_then(Value::as_str) {
            self.finish_reason = Some(fr.to_string());
        }
        let delta = first.get("delta").or_else(|| first.get("message"));
        let Some(delta) = delta else { return };
        if let Some(content) = delta.get("content").and_then(Value::as_str) {
            if !content.is_empty() {
                self.content.push_str(content);
                for sentence in self.content_chunker.push(content) {
                    on_sentence(SentenceSource::Content, &sentence);
                }
            }
        }
        if let Some(calls) = delta.get("tool_calls").and_then(Value::as_array) {
            for (pos, call) in calls.iter().enumerate() {
                let idx = call.get("index").and_then(Value::as_u64).unwrap_or(pos as u64) as usize;
                while self.tool_calls.len() <= idx {
                    self.tool_calls.push(ToolCallAcc {
                        id: String::new(),
                        name: String::new(),
                        arguments: String::new(),
                    });
                }
                let function = call.get("function").unwrap_or(call);
                let name_frag = function.get("name").and_then(Value::as_str).unwrap_or("");
                let args_frag = function.get("arguments").and_then(Value::as_str).unwrap_or("");
                let is_answer;
                {
                    let acc = &mut self.tool_calls[idx];
                    if let Some(id) = call.get("id").and_then(Value::as_str) {
                        if !id.is_empty() {
                            acc.id = id.to_string();
                        }
                    }
                    // Servers send the tool name whole, in the first
                    // delta that carries it; the first non-empty
                    // fragment is the name.
                    if acc.name.is_empty() && !name_frag.is_empty() {
                        acc.name = name_frag.to_string();
                    }
                    if !args_frag.is_empty() {
                        acc.arguments.push_str(args_frag);
                    }
                    is_answer = acc.name == "answer";
                }
                if is_answer && !args_frag.is_empty() {
                    if self.answer_extractor.is_none() {
                        self.answer_extractor = Some(AnswerTextExtractor::new());
                    }
                    let decoded = self
                        .answer_extractor
                        .as_mut()
                        .expect("extractor just created")
                        .feed(args_frag);
                    if !decoded.is_empty() {
                        for sentence in self.answer_chunker.push(&decoded) {
                            on_sentence(SentenceSource::AnswerText, &sentence);
                        }
                    }
                }
            }
        }
    }

    /// End of stream: assemble the reply and the sentences that only
    /// completed now, per source.
    fn finish(mut self) -> Result<(ChatReply, Vec<String>, Vec<String>), ModelError> {
        let mut content_tail = Vec::new();
        if let Some(last) = self.content_chunker.finish() {
            content_tail.push(last);
        }
        let mut answer_tail = Vec::new();
        if let Some(last) = self.answer_chunker.finish() {
            answer_tail.push(last);
        }
        let mut tool_calls = Vec::new();
        for acc in self.tool_calls {
            if acc.name.is_empty() && acc.arguments.is_empty() {
                continue; // a placeholder slot some servers open and never fill
            }
            if acc.name.is_empty() {
                return Err(ModelError::BadResponse("tool_call has no function name".to_string()));
            }
            tool_calls.push(ToolCall { id: acc.id, name: acc.name, arguments: acc.arguments });
        }
        let content = if self.content.trim().is_empty() {
            None
        } else {
            Some(self.content)
        };
        Ok((
            ChatReply {
                content,
                tool_calls,
                finish_reason: self.finish_reason,
                model: self.model,
                usage: self.usage,
            },
            content_tail,
            answer_tail,
        ))
    }
}

/// A `Transfer-Encoding: chunked` decoder over a buffered reader.
/// Only the framing is implemented: hex size lines (extensions
/// ignored), exact data, trailing CRLF, and the trailer section
/// after the zero chunk, which is skimmed to its blank line.
struct ChunkedReader<R: BufRead> {
    inner: R,
    cur: Vec<u8>,
    pos: usize,
    done: bool,
    in_trailers: bool,
}

impl<R: BufRead> ChunkedReader<R> {
    fn new(inner: R) -> ChunkedReader<R> {
        ChunkedReader { inner, cur: Vec::new(), pos: 0, done: false, in_trailers: false }
    }

    /// Load the next chunk's bytes into `cur` (or reach end-of-body).
    fn load(&mut self) -> std::io::Result<()> {
        self.cur.clear();
        self.pos = 0;
        if self.done {
            return Ok(());
        }
        loop {
            if self.in_trailers {
                let mut line = String::new();
                let n = self.inner.read_line(&mut line)?;
                if n == 0 || line.trim().is_empty() {
                    self.done = true;
                }
                return Ok(());
            }
            let mut line = String::new();
            let n = self.inner.read_line(&mut line)?;
            if n == 0 {
                self.done = true;
                return Ok(());
            }
            let size_str = line.trim().split(';').next().unwrap_or("");
            let size = usize::from_str_radix(size_str, 16).map_err(|_| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("bad chunk size line {line:?}"),
                )
            })?;
            if size == 0 {
                self.in_trailers = true;
                continue;
            }
            self.cur = vec![0u8; size];
            self.inner.read_exact(&mut self.cur)?;
            let mut crlf = [0u8; 2];
            self.inner.read_exact(&mut crlf)?;
            return Ok(());
        }
    }
}

impl<R: BufRead> Read for ChunkedReader<R> {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        if self.pos >= self.cur.len() {
            self.load()?;
            if self.cur.is_empty() {
                return Ok(0);
            }
        }
        let n = std::cmp::min(out.len(), self.cur.len() - self.pos);
        out[..n].copy_from_slice(&self.cur[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

impl<R: BufRead> BufRead for ChunkedReader<R> {
    fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
        if self.pos >= self.cur.len() {
            self.load()?;
        }
        Ok(&self.cur[self.pos..])
    }

    fn consume(&mut self, amt: usize) {
        self.pos = std::cmp::min(self.cur.len(), self.pos + amt);
    }
}

fn is_timeout(e: &std::io::Error) -> bool {
    matches!(e.kind(), std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock)
}

/// Read SSE data lines until `[DONE]`, EOF, or an error. Returns
/// whether `[DONE]` was seen; the caller decides what an unmarked
/// end means (with a `finish_reason` it is fine — some servers just
/// close — without one the answer is cut and must fail).
fn read_sse<R: BufRead>(
    reader: &mut R,
    acc: &mut StreamAcc,
    on_sentence: &mut dyn FnMut(SentenceSource, &str),
) -> Result<bool, ModelError> {
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => return Ok(false),
            Ok(_) => {}
            Err(e) if is_timeout(&e) => return Err(ModelError::Timeout),
            Err(e) => return Err(ModelError::Connect(format!("read: {e}"))),
        }
        let Some(data) = line.trim_end().strip_prefix("data:") else {
            continue; // event:/id:/comment lines carry nothing we need
        };
        let data = data.trim();
        if data == "[DONE]" {
            return Ok(true);
        }
        if data.is_empty() {
            continue;
        }
        let value: Value = serde_json::from_str(data)
            .map_err(|e| ModelError::BadResponse(format!("stream chunk is not JSON: {e}")))?;
        acc.apply(&value, on_sentence);
    }
}

/// Read a whole (non-streaming) body, capped like `chat`.
fn read_body<R: Read>(reader: &mut R) -> Result<String, ModelError> {
    let mut buf = Vec::with_capacity(8 * 1024);
    let mut chunk = [0u8; 8192];
    loop {
        match reader.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if buf.len() > MAX_RESPONSE_BYTES {
                    break; // the JSON parse will fail as BadResponse, as in `chat`
                }
            }
            Err(e) if is_timeout(&e) => return Err(ModelError::Timeout),
            Err(e) => return Err(ModelError::Connect(format!("read: {e}"))),
        }
    }
    Ok(String::from_utf8_lossy(&buf).to_string())
}

/// POST one chat request whose payload already carries
/// `stream: true`, and stream the reply: complete sentences go to
/// `on_sentence` as they land; the assembled reply returns at the
/// end. See the section comment above for the failure contract.
pub fn chat_stream(
    url: &str,
    bearer: Option<&str>,
    payload: &Value,
    timeout: Duration,
    on_sentence: &mut dyn FnMut(SentenceSource, &str),
) -> Result<StreamedReply, ModelError> {
    let (host, port, path) = split_url(url)?;
    let addr = (host.as_str(), port)
        .to_socket_addrs()
        .map_err(|e| ModelError::Connect(format!("resolve {host}:{port}: {e}")))?
        .next()
        .ok_or_else(|| ModelError::Connect(format!("{host}:{port} resolved to nothing")))?;
    let addr: SocketAddr = addr;

    let body = payload.to_string();
    let mut stream = TcpStream::connect_timeout(&addr, timeout)
        .map_err(|e| ModelError::Connect(format!("{addr}: {e}")))?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|e| ModelError::Connect(format!("read timeout: {e}")))?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(|e| ModelError::Connect(format!("write timeout: {e}")))?;

    let auth = match bearer {
        Some(k) => format!("Authorization: Bearer {k}\r\n"),
        None => String::new(),
    };
    let head = format!(
        "POST {path} HTTP/1.1\r\nHost: {addr}\r\nUser-Agent: agentd/0.1\r\n\
         Content-Type: application/json\r\n{auth}Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).map_err(|e| ModelError::Connect(format!("write: {e}")))?;
    stream.write_all(body.as_bytes()).map_err(|e| ModelError::Connect(format!("write body: {e}")))?;
    stream.flush().map_err(|e| ModelError::Connect(format!("flush: {e}")))?;

    let mut reader = BufReader::new(stream);
    // The head is line-shaped and never chunked; read it line by line.
    let mut status: Option<u16> = None;
    let mut content_type = String::new();
    let mut chunked = false;
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => {
                return Err(ModelError::BadResponse(
                    "response ended before its head completed".to_string(),
                ))
            }
            Ok(_) => {}
            Err(e) if is_timeout(&e) => return Err(ModelError::Timeout),
            Err(e) => return Err(ModelError::Connect(format!("read head: {e}"))),
        }
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            break;
        }
        if status.is_none() {
            status = trimmed
                .split(' ')
                .nth(1)
                .and_then(|s| s.parse::<u16>().ok());
            if status.is_none() {
                return Err(ModelError::BadResponse(format!("unparseable status line `{trimmed}`")));
            }
            continue;
        }
        if let Some((name, value)) = trimmed.split_once(':') {
            let value = value.trim();
            match name.trim().to_ascii_lowercase().as_str() {
                "content-type" => content_type = value.to_string(),
                "transfer-encoding" => {
                    if value.to_ascii_lowercase().contains("chunked") {
                        chunked = true;
                    }
                }
                _ => {}
            }
        }
    }
    let status = status.unwrap_or(0);

    if status >= 400 {
        let body_text = if chunked {
            let mut cr = ChunkedReader::new(reader);
            read_body(&mut cr)?
        } else {
            read_body(&mut reader)?
        };
        let excerpt: String = body_text.chars().take(240).collect();
        return Err(ModelError::Http(status, excerpt));
    }

    let mut acc = StreamAcc::new();
    if content_type.to_ascii_lowercase().contains("text/event-stream") {
        let saw_done = if chunked {
            let mut cr = BufReader::new(ChunkedReader::new(reader));
            read_sse(&mut cr, &mut acc, on_sentence)?
        } else {
            read_sse(&mut reader, &mut acc, on_sentence)?
        };
        if !saw_done && acc.finish_reason.is_none() {
            return Err(ModelError::BadResponse(
                "stream ended before completion (no [DONE], no finish_reason)".to_string(),
            ));
        }
        let (reply, content_tail, answer_tail) = acc.finish()?;
        Ok(StreamedReply { reply, streamed: true, content_tail, answer_tail })
    } else {
        // The brain answered whole despite `stream: true`. Parse it
        // exactly as `chat` would; nothing was streamed, and the
        // caller journals the answer as buffered, not chunked.
        let body_text = if chunked {
            let mut cr = ChunkedReader::new(reader);
            read_body(&mut cr)?
        } else {
            read_body(&mut reader)?
        };
        let value: Value = serde_json::from_str(&body_text)
            .map_err(|e| ModelError::BadResponse(format!("body is not JSON: {e}")))?;
        let reply = parse_chat_value(&value)?;
        Ok(StreamedReply { reply, streamed: false, content_tail: Vec::new(), answer_tail: Vec::new() })
    }
}

#[cfg(test)]
mod stream_tests {
    use super::*;

    #[test]
    fn content_deltas_accumulate_and_sentences_fire_in_order() {
        let mut acc = StreamAcc::new();
        let mut heard: Vec<(SentenceSource, String)> = Vec::new();
        let mut sink = |src: SentenceSource, s: &str| heard.push((src, s.to_string()));
        acc.apply(
            &json!({"model": "m", "choices": [{"index": 0, "delta": {"content": "Open maps and give me directions to San "}, "finish_reason": null}]}),
            &mut sink,
        );
        acc.apply(
            &json!({"choices": [{"index": 0, "delta": {"content": "Francisco. It takes 2.5 hours. Drive safe"}, "finish_reason": null}]}),
            &mut sink,
        );
        acc.apply(
            &json!({"choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}], "usage": {"prompt_tokens": 9, "completion_tokens": 20, "total_tokens": 29}}),
            &mut sink,
        );
        assert_eq!(
            heard,
            vec![
                (SentenceSource::Content, "Open maps and give me directions to San Francisco.".to_string()),
                (SentenceSource::Content, "It takes 2.5 hours.".to_string()),
            ]
        );
        let (reply, content_tail, answer_tail) = acc.finish().unwrap();
        assert_eq!(reply.content.as_deref(), Some("Open maps and give me directions to San Francisco. It takes 2.5 hours. Drive safe"));
        assert_eq!(reply.finish_reason.as_deref(), Some("stop"));
        assert_eq!(content_tail, vec!["Drive safe".to_string()]);
        assert!(answer_tail.is_empty());
        assert!(reply.usage.iter().any(|(k, v)| k == "total_tokens" && *v == 29));
    }

    #[test]
    fn answer_tool_text_streams_out_of_its_arguments() {
        let mut acc = StreamAcc::new();
        let mut heard: Vec<(SentenceSource, String)> = Vec::new();
        let mut sink = |src: SentenceSource, s: &str| heard.push((src, s.to_string()));
        acc.apply(
            &json!({"choices": [{"index": 0, "delta": {"tool_calls": [{"index": 0, "id": "call-1", "type": "function", "function": {"name": "answer", "arguments": "{\"text\": \"Maps is open. The drive"}}]}, "finish_reason": null}]}),
            &mut sink,
        );
        acc.apply(
            &json!({"choices": [{"index": 0, "delta": {"tool_calls": [{"index": 0, "function": {"arguments": " takes about an hour.\"}"}}]}, "finish_reason": null}]}),
            &mut sink,
        );
        acc.apply(
            &json!({"choices": [{"index": 0, "delta": {}, "finish_reason": "tool_calls"}]}),
            &mut sink,
        );
        assert_eq!(heard, vec![(SentenceSource::AnswerText, "Maps is open.".to_string())]);
        let (reply, _, answer_tail) = acc.finish().unwrap();
        assert_eq!(reply.tool_calls.len(), 1);
        assert_eq!(reply.tool_calls[0].name, "answer");
        assert_eq!(reply.tool_calls[0].arguments, "{\"text\": \"Maps is open. The drive takes about an hour.\"}");
        assert_eq!(answer_tail, vec!["The drive takes about an hour.".to_string()]);
    }

    #[test]
    fn other_tools_arguments_do_not_leak_into_speech() {
        let mut acc = StreamAcc::new();
        let mut heard: Vec<(SentenceSource, String)> = Vec::new();
        let mut sink = |src: SentenceSource, s: &str| heard.push((src, s.to_string()));
        acc.apply(
            &json!({"choices": [{"index": 0, "delta": {"tool_calls": [{"index": 0, "id": "c1", "type": "function", "function": {"name": "app_launch", "arguments": "{\"app\": \"maps. Now\"}"}}]}, "finish_reason": null}]}),
            &mut sink,
        );
        assert!(heard.is_empty(), "a non-answer tool's arguments are never sentences: {heard:?}");
    }

    #[test]
    fn usage_only_and_empty_choices_objects_are_tolerated() {
        let mut acc = StreamAcc::new();
        let mut sink = |_: SentenceSource, _: &str| {};
        acc.apply(&json!({"choices": [], "usage": {"total_tokens": 3}}), &mut sink);
        acc.apply(&json!({"id": "chatcmpl-x"}), &mut sink);
        let (reply, _, _) = acc.finish().unwrap();
        assert!(reply.usage.iter().any(|(k, v)| k == "total_tokens" && *v == 3));
    }
}
