//! A deliberately small HTTP/1.1 implementation: enough to serve `POST
//! /request`, `GET /events` (SSE) and a static file, and enough to be tested
//! against malformed input. No dependency, no TLS — the phone talks to the
//! homelab over plain HTTP on Meshnet (see `docs/track-a/BUILD.md`).
//!
//! Hard limits, because this runs on a phone: bounded header block, bounded
//! body, and a request line that must be exactly `METHOD SP TARGET SP HTTP/1.x`.

use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::time::Duration;

pub const MAX_HEADER_BYTES: usize = 16 * 1024;
pub const MAX_BODY_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub query: Option<String>,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        let want = name.to_ascii_lowercase();
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(&want))
            .map(|(_, v)| v.as_str())
    }

    pub fn query_param(&self, key: &str) -> Option<&str> {
        let q = self.query.as_deref()?;
        q.split('&')
            .find_map(|pair| pair.split_once('='))
            .filter(|(k, _)| *k == key)
            .map(|(_, v)| v)
    }

    pub fn body_text(&self) -> String {
        String::from_utf8_lossy(&self.body).to_string()
    }
}

/// Why a request was refused. The reason string is what the client sees, so it
/// names the rule that was broken.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HttpError {
    /// Malformed request line / headers / body — respond 400.
    BadRequest(String),
    /// Header block or body over the limit — respond 413.
    TooLarge(String),
    /// Method we do not implement — respond 405.
    MethodNotSupported(String),
    /// Client stalled mid-request — respond 408 and hang up.
    Timeout,
}

impl HttpError {
    pub fn status(&self) -> u16 {
        match self {
            HttpError::BadRequest(_) => 400,
            HttpError::TooLarge(_) => 413,
            HttpError::MethodNotSupported(_) => 405,
            HttpError::Timeout => 408,
        }
    }

    /// What the client is told, naming the rule that was broken.
    pub fn reason_text(&self) -> String {
        match self {
            HttpError::BadRequest(m) => format!("bad request: {m}"),
            HttpError::TooLarge(m) => format!("payload too large: {m}"),
            HttpError::MethodNotSupported(m) => format!("method not supported: {m}"),
            HttpError::Timeout => "request timed out before it was complete".to_string(),
        }
    }
}

/// Read one request from the stream. `Ok(None)` means the peer closed it
/// cleanly between requests.
pub fn read_request(stream: &mut TcpStream) -> io::Result<Result<Request, HttpError>> {
    stream.set_read_timeout(Some(Duration::from_secs(15)))?;
    let mut buf = Vec::with_capacity(1024);
    let mut byte = [0u8; 1];
    let head_end = loop {
        if let Some(pos) = find_double_crlf(&buf) {
            break pos;
        }
        if buf.len() >= MAX_HEADER_BYTES {
            return Ok(Err(HttpError::TooLarge(format!(
                "header block exceeds {MAX_HEADER_BYTES} bytes"
            ))));
        }
        match stream.read(&mut byte) {
            Ok(0) => {
                return Ok(if buf.is_empty() {
                    Err(HttpError::BadRequest("connection closed before a request line".to_string()))
                } else {
                    Err(HttpError::BadRequest("connection closed mid request headers".to_string()))
                })
            }
            Ok(_) => buf.extend_from_slice(&byte),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::TimedOut => {
                return Ok(Err(HttpError::Timeout))
            }
            Err(e) => return Err(e),
        }
    };

    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let mut lines = head.split("\r\n");
    let request_line = lines.next().unwrap_or("");
    let mut parts = request_line.split(' ');
    let (Some(method), Some(target), Some(version)) = (parts.next(), parts.next(), parts.next()) else {
        return Ok(Err(HttpError::BadRequest(format!("malformed request line `{request_line}`"))));
    };
    if !version.eq_ignore_ascii_case("HTTP/1.1") && !version.eq_ignore_ascii_case("HTTP/1.0") {
        return Ok(Err(HttpError::BadRequest(format!("unsupported version `{version}`"))));
    }
    if !matches!(method, "GET" | "POST") {
        return Ok(Err(HttpError::MethodNotSupported(method.to_string())));
    }
    let (path, query) = match target.split_once('?') {
        Some((p, q)) => (p.to_string(), Some(q.to_string())),
        None => (target.to_string(), None),
    };

    let mut headers = Vec::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let Some((k, v)) = line.split_once(':') else {
            return Ok(Err(HttpError::BadRequest(format!("malformed header line `{line}`"))));
        };
        headers.push((k.trim().to_string(), v.trim().to_string()));
    }

    let length = match headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .map(|(_, v)| v.trim().parse::<usize>())
    {
        Some(Err(_)) => return Ok(Err(HttpError::BadRequest("content-length is not a number".to_string()))),
        Some(Ok(n)) => n,
        None => 0,
    };
    if length > MAX_BODY_BYTES {
        return Ok(Err(HttpError::TooLarge(format!(
            "body of {length} bytes exceeds the {MAX_BODY_BYTES} byte limit"
        ))));
    }

    // `head_end` points at the first byte of the CRLF CRLF that ends the
    // headers, so the body starts four bytes later.
    let mut body = buf[head_end + 4..].to_vec();
    let mut scratch = [0u8; 4096];
    while body.len() < length {
        let want = (length - body.len()).min(scratch.len());
        match stream.read(&mut scratch[..want]) {
            Ok(0) => return Ok(Err(HttpError::BadRequest("connection closed mid body".to_string()))),
            Ok(n) => body.extend_from_slice(&scratch[..n]),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::TimedOut => {
                return Ok(Err(HttpError::Timeout))
            }
            Err(e) => return Err(e),
        }
    }

    Ok(Ok(Request { method: method.to_string(), path, query, headers, body }))
}

fn find_double_crlf(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

/// Send a complete response with a body we already have.
pub fn write_response(
    stream: &mut TcpStream,
    status: u16,
    reason: &str,
    content_type: &str,
    body: &[u8],
) -> io::Result<()> {
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()
}

pub fn respond_text(stream: &mut TcpStream, status: u16, reason: &str, text: &str) -> io::Result<()> {
    write_response(stream, status, reason, "text/plain; charset=utf-8", text.as_bytes())
}

pub fn respond_json(stream: &mut TcpStream, status: u16, reason: &str, json: &str) -> io::Result<()> {
    write_response(stream, status, reason, "application/json", json.as_bytes())
}

/// Open a chunked SSE stream. The caller then writes events and finishes with
/// [`finish_chunked`].
pub fn start_sse(stream: &mut TcpStream) -> io::Result<()> {
    stream.set_nodelay(true)?;
    stream.write_all(
        b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-store\r\n\
          Connection: close\r\nTransfer-Encoding: chunked\r\n\r\n",
    )?;
    stream.flush()
}

/// One chunk: SSE frames are written exactly as the AG-UI/EventSource reader
/// expects them (`id:` then `data:` then a blank line).
pub fn write_chunk(stream: &mut TcpStream, payload: &str) -> io::Result<()> {
    stream.write_all(format!("{:x}\r\n", payload.len()).as_bytes())?;
    stream.write_all(payload.as_bytes())?;
    stream.write_all(b"\r\n")?;
    stream.flush()
}

pub fn finish_chunked(stream: &mut TcpStream) -> io::Result<()> {
    stream.write_all(b"0\r\n\r\n")?;
    stream.flush()
}
