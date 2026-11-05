//! A scripted fake OpenAI-compatible brain for the host tests.
//!
//! It is a real TCP server on 127.0.0.1, because the thing under test is the
//! HTTP client and the fallback chain — a mock function would prove neither.
//! Each scripted step says what this brain does with the next request: answer,
//! stall (to force a timeout), or fail with a status.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

#[derive(Clone, Debug)]
pub enum Step {
    /// 200 with this JSON body.
    Reply(String),
    /// Accept, then stall for this long without answering (client must time out).
    Stall(Duration),
    /// Answer with this HTTP status and a short body.
    Status(u16, String),
    /// Close the connection without any response.
    HangUp,
}

pub struct FakeBrain {
    pub port: u16,
    /// Every request body the fake saw, in order — tests assert on these.
    pub seen: Arc<Mutex<Vec<String>>>,
    pub hits: Arc<AtomicUsize>,
}

impl FakeBrain {
    pub fn bodies(&self) -> Vec<String> {
        self.seen.lock().unwrap().clone()
    }

    pub fn hits(&self) -> usize {
        self.hits.load(Ordering::SeqCst)
    }
}

/// Start a fake brain on an ephemeral port. The thread lives for the whole
/// test process; each scripted step handles one connection, and once the
/// script is exhausted further requests are stalled (a brain that ran out of
/// answers is a brain that is not answering).
pub fn spawn(name: &str, script: Vec<Step>) -> FakeBrain {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake brain");
    let port = listener.local_addr().expect("local addr").port();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let hits = Arc::new(AtomicUsize::new(0));

    let seen_thread = Arc::clone(&seen);
    let hits_thread = Arc::clone(&hits);
    thread::spawn(move || {
        let mut steps = script.into_iter();
        for step in steps.by_ref() {
            let (mut stream, _) = match listener.accept() {
                Ok(s) => s,
                Err(_) => return,
            };
            hits_thread.fetch_add(1, Ordering::SeqCst);
            let body = read_request_body(&mut stream);
            seen_thread.lock().unwrap().push(body);
            match step {
                Step::Reply(json) => write_http(&mut stream, 200, "OK", "application/json", &json),
                Step::Stall(d) => {
                    thread::sleep(d);
                    drop(stream);
                }
                Step::Status(code, text) => write_http(&mut stream, code, "Error", "text/plain", &text),
                Step::HangUp => drop(stream),
            }
        }
        // Script exhausted: stall anything else so a second call times out.
        loop {
            let (stream, _) = match listener.accept() {
                Ok(s) => s,
                Err(_) => return,
            };
            hits_thread.fetch_add(1, Ordering::SeqCst);
            read_request_body(&mut { stream });
            thread::sleep(Duration::from_secs(30));
        }
    });

    let _ = name;
    FakeBrain { port, seen, hits }
}

fn read_request_body(stream: &mut TcpStream) -> String {
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    let head_end = loop {
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break pos;
        }
        if stream.read(&mut byte).unwrap_or(0) == 0 {
            return String::new();
        }
        buf.extend_from_slice(&byte);
        if buf.len() > 200_000 {
            return String::new();
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let length = head
        .lines()
        .find(|l| l.to_ascii_lowercase().starts_with("content-length:"))
        .and_then(|l| l.split_once(':').map(|(_, v)| v.trim().parse::<usize>().unwrap_or(0)))
        .unwrap_or(0);
    let mut body = buf[head_end + 4..].to_vec();
    let mut scratch = [0u8; 4096];
    while body.len() < length {
        match stream.read(&mut scratch) {
            Ok(0) | Err(_) => break,
            Ok(n) => body.extend_from_slice(&scratch[..n]),
        }
    }
    String::from_utf8_lossy(&body).to_string()
}

fn write_http(stream: &mut TcpStream, status: u16, reason: &str, content_type: &str, body: &str) {
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body.as_bytes());
    let _ = stream.flush();
}

/// The chat-completions envelope around one assistant message.
pub fn chat_body(message: serde_json::Value, model: &str) -> String {
    serde_json::json!({
        "id": "chatcmpl-fake",
        "object": "chat.completion",
        "model": model,
        "choices": [{"index": 0, "message": message, "finish_reason": "stop"}],
        "usage": {"prompt_tokens": 40, "completion_tokens": 12, "total_tokens": 52}
    })
    .to_string()
}

/// An assistant message that asks for a tool call.
pub fn tool_call_message(call_id: &str, name: &str, arguments: &str) -> serde_json::Value {
    serde_json::json!({
        "role": "assistant",
        "content": null,
        "tool_calls": [{"id": call_id, "type": "function", "function": {"name": name, "arguments": arguments}}]
    })
}

/// An assistant message that just answers.
pub fn text_message(text: &str) -> serde_json::Value {
    serde_json::json!({"role": "assistant", "content": text})
}
