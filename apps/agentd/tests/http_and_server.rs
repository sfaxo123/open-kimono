//! The HTTP surface, tested over a real socket: routes, the SSE contract
//! (Lane 2's L2-D4), reconnect backfill, and the ways a client can get a 4xx.

mod common;

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use agentd::agent::Agent;
use agentd::config::{self, Config};
use agentd::journal::Journal;
use agentd::phone::ProbeRoot;
use agentd::server::Server;
use common::{chat_body, spawn, text_message, Step};

/// Grab a free loopback port, then hand it to the server.
fn free_port() -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").expect("probe bind");
    let port = l.local_addr().unwrap().port();
    drop(l);
    port
}

struct TestServer {
    port: u16,
    journal: Journal,
}

/// Start agentd's HTTP server against a scripted fake brain.
fn start(gateway_steps: Vec<Step>) -> (TestServer, common::FakeBrain) {
    let brain = spawn("gateway", gateway_steps);
    let local = free_port();
    let port = free_port();
    let thread: String = format!("{:?}", std::thread::current().id())
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    let key_path = std::env::temp_dir().join(format!("agentd-http-key-{}-{thread}", std::process::id()));
    std::fs::write(&key_path, "test-key-not-a-secret\n").unwrap();
    let text = format!(
        "listen = \"127.0.0.1:{port}\"\n\
         gateway_url = \"http://127.0.0.1:{}/v1/chat/completions\"\n\
         gateway_model = \"auto\"\n\
         local_url = \"http://127.0.0.1:{local}/v1/chat/completions\"\n\
         local_model = \"gemma3-1b\"\n\
         key_file = \"{}\"\n\
         model_timeout_sec = 5\n",
        brain.port,
        key_path.display()
    );
    let cfg: Config = config::load(&text).expect("config");
    let journal = Journal::new(256);
    let agent = Arc::new(Agent::with_root(cfg.clone(), journal.clone(), ProbeRoot(std::env::temp_dir())));
    let server = Server { config: cfg, agent, journal: journal.clone() };
    thread::spawn(move || {
        let _ = server.serve();
    });
    wait_for_port(port);
    (TestServer { port, journal }, brain)
}

fn wait_for_port(port: u16) {
    for _ in 0..100 {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return;
        }
        thread::sleep(Duration::from_millis(20));
    }
    panic!("server never came up on port {port}");
}

fn send(port: u16, request: &str) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    stream.write_all(request.as_bytes()).expect("write");
    stream.flush().unwrap();
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if buf.len() > 200_000 {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    String::from_utf8_lossy(&buf).to_string()
}

fn status_of(response: &str) -> u16 {
    response.split_whitespace().nth(1).and_then(|s| s.parse().ok()).unwrap_or(0)
}

fn body_of(response: &str) -> String {
    response.split_once("\r\n\r\n").map(|(_, b)| b.to_string()).unwrap_or_default()
}

/// POST with a correctly computed Content-Length.
fn post(port: u16, body: &str) -> String {
    send(port, &format!(
        "POST /request HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    ))
}

#[test]
fn post_request_runs_the_loop_and_returns_the_answer() {
    let (server, brain) = start(vec![Step::Reply(chat_body(text_message("Four two."), "auto"))]);
    let response = post(server.port, "{\"text\": \"say 42\"}");
    assert_eq!(status_of(&response), 200, "got: {response}");
    let body: serde_json::Value = serde_json::from_str(body_of(&response).trim()).expect("json body");
    assert_eq!(body["answer"], "Four two.");
    assert!(body["run_id"].as_str().unwrap().starts_with("run-"));
    assert_eq!(brain.hits(), 1);
    assert!(server.journal.len() >= 4, "the run must have left journal records");
}

#[test]
fn a_body_without_text_is_refused_before_any_model_call() {
    let (server, brain) = start(vec![Step::Reply(chat_body(text_message("nope"), "auto"))]);
    for (body, expected) in [
        ("", "empty body"),
        ("not json", "not JSON"),
        ("{\"prompt\": \"hi\"}", "string field `text`"),
        ("{\"text\": \"   \"}", "must not be empty"),
    ] {
        let response = post(server.port, body);
        assert_eq!(status_of(&response), 400, "body {body:?} should be refused: {response}");
        assert!(body_of(&response).contains(expected), "body {body:?} should mention {expected}: {}", body_of(&response));
    }
    assert_eq!(brain.hits(), 0, "a refused request must never reach the model");
}

#[test]
fn health_reports_the_journal_and_the_brain_chain() {
    let (server, _) = start(vec![Step::HangUp]);
    let response = send(server.port, "GET /health HTTP/1.1\r\nHost: x\r\n\r\n");
    assert_eq!(status_of(&response), 200);
    let body: serde_json::Value = serde_json::from_str(body_of(&response).trim()).expect("json");
    assert_eq!(body["ok"], true);
    assert_eq!(body["brains"][0], "homelab-gateway");
    assert_eq!(body["brains"][1], "on-device-llama");
}

#[test]
fn power_answers_read_only_over_http_for_the_kimono_panel() {
    let (server, _) = start(vec![Step::HangUp]);
    let response = send(server.port, "GET /power HTTP/1.1\r\nHost: x\r\n\r\n");
    assert_eq!(status_of(&response), 200);
    let body: serde_json::Value = serde_json::from_str(body_of(&response).trim()).expect("json");
    assert_eq!(body["read_only"], true, "the panel must be told this is read-only: {body}");
    let names: Vec<&str> = body["components"].as_array().expect("components").iter().map(|c| c["component"].as_str().unwrap()).collect();
    assert_eq!(names, ["screen", "bluetooth", "wifi", "local_model", "services"], "the tracked set is fixed: {names:?}");
    for component in body["components"].as_array().unwrap() {
        // Every component states how it was observed, so the UI can say so too.
        assert!(component["source"].as_str().unwrap().len() > 3, "no source: {component}");
        assert!(["on", "off", "unknown"].contains(&component["state"].as_str().unwrap()), "bad state: {component}");
    }
    assert!(body["idle_timeout_seconds"].as_u64().unwrap() > 0);
}

#[test]
fn unknown_route_and_unsupported_method_are_named_not_hidden() {
    let (server, _) = start(vec![Step::HangUp]);
    let missing = send(server.port, "GET /nope HTTP/1.1\r\nHost: x\r\n\r\n");
    assert_eq!(status_of(&missing), 404);
    assert!(body_of(&missing).contains("no route for GET /nope"));

    let put = send(server.port, "PUT /request HTTP/1.1\r\nHost: x\r\nContent-Length: 0\r\n\r\n");
    assert_eq!(status_of(&put), 405, "PUT must be refused: {put}");
    assert!(body_of(&put).contains("method not supported: PUT"), "got: {}", body_of(&put));
}

#[test]
fn a_malformed_request_line_gets_a_400_that_says_why() {
    let (server, _) = start(vec![Step::HangUp]);
    let response = send(server.port, "GARBAGE\r\n\r\n");
    assert_eq!(status_of(&response), 400);
    assert!(body_of(&response).contains("malformed request line"), "got: {}", body_of(&response));

    // A well-formed line with an unsupported version is refused too, by name.
    let old = send(server.port, "GET /health HTTP/9.9\r\nHost: x\r\n\r\n");
    assert_eq!(status_of(&old), 400);
    assert!(body_of(&old).contains("unsupported version `HTTP/9.9`"), "got: {}", body_of(&old));
}

#[test]
fn an_oversized_body_is_refused_with_413() {
    let (server, _) = start(vec![Step::HangUp]);
    let huge = "x".repeat(70 * 1024);
    let response = send(
        server.port,
        &format!("POST /request HTTP/1.1\r\nHost: x\r\nContent-Length: {}\r\n\r\n{huge}", huge.len()),
    );
    assert_eq!(status_of(&response), 413, "got status {}", status_of(&response));
    assert!(body_of(&response).contains("exceeds the 65536 byte limit"), "got: {}", body_of(&response));
}

#[test]
fn sse_events_stream_one_journal_record_per_frame_with_ids() {
    let (server, _brain) = start(vec![
        Step::Reply(chat_body(text_message("Streaming works."), "auto")),
    ]);

    let mut stream = TcpStream::connect(("127.0.0.1", server.port)).expect("connect");
    stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    stream.write_all(b"GET /events HTTP/1.1\r\nHost: x\r\nAccept: text/event-stream\r\n\r\n").unwrap();
    stream.flush().unwrap();

    // Read the SSE headers.
    let mut buf = Vec::new();
    while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
        let mut byte = [0u8; 1];
        if stream.read(&mut byte).unwrap_or(0) == 0 {
            panic!("SSE stream closed before headers");
        }
        buf.push(byte[0]);
    }
    let head = String::from_utf8_lossy(&buf).to_string();
    assert!(head.contains("text/event-stream"), "got: {head}");
    assert!(head.contains("Transfer-Encoding: chunked"), "SSE must be chunked: {head}");

    // Trigger a request from another "client" while the SSE stream is open.
    let response = post(server.port, "{\"text\": \"stream please\"}");
    assert_eq!(status_of(&response), 200, "the request itself must succeed: {response}");

    // Read frames until we see the RUN_STARTED record.
    let mut frames = String::new();
    let mut seen_run_started = false;
    for _ in 0..200 {
        let mut chunk = [0u8; 4096];
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => frames.push_str(&String::from_utf8_lossy(&chunk[..n])),
            Err(_) => break,
        }
        if frames.contains("RUN_STARTED") {
            seen_run_started = true;
            break;
        }
    }
    assert!(seen_run_started, "no RUN_STARTED frame arrived; got: {frames}");

    // Each frame must carry an id and exactly one parseable journal record.
    let data_lines: Vec<&str> = frames.lines().filter(|l| l.starts_with("data: ")).collect();
    assert!(!data_lines.is_empty(), "no data: lines in {frames}");
    for line in &data_lines {
        let json = line["data: ".len()..].trim();
        let event = trace_events::from_json_line(json).unwrap_or_else(|e| panic!("SSE data is not a journal record: {e}\n{json}"));
        assert!(event.seq > 0 || event.seq == 0, "seq present");
    }
    let id_lines: Vec<&str> = frames.lines().filter(|l| l.starts_with("id: ")).collect();
    assert_eq!(id_lines.len(), data_lines.len(), "every data frame needs an id frame");
}

#[test]
fn reconnecting_with_last_event_id_gets_the_gap_back() {
    let (server, _) = start(vec![Step::Reply(chat_body(text_message("First run."), "auto"))]);
    let response = post(server.port, "{\"text\": \"hello\"}");
    assert_eq!(status_of(&response), 200, "got: {response}");
    let total = server.journal.len() as u64;
    assert!(total >= 4);

    // Reconnect claiming we already saw everything up to seq 1.
    let replay = send(server.port, &format!("GET /events HTTP/1.1\r\nHost: x\r\nLast-Event-ID: 1\r\n\r\n"));
    assert_eq!(status_of(&replay), 200);
    let data_lines: Vec<&str> = replay.lines().filter(|l| l.starts_with("data: ")).collect();
    // seq 0 is the one event the client claims to have already seen.
    assert!(data_lines.len() as u64 >= total - 2, "backfill must return the gap: {} lines for {total} events", data_lines.len());
    let mut max_seq = 0;
    for line in data_lines {
        let json = line["data: ".len()..].trim();
        let event = trace_events::from_json_line(json).expect("backfilled record parses");
        assert!(event.seq > 1, "Last-Event-ID must be honoured: got seq {}", event.seq);
        max_seq = max_seq.max(event.seq);
    }
    assert_eq!(max_seq, total - 1, "the backfill must reach the newest event");
}

/// Start the server with a `static_dir`: a fresh temp `kimono/` directory
/// holding exactly `files`, plus a `secret.txt` NEXT TO it that no request
/// may ever reach. Returns the server, the brain, and the temp root.
fn start_with_static(files: &[(&str, &str)]) -> (TestServer, common::FakeBrain, std::path::PathBuf) {
    let brain = spawn("gateway", vec![Step::HangUp]);
    let local = free_port();
    let port = free_port();
    let thread: String = format!("{:?}", std::thread::current().id())
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    let root = std::env::temp_dir().join(format!("agentd-static-{}-{thread}", std::process::id()));
    let dir = root.join("kimono");
    std::fs::create_dir_all(&dir).unwrap();
    for (name, contents) in files {
        std::fs::write(dir.join(name), contents).unwrap();
    }
    std::fs::write(root.join("secret.txt"), "TOP-SECRET-DO-NOT-SERVE").unwrap();
    let key_path = root.join("key");
    std::fs::write(&key_path, "test-key-not-a-secret\n").unwrap();
    let text = format!(
        "listen = \"127.0.0.1:{port}\"\n\
         gateway_url = \"http://127.0.0.1:{}/v1/chat/completions\"\n\
         gateway_model = \"auto\"\n\
         local_url = \"http://127.0.0.1:{local}/v1/chat/completions\"\n\
         local_model = \"gemma3-1b\"\n\
         key_file = \"{}\"\n\
         static_dir = \"{}\"\n\
         model_timeout_sec = 5\n",
        brain.port,
        key_path.display(),
        dir.display()
    );
    let cfg: Config = config::load(&text).expect("config");
    let journal = Journal::new(256);
    let agent = Arc::new(Agent::with_root(cfg.clone(), journal.clone(), ProbeRoot(std::env::temp_dir())));
    let server = Server { config: cfg, agent, journal: journal.clone() };
    thread::spawn(move || {
        let _ = server.serve();
    });
    wait_for_port(port);
    (TestServer { port, journal }, brain, root)
}

#[test]
fn static_assets_on_the_allowlist_are_served_with_content_types() {
    let files: &[(&str, &str)] = &[
        ("index.html", "<html>kimono</html>"),
        ("app.js", "console.log('app');"),
        ("render.mjs", "export const r = 1;"),
        ("dom.mjs", "export const d = 1;"),
        ("modules.mjs", "export const m = 1;"),
        ("style.css", "body { margin: 0; }"),
    ];
    let (server, _brain, _root) = start_with_static(files);
    for (path, ctype, body) in [
        ("/", "text/html", "<html>kimono</html>"),
        ("/index.html", "text/html", "<html>kimono</html>"),
        ("/app.js", "text/javascript", "console.log('app');"),
        ("/render.mjs", "text/javascript", "export const r = 1;"),
        ("/dom.mjs", "text/javascript", "export const d = 1;"),
        ("/modules.mjs", "text/javascript", "export const m = 1;"),
        ("/style.css", "text/css", "body { margin: 0; }"),
    ] {
        let response = send(server.port, &format!("GET {path} HTTP/1.1\r\nHost: x\r\n\r\n"));
        assert_eq!(status_of(&response), 200, "GET {path}: {response}");
        assert!(response.contains(&format!("Content-Type: {ctype}")), "GET {path} content type: {response}");
        assert_eq!(body_of(&response), body, "GET {path} body");
    }
}

#[test]
fn static_names_off_the_allowlist_404_even_when_the_file_exists() {
    // evil.js and secret.txt are IN the directory; the allowlist, not the
    // disk, decides what is public.
    let (server, _brain, _root) = start_with_static(&[
        ("index.html", "<html>kimono</html>"),
        ("evil.js", "alert('x')"),
        ("secret.txt", "in-dir secret"),
    ]);
    for path in ["/evil.js", "/secret.txt", "/favicon.ico", "/APP.JS", "/index.html.bak", "/modules.json"] {
        let response = send(server.port, &format!("GET {path} HTTP/1.1\r\nHost: x\r\n\r\n"));
        assert_eq!(status_of(&response), 404, "GET {path} must 404: {response}");
        assert!(!body_of(&response).contains("in-dir secret"), "GET {path} leaked a file: {response}");
    }
    // The allowlisted page still serves.
    let ok = send(server.port, "GET / HTTP/1.1\r\nHost: x\r\n\r\n");
    assert_eq!(status_of(&ok), 200);
}

#[test]
fn static_traversal_spellings_all_404() {
    let (server, _brain, _root) = start_with_static(&[("index.html", "<html>kimono</html>")]);
    // secret.txt sits NEXT TO the static dir (start_with_static wrote it);
    // no spelling of reaching it -- or anything else outside the list --
    // may work.
    for path in [
        "/../secret.txt",
        "/../../secret.txt",
        "/%2e%2e/secret.txt",
        "/%2E%2E%2Fsecret.txt",
        "/..%2fsecret.txt",
        "/....//secret.txt",
        "/..\\secret.txt",
        "/..%5csecret.txt",
        "//secret.txt",
        "/./../secret.txt",
        "/etc/passwd",
        "/../agentic/agentd-kimono.toml",
    ] {
        let response = send(server.port, &format!("GET {path} HTTP/1.1\r\nHost: x\r\n\r\n"));
        assert_eq!(status_of(&response), 404, "GET {path} must 404: {response}");
        assert!(!body_of(&response).contains("TOP-SECRET-DO-NOT-SERVE"), "GET {path} leaked the secret: {response}");
    }
}

#[test]
fn without_static_dir_the_fallback_text_still_answers() {
    let (server, _) = start(vec![Step::HangUp]);
    for path in ["/", "/index.html", "/app.js", "/modules.mjs"] {
        let response = send(server.port, &format!("GET {path} HTTP/1.1\r\nHost: x\r\n\r\n"));
        assert_eq!(status_of(&response), 200, "GET {path}: {response}");
        assert!(body_of(&response).contains("agentd is up"), "GET {path} fallback: {response}");
    }
}
