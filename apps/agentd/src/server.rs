//! The HTTP surface of agentd: `POST /request`, `GET /events` (SSE),
//! `GET /health`, and an optional static directory for the kimono view.
//!
//! The SSE contract is Lane 2's (STATUS L2-D4): one chunk per journal record,
//! `id:` is the envelope `seq`, `data:` is exactly one trace-events JSONL line,
//! and a reconnecting client may send `Last-Event-ID` to get the gap back.

use std::io::Write;
use std::net::TcpListener;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use serde_json::{json, Value};

use crate::agent::Agent;
use crate::config::Config;
use crate::kimono::Recorder;
use crate::http::{self, Request};
use crate::journal::Journal;

/// How long an idle SSE stream waits before sending a keepalive comment. A
/// dead client is then detected by a write error instead of living forever.
const SSE_KEEPALIVE: Duration = Duration::from_secs(15);
/// How often the service clock is advanced. One second is finer than any
/// service the plan asks for and costs nothing on the phone.
const SERVICE_TICK: Duration = Duration::from_secs(1);

/// The fixed set of files served from `static_dir`: the kimono page and
/// exactly the assets it loads (apps/kimono-ui/web/, mirrored byte for
/// byte in the phone's ~/agentic/kimono/). The list, not the disk,
/// decides what is public: a file that exists in the directory but is
/// not listed here is still a 404.
const STATIC_FILES: &[(&str, &str)] = &[
    ("index.html", "text/html; charset=utf-8"),
    ("app.js", "text/javascript; charset=utf-8"),
    ("render.mjs", "text/javascript; charset=utf-8"),
    ("dom.mjs", "text/javascript; charset=utf-8"),
    ("modules.mjs", "text/javascript; charset=utf-8"),
    ("style.css", "text/css; charset=utf-8"),
];

/// Resolve a request path to a [`STATIC_FILES`] entry; `/` is the page
/// itself. The returned name is the allowlist's own literal: request
/// input is only ever compared against the list, never used to build a
/// filesystem path, so traversal spellings (`/../`, `%2e%2e`, backslash
/// variants, absolute paths) can only miss the list and 404.
fn static_entry(path: &str) -> Option<&'static (&'static str, &'static str)> {
    let name = if path == "/" { "index.html" } else { path.strip_prefix('/')? };
    STATIC_FILES.iter().find(|(listed, _)| *listed == name)
}

pub struct Server {
    pub config: Config,
    pub agent: Arc<Agent>,
    pub journal: Journal,
}

impl Server {
    /// Bind and serve forever, one thread per connection.
    pub fn serve(&self) -> std::io::Result<()> {
        let listener = TcpListener::bind(&self.config.listen)?;
        // The service clock runs on its own thread: a timer that comes due while
        // nobody is asking a question still fires and still lands in the journal.
        let ticker = self.agent.clone();
        thread::spawn(move || loop {
            thread::sleep(SERVICE_TICK);
            ticker.services.tick();
            // The power manager advances on the same beat: idle timeouts are
            // measured in real time, and a transition is journaled with a reason.
            ticker.power.tick(Some(&ticker.services));
        });
        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    let server = Server { config: self.config.clone(), agent: self.agent.clone(), journal: self.journal.clone() };
                    thread::spawn(move || {
                        let mut stream = stream;
                        if let Err(e) = server.serve_connection(&mut stream) {
                            // A peer that hangs up mid-request is normal; log nothing loud.
                            let _ = writeln!(std::io::stderr(), "agentd: connection closed: {e}");
                        }
                    });
                }
                Err(e) => {
                    let _ = writeln!(std::io::stderr(), "agentd: accept failed: {e}");
                    thread::sleep(Duration::from_millis(50));
                }
            }
        }
        Ok(())
    }

    fn serve_connection(&self, stream: &mut std::net::TcpStream) -> std::io::Result<()> {
        match http::read_request(stream)? {
            Ok(request) => self.route(stream, &request),
            Err(err) => http::respond_text(stream, err.status(), reason_for(err.status()), &err.reason_text()),
        }
    }

    fn route(&self, stream: &mut std::net::TcpStream, request: &Request) -> std::io::Result<()> {
        match (request.method.as_str(), request.path.as_str()) {
            ("GET", "/health") => {
                let body = json!({
                    "ok": true,
                    "events": self.journal.len(),
                    "dropped": self.journal.dropped(),
                    "brains": self.agent.config.brains.iter().map(|b| b.name.clone()).collect::<Vec<_>>(),
                });
                http::respond_json(stream, 200, "OK", &body.to_string())
            }
            ("GET", "/events") => self.stream_events(stream, request),
            // The kimono view's power panel reads this. Read-only by design.
            ("GET", "/power") => http::respond_json(
                stream,
                200,
                "OK",
                &self.agent.power.status(Some(&self.agent.services)).to_string(),
            ),
            ("POST", "/request") => self.handle_request(stream, request),
            ("POST", "/follow-up") => self.handle_follow_up(stream, request),
            ("GET", path) => match static_entry(path) {
                Some(entry) => self.serve_static(stream, entry),
                None => http::respond_text(stream, 404, "Not Found", &format!("no route for GET {path}")),
            },
            (method, path) => http::respond_text(
                stream,
                405,
                "Method Not Allowed",
                &format!("{method} {path} is not supported; try POST /request or GET /events"),
            ),
        }
    }

    /// `POST /request {"text": "...", "confirm": "<label>", "session":
    /// "<id>", "follow_up": true, "asr_confidence": 0.8}` — runs the
    /// whole loop, answers when done. `confirm` authorises exactly the
    /// commit-class call carrying that label (A1.2 broker gate).
    /// `session`/`follow_up` are the CV1 conversation fields: both
    /// optional, both journaled when present. `asr_confidence` is the
    /// CV2 transcription-confidence value (0..=1) the caller measured
    /// for the text — today voiced's acoustic-level proxy; absent
    /// means the caller has no signal and the turn behaves as before
    /// CV2.
    fn handle_request(&self, stream: &mut std::net::TcpStream, request: &Request) -> std::io::Result<()> {
        let fields = match parse_request_full(&request.body_text()) {
            Ok(t) => t,
            Err(e) => return http::respond_json(stream, 400, "Bad Request", &json!({"error": e}).to_string()),
        };
        let summary = self.agent.handle_session_conf_rid(
            &fields.text,
            fields.confirm.as_deref(),
            fields.session.as_deref(),
            fields.follow_up,
            fields.asr_confidence,
            fields.run_id.as_deref(),
        );
        let mut body = json!({
            "run_id": summary.run_id,
            "answer": summary.answer,
            "error": summary.error,
            "brain": summary.brain,
        });
        if let Some(s) = &fields.session {
            body["session"] = json!(s);
        }
        let status = if summary.error.is_some() { 502 } else { 200 };
        http::respond_json(stream, status, if status == 200 { "OK" } else { "Bad Gateway" }, &body.to_string())
    }

    /// `POST /follow-up {"session": "<id>", "state": "open"|"closed"}` —
    /// the wakeword listener reports its follow-up window's real state,
    /// and the state lands in the journal the kimono renders. The
    /// listener owns the microphone; this endpoint exists so the page's
    /// "still listening" indicator is the listener's own report, never
    /// agentd's guess. It carries no request and starts no run.
    fn handle_follow_up(&self, stream: &mut std::net::TcpStream, request: &Request) -> std::io::Result<()> {
        let parsed = parse_follow_up(&request.body_text());
        match parsed {
            Ok((session, state)) => {
                let rec = Recorder::new(self.journal.clone(), &session);
                rec.follow_up_window(&session, &state);
                // The window is part of session state, so the session
                // card updates with it: mirror the report into the
                // store and re-emit the card from the new snapshot.
                self.agent.sessions.set_follow_up(&session, state == "open", std::time::Instant::now());
                if let Some(snap) = self.agent.sessions.snapshot(&session, std::time::Instant::now()) {
                    let (manifest, inputs) =
                        crate::modules::session_card(&snap.id, snap.turn_count, snap.follow_up_open, &snap.pending);
                    if crate::modules::validate_manifest(&manifest).is_empty()
                        && crate::modules::missing_required_inputs(&manifest, &inputs).is_empty()
                    {
                        rec.module_manifest(manifest, inputs);
                    }
                }
                http::respond_json(stream, 200, "OK", "{\"ok\":true}")
            }
            Err(e) => http::respond_json(stream, 400, "Bad Request", &json!({"error": e}).to_string()),
        }
    }

    /// `GET /events` — the open kimono, live.
    fn stream_events(&self, stream: &mut std::net::TcpStream, request: &Request) -> std::io::Result<()> {
        let last_id = request
            .header("last-event-id")
            .or_else(|| request.query_param("after"))
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0);

        // Subscribe first so nothing pushed during the backfill is missed; the
        // `last_sent` guard keeps the backfill from duplicating a live event.
        // `None` means "this reader has seen nothing", so `seq 0` is delivered.
        let rx = self.journal.subscribe();
        let mut last_sent: Option<u64> = if last_id == 0 { None } else { Some(last_id) };
        http::start_sse(stream)?;

        for event in backfill_from(&self.journal, last_id) {
            if last_sent.is_some_and(|sent| event.seq <= sent) {
                continue;
            }
            last_sent = Some(event.seq);
            let line = trace_events::to_json_line(&event).unwrap_or_else(|_| "{\"kind\":\"op6\",\"type\":\"journal_dropped\",\"count\":0,\"reason\":\"serialize failed\"}".to_string());
            http::write_chunk(stream, &format!("id: {}\ndata: {line}\n\n", event.seq))?;
        }

        loop {
            match rx.recv_timeout(SSE_KEEPALIVE) {
                Ok(event) => {
                    if last_sent.is_some_and(|sent| event.seq <= sent) {
                        continue;
                    }
                    last_sent = Some(event.seq);
                    let line = trace_events::to_json_line(&event).unwrap_or_else(|_| "{}".to_string());
                    http::write_chunk(stream, &format!("id: {}\ndata: {line}\n\n", event.seq))?;
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    http::write_chunk(stream, ": keepalive\n\n")?;
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        http::finish_chunked(stream)
    }

    /// The kimono page and its assets. Only the fixed [`STATIC_FILES`]
    /// allowlist is ever read: the router resolves the request path to an
    /// allowlist entry, and the filesystem path is built from that
    /// entry's literal name -- never from request input -- so there is
    /// no traversal to guard.
    fn serve_static(&self, stream: &mut std::net::TcpStream, entry: &'static (&'static str, &'static str)) -> std::io::Result<()> {
        let Some(dir) = self.config.static_dir.as_ref() else {
            return http::respond_text(
                stream,
                200,
                "OK",
                "agentd is up. POST /request {\"text\": \"...\"} or GET /events for the live journal.\n",
            );
        };
        let (name, content_type) = entry;
        match std::fs::read(dir.join(name)) {
            Ok(bytes) => http::write_response(stream, 200, "OK", content_type, &bytes),
            Err(e) => http::respond_text(stream, 500, "Internal Server Error", &format!("static_dir {}: {e}", dir.display())),
        }
    }
}

/// What a fresh or reconnecting reader needs: everything after `last_id`, and
/// for a brand-new reader (`last_id == 0`) the whole held journal — `seq 0` is
/// a real event, not a sentinel, so it must not be skipped.
fn backfill_from(journal: &Journal, last_id: u64) -> Vec<trace_events::Event> {
    if last_id == 0 {
        journal.snapshot()
    } else {
        journal.since(last_id)
    }
}

/// Validate the request body. The error text is what the client sees, so it
/// names exactly what was wrong.
pub fn parse_request_text(body: &str) -> Result<String, String> {
    parse_request(body).map(|(text, _)| text)
}

/// Session ids are minted by the wakeword listener (`sess-<epoch>-<n>`);
/// the wire rule is deliberately narrow so an id is always safe to log,
/// journal and use as a map key: 1-64 chars of `[A-Za-z0-9._-]`.
pub fn valid_session_id(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// Run ids are minted by voiced for CV3 streaming (`run-v<micros>-<pid>`)
/// or by agentd itself (`run-<micros>-<n>`); the wire rule accepts
/// exactly that shape: the `run-` prefix plus 1-60 chars of
/// `[A-Za-z0-9._-]`, so an adopted id is always safe to log, journal
/// and use as a correlation key.
pub fn valid_run_id(s: &str) -> bool {
    s.len() > 4 && s.len() <= 64 && s.starts_with("run-") && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// Everything a `/request` body may carry, validated.
pub struct RequestFields {
    pub text: String,
    pub confirm: Option<String>,
    pub session: Option<String>,
    pub follow_up: bool,
    /// CV2: the caller's transcription confidence for `text`, when
    /// it has one (0..=1). Validated like every other field.
    pub asr_confidence: Option<f32>,
    /// CV3: a caller-minted run id to adopt, when the caller needs
    /// to know the run's id before the run starts (voiced's
    /// streaming tail). Validated like the session id, plus the
    /// `run-` prefix every run id carries.
    pub run_id: Option<String>,
}

/// The full request: the text, plus the caller's explicit confirmation
/// for one commit-class label when they gave one.
pub fn parse_request(body: &str) -> Result<(String, Option<String>), String> {
    parse_request_full(body).map(|f| (f.text, f.confirm))
}

/// The full request, including the CV1 conversation fields.
pub fn parse_request_full(body: &str) -> Result<RequestFields, String> {
    if body.trim().is_empty() {
        return Err("empty body: expected {\"text\": \"...\"}".to_string());
    }
    let value: Value = serde_json::from_str(body).map_err(|e| format!("body is not JSON: {e}"))?;
    let text = value
        .get("text")
        .and_then(Value::as_str)
        .ok_or_else(|| "body must be an object with a string field `text`".to_string())?;
    if text.trim().is_empty() {
        return Err("`text` must not be empty".to_string());
    }
    if text.len() > 4000 {
        return Err(format!("`text` is {} bytes; the limit is 4000", text.len()));
    }
    let confirm = match value.get("confirm") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) if !s.trim().is_empty() && s.len() <= 200 => Some(s.clone()),
        Some(_) => return Err("field `confirm` must be a non-empty string naming the action being confirmed".to_string()),
    };
    let session = match value.get("session") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) if valid_session_id(s) => Some(s.clone()),
        Some(_) => return Err("field `session` must be a session id (1-64 chars of [A-Za-z0-9._-])".to_string()),
    };
    let follow_up = match value.get("follow_up") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err("field `follow_up` must be a boolean".to_string()),
    };
    let asr_confidence = match value.get("asr_confidence") {
        None | Some(Value::Null) => None,
        Some(v) => match v.as_f64() {
            Some(c) if (0.0..=1.0).contains(&c) => Some(c as f32),
            _ => return Err("field `asr_confidence` must be a number in [0, 1]".to_string()),
        },
    };
    let run_id = match value.get("run_id") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) if valid_run_id(s) => Some(s.clone()),
        Some(_) => return Err("field `run_id` must be a run id (`run-` plus 1-60 chars of [A-Za-z0-9._-])".to_string()),
    };
    Ok(RequestFields { text: text.to_string(), confirm, session, follow_up, asr_confidence, run_id })
}

/// Validate a `/follow-up` body: the session id and the window state,
/// nothing else. The state vocabulary is exactly the two transitions
/// the listener performs.
pub fn parse_follow_up(body: &str) -> Result<(String, String), String> {
    if body.trim().is_empty() {
        return Err("empty body: expected {\"session\": \"...\", \"state\": \"open\"|\"closed\"}".to_string());
    }
    let value: Value = serde_json::from_str(body).map_err(|e| format!("body is not JSON: {e}"))?;
    let session = value
        .get("session")
        .and_then(Value::as_str)
        .filter(|s| valid_session_id(s))
        .ok_or_else(|| "field `session` must be a session id (1-64 chars of [A-Za-z0-9._-])".to_string())?;
    let state = value
        .get("state")
        .and_then(Value::as_str)
        .filter(|s| matches!(*s, "open" | "closed"))
        .ok_or_else(|| "field `state` must be \"open\" or \"closed\"".to_string())?;
    Ok((session.to_string(), state.to_string()))
}

fn reason_for(status: u16) -> &'static str {
    match status {
        400 => "Bad Request",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        413 => "Payload Too Large",
        _ => "Error",
    }
}
