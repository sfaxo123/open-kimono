//! CV1 wire shape: a session turn's POST carries `session` and
//! `follow_up`; a sessionless turn's POST is the old wire exactly
//! ({"text": ...} and nothing else). The fake agentd here is a raw
//! TcpListener that captures the request body and answers with a
//! run id, because the thing under test is the bytes voiced sends.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use voiced::agentd::{post_request, post_request_session, Endpoint};
use voiced::capture::JournalOut;

#[derive(Clone)]
struct Sink(Arc<Mutex<Vec<u8>>>);

impl Write for Sink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn write_all(&mut self, buf: &[u8]) -> std::io::Result<()> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn journal() -> JournalOut {
    let sink = Sink(Arc::new(Mutex::new(Vec::new())));
    JournalOut::with_service(Box::new(sink), "voiced-test".to_string())
}

/// Accept exactly one request, capture its body, reply with a run id.
fn capture_one() -> (Endpoint, std::thread::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake agentd");
    let port = listener.local_addr().unwrap().port();
    let handle = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
        let mut raw = Vec::new();
        let mut buf = [0u8; 1024];
        let body_start = loop {
            let n = stream.read(&mut buf).expect("read");
            raw.extend_from_slice(&buf[..n]);
            if let Some(pos) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&raw[..pos]).to_string();
                let len: usize = head
                    .lines()
                    .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse().unwrap_or(0)))
                    .unwrap_or(0);
                if raw.len() - (pos + 4) >= len {
                    break pos + 4;
                }
            }
        };
        let body = String::from_utf8_lossy(&raw[body_start..]).to_string();
        let reply = "{\"run_id\":\"run-test-1\"}";
        let http = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", reply.len(), reply);
        stream.write_all(http.as_bytes()).expect("reply");
        body
    });
    (Endpoint { host: "127.0.0.1".to_string(), port }, handle)
}

#[test]
fn session_turn_posts_session_and_follow_up() {
    let (ep, handle) = capture_one();
    let mut j = journal();
    let run_id = post_request_session(&ep, "how long does it take?", Some("sess-1730000000-1"), true, Some(0.4), Duration::from_secs(5), &mut j).expect("post");
    assert_eq!(run_id, "run-test-1");
    let body: serde_json::Value = serde_json::from_str(&handle.join().expect("server thread")).expect("body is JSON");
    assert_eq!(body["text"].as_str(), Some("how long does it take?"));
    assert_eq!(body["session"].as_str(), Some("sess-1730000000-1"));
    assert_eq!(body["follow_up"], serde_json::Value::Bool(true));
    // CV2: the STT confidence rides the wire when the engine reported one.
    let conf = body["asr_confidence"].as_f64().expect("asr_confidence is a number");
    assert!((conf - 0.4).abs() < 1e-6, "asr_confidence rides the wire: {conf}");
}

#[test]
fn sessionless_turn_posts_the_old_wire_exactly() {
    let (ep, handle) = capture_one();
    let mut j = journal();
    let run_id = post_request(&ep, "what time is it", Duration::from_secs(5), &mut j).expect("post");
    assert_eq!(run_id, "run-test-1");
    let raw = handle.join().expect("server thread");
    let body: serde_json::Value = serde_json::from_str(&raw).expect("body is JSON");
    assert_eq!(body.as_object().unwrap().len(), 1, "sessionless body carries text only: {raw}");
    assert_eq!(body["text"].as_str(), Some("what time is it"));
}
