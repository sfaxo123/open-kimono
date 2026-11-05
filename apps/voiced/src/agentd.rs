//! The agentd client: `POST /request`, then tail `GET /events` for the answer
//! to speak. voiced never invents an answer — it reads the same journal the
//! kimono view shows, which is what keeps voice and screen telling the truth
//! about the same run (VISION §3). The answer is bound to the run id agentd
//! returns for the POST: the tail accepts only that run's events, so a
//! backlog replay can never substitute an older run's answer.
//!
//! Read discipline (hardened after the CV1 isolation incident, when a
//! spoken answer was lost while agentd completed the run correctly):
//! agentd's `/request` is synchronous — it writes nothing until the run
//! finishes — so the POST response is awaited on the caller's answer
//! budget, never on a short fixed timeout. Every non-streaming read is a
//! full response (head + the body its content-length promises); a read
//! that ends early is an error with the byte counts in it, never a
//! silently truncated buffer. Transport failures before a response or
//! stream is established (connect refused, empty reply, no status line,
//! a head or body cut short) are retried a bounded number of times with
//! small backoff; once established, failures stay loud and are never
//! retried into a state the run_id bracket has not vetted.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use trace_events::{AgUiEvent, HandoffStatus, Op6Event, Payload};

use crate::capture::JournalOut;
use crate::error::VoiceError;

#[derive(Clone, Debug, PartialEq)]
pub struct Endpoint {
    pub host: String,
    pub port: u16,
}

impl Endpoint {
    pub fn parse(url: &str) -> Result<Endpoint, VoiceError> {
        let rest = url
            .strip_prefix("http://")
            .ok_or_else(|| VoiceError::Config(format!("agentd URL must start with http://, got {url:?}")))?;
        let (host_port, path) = match rest.find('/') {
            Some(i) => (&rest[..i], &rest[i..]),
            None => (rest, ""),
        };
        if !path.is_empty() && path != "/" {
            return Err(VoiceError::Config(format!("agentd URL must not carry a path, got {url:?}")));
        }
        let (host, port) = host_port
            .rsplit_once(':')
            .ok_or_else(|| VoiceError::Config(format!("agentd URL must carry a port, got {url:?}")))?;
        if host.is_empty() {
            return Err(VoiceError::Config(format!("agentd URL has no host, got {url:?}")));
        }
        let port: u16 = port
            .parse()
            .map_err(|_| VoiceError::Config(format!("agentd URL port is not a number, got {url:?}")))?;
        Ok(Endpoint { host: host.to_string(), port })
    }

    pub fn addr(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

fn emit(j: &mut JournalOut, trace: trace_events::TraceId, payload: Payload) {
    let _ = j.emit(trace, None, None, payload);
}

fn connect(ep: &Endpoint, j: &mut JournalOut, trace: trace_events::TraceId, what: &str) -> Result<TcpStream, VoiceError> {
    TcpStream::connect(ep.addr()).map_err(|e| {
        emit(j, trace, Payload::Op6(Op6Event::Handoff { destination: format!("agentd@{}", ep.addr()), reason: format!("{what}: {e}"), status: HandoffStatus::Failed }));
        VoiceError::AgentUnreachable { endpoint: ep.addr(), detail: e.to_string() }
    })
}

/// How many times one read path is attempted before the turn fails.
/// Only failures that happen before the peer's answer is established
/// are ever retried (see [`AttemptFail`]); an established stream that
/// breaks mid-run fails the turn on the first attempt, as before.
const MAX_ATTEMPTS: u32 = 3;
/// The pauses between attempts: small, because the failures this covers
/// are a restarting or momentarily unreachable agentd on loopback.
const RETRY_BACKOFF: [Duration; 2] = [Duration::from_millis(250), Duration::from_millis(750)];

/// A failed attempt at one of the read paths. `retryable` is true only
/// for transport failures that happened before anything was established
/// — no response bytes accepted, no stream head validated — where
/// starting over cannot duplicate or corrupt anything: the POST retry
/// still speaks only the run its own response names, and a re-connected
/// tail still accepts only events inside that run's bracket. A response
/// the server actually gave (a refusal, a 2xx with no run id) is never
/// retryable: the server has spoken, and the failure is the turn's own.
struct AttemptFail {
    err: VoiceError,
    retryable: bool,
}

impl AttemptFail {
    fn retryable(err: VoiceError) -> Self {
        AttemptFail { err, retryable: true }
    }
    fn fatal(err: VoiceError) -> Self {
        AttemptFail { err, retryable: false }
    }
}

/// Journal a failed attempt that is about to be retried, so the kimono
/// shows the recovery instead of a turn that mysteriously took longer.
fn note_retry(j: &mut JournalOut, trace: trace_events::TraceId, ep: &Endpoint, what: &str, attempt: u32, err: &VoiceError) {
    emit(
        j,
        trace,
        Payload::Op6(Op6Event::Handoff {
            destination: format!("agentd@{}", ep.addr()),
            reason: format!("{what} attempt {attempt} of {MAX_ATTEMPTS} failed before anything was established ({err}); retrying"),
            status: HandoffStatus::Failed,
        }),
    );
}

/// The highest `seq` agentd has already written, from `GET /health`'s event
/// count. Used only as the `Last-Event-ID` resume hint for the tail, to
/// skip replaying the backlog. It is NOT a correctness input: the count
/// is the number of records the journal ring currently holds, which parts
/// company with the last seq as soon as the ring evicts, and either way
/// the spoken answer must not depend on it — `tail_answer` binds the
/// answer to the run id instead. A server that reports nothing yields 0,
/// which means "send me everything": the safe direction. For the same
/// reason this probe is not retried: a failed hint costs replay, never
/// an answer, and retrying it would tax every healthy turn.
pub fn journal_last_seq(ep: &Endpoint) -> u64 {
    let Ok(mut sock) = TcpStream::connect(ep.addr()) else { return 0 };
    sock.set_read_timeout(Some(Duration::from_secs(5))).ok();
    if sock.write_all(format!("GET /health HTTP/1.1\r\nHost: {}\r\nconnection: close\r\n\r\n", ep.addr()).as_bytes()).is_err() {
        return 0;
    }
    let Ok(raw) = read_full_response(&mut sock) else { return 0 };
    let body = body_of(&String::from_utf8_lossy(&raw));
    serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|v| v.get("events").and_then(|e| e.as_u64()))
        // `events` is a count; the last id written is one below it.
        .map(|n| n.saturating_sub(1))
        .unwrap_or(0)
}

/// Send the request and return the run id agentd assigned to it. That id
/// is the answer's identity: `tail_answer` will accept only the run
/// carrying it. voiced does not render or speak the request itself: the
/// journal carries it, so the kimono shows it exactly once.
///
/// `timeout` is the caller's answer budget. agentd runs the whole loop
/// before writing the response, so the wait for this response IS the
/// wait for the run; a shorter fixed timeout would abandon healthy
/// in-flight runs and lose their answers (the CV1 isolation incident:
/// the old 10 s read timeout fired while agentd finished the run and
/// journaled the correct answer).
pub fn post_request(ep: &Endpoint, text: &str, timeout: Duration, j: &mut JournalOut) -> Result<String, VoiceError> {
    post_request_session(ep, text, None, false, None, timeout, j)
}

/// The same POST, carrying the CV1 conversation fields when the turn
/// belongs to a session: `session` names the conversation in agentd
/// (its earlier turns and entities become this turn's context) and
/// `follow_up` marks a turn the listener accepted without the wake
/// word inside the follow-up window. Both are omitted — not nulled —
/// for a sessionless turn, so the old wire shape is byte-identical.
/// CV2 adds `asr_confidence` under the same rule: present only when
/// the STT engine reported one for this turn.
pub fn post_request_session(ep: &Endpoint, text: &str, session: Option<&str>, follow_up: bool, asr_confidence: Option<f32>, timeout: Duration, j: &mut JournalOut) -> Result<String, VoiceError> {
    post_request_session_rid(ep, text, session, follow_up, asr_confidence, None, timeout, j)
}

/// Mint the run id voiced asks agentd to adopt (CV3): unique per
/// turn, and inside agentd's accepted shape (`run-` plus up to 60
/// chars of `[A-Za-z0-9._-]`). Uniqueness comes from the microsecond
/// clock and the pid — no shared counter, no coordination.
pub fn mint_run_id() -> String {
    let micros = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_micros())
        .unwrap_or(0);
    format!("run-v{micros}-{}", std::process::id())
}

/// The same POST as [`post_request_session`], additionally carrying
/// the caller-minted `run_id` (CV3): agentd adopts it when it
/// understands the field, which lets the tail start *before* the
/// POST returns and still bind to this turn's run by identity. A
/// pre-CV3 agentd ignores the field and names its own run; the id
/// in the response is the truth either way.
pub fn post_request_session_rid(
    ep: &Endpoint,
    text: &str,
    session: Option<&str>,
    follow_up: bool,
    asr_confidence: Option<f32>,
    run_id: Option<&str>,
    timeout: Duration,
    j: &mut JournalOut,
) -> Result<String, VoiceError> {
    let trace = j.new_trace();
    emit(j, trace, Payload::Op6(Op6Event::Handoff { destination: format!("agentd@{}", ep.addr()), reason: format!("POST /request {:?}", truncate(text, 60)), status: HandoffStatus::Started }));
    let mut attempt = 1;
    loop {
        match post_request_once(ep, text, session, follow_up, asr_confidence, run_id, timeout, j, trace) {
            Ok(id) => return Ok(id),
            Err(fail) => {
                if !fail.retryable || attempt >= MAX_ATTEMPTS {
                    return Err(fail.err);
                }
                note_retry(j, trace, ep, "POST /request", attempt, &fail.err);
                std::thread::sleep(RETRY_BACKOFF[(attempt - 1) as usize]);
                attempt += 1;
            }
        }
    }
}

/// One attempt at the POST. See [`AttemptFail`] for what may be retried:
/// only what failed before a complete response arrived. A retry can
/// start a second run server-side if the first attempt's response was
/// lost after agentd acted on it — bounded (three attempts), journaled
/// per attempt, and the spoken answer still binds to the run the
/// accepted response names, never to the lost one.
fn post_request_once(ep: &Endpoint, text: &str, session: Option<&str>, follow_up: bool, asr_confidence: Option<f32>, run_id: Option<&str>, timeout: Duration, j: &mut JournalOut, trace: trace_events::TraceId) -> Result<String, AttemptFail> {
    let mut sock = connect(ep, j, trace, "POST /request connect").map_err(AttemptFail::retryable)?;
    sock.set_read_timeout(Some(timeout)).ok();
    sock.set_write_timeout(Some(Duration::from_secs(10))).ok();
    let mut obj = serde_json::json!({ "text": text });
    if let Some(s) = session {
        obj["session"] = serde_json::Value::String(s.to_string());
    }
    if follow_up {
        obj["follow_up"] = serde_json::Value::Bool(true);
    }
    if let Some(c) = asr_confidence {
        obj["asr_confidence"] = serde_json::json!(c);
    }
    if let Some(r) = run_id {
        obj["run_id"] = serde_json::Value::String(r.to_string());
    }
    let body = obj.to_string();
    let head = format!(
        "POST /request HTTP/1.1\r\nHost: {}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        ep.addr(),
        body.len()
    );
    sock.write_all(head.as_bytes()).map_err(|e| AttemptFail::retryable(VoiceError::AgentUnreachable { endpoint: ep.addr(), detail: format!("write: {e}") }))?;

    let raw = read_full_response(&mut sock).map_err(|detail| AttemptFail::retryable(VoiceError::AgentUnreachable { endpoint: ep.addr(), detail }))?;
    let head_text = String::from_utf8_lossy(&raw).to_string();
    let status = status_code(&head_text)
        .ok_or_else(|| AttemptFail::retryable(VoiceError::AgentUnreachable { endpoint: ep.addr(), detail: "no HTTP status line".into() }))?;
    let resp_body = body_of(&head_text);
    if !(200..300).contains(&status) {
        emit(j, trace, Payload::Op6(Op6Event::Handoff { destination: format!("agentd@{}", ep.addr()), reason: format!("refused with HTTP {status}: {}", truncate(&resp_body, 120)), status: HandoffStatus::Failed }));
        return Err(AttemptFail::fatal(VoiceError::AgentRefused { endpoint: ep.addr(), status, detail: truncate(&resp_body, 200) }));
    }
    // The response names the run this POST started. Without that id the
    // answer cannot be told apart from any other run's, so the turn fails
    // here rather than guess from the journal afterwards.
    let run_id = serde_json::from_str::<serde_json::Value>(&resp_body)
        .ok()
        .and_then(|v| v.get("run_id").and_then(|r| r.as_str()).map(str::to_string))
        .filter(|r| !r.is_empty());
    match run_id {
        Some(id) => Ok(id),
        None => {
            emit(j, trace, Payload::Op6(Op6Event::Handoff { destination: format!("agentd@{}", ep.addr()), reason: "POST response carried no run_id".into(), status: HandoffStatus::Failed }));
            Err(AttemptFail::fatal(VoiceError::NoAnswer { endpoint: ep.addr(), detail: format!("the POST response carried no run_id, so no journal answer can be correlated to this request: {}", truncate(&resp_body, 120)) }))
        }
    }
}

/// Tail the journal until the run `run_id` — the run our own POST
/// started, named in its response — finishes, and return that run's
/// answer.
///
/// Correlation is by identity, never by position. Events are accepted
/// only inside our run's bracket (its RUN_STARTED … RUN_FINISHED, or a
/// text message carrying the run id as its messageId, which is how
/// agentd journals the answer); other runs in the stream — the whole
/// backlog, when the resume hint undershoots — are stepped over. The
/// `after` hint only skips replay work. If our run never appears, errors,
/// or finishes without an assistant message, the turn fails instead of
/// speaking some other run's words.
pub fn tail_answer(ep: &Endpoint, request: &str, run_id: &str, j: &mut JournalOut, timeout: Duration, after: u64) -> Result<String, VoiceError> {
    let trace = j.new_trace();
    let mut reader = tail_establish(ep, j, trace, timeout, after)?;

    let mut in_ours = false;
    let mut foreign_open: u32 = 0;
    let mut confirmed = false;
    let mut cur: Option<(String, bool)> = None;
    let mut buffer = String::new();
    let mut answer: Option<String> = None;
    let mut finished = false;
    let mut data = String::new();

    'stream: loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut || e.kind() == std::io::ErrorKind::WouldBlock => {
                emit(j, trace, Payload::Op6(Op6Event::Handoff { destination: format!("agentd@{}", ep.addr()), reason: format!("no run finished within {} s", timeout.as_secs()), status: HandoffStatus::Failed }));
                return Err(VoiceError::NoAnswer { endpoint: ep.addr(), detail: format!("timed out after {} s", timeout.as_secs()) });
            }
            Err(e) => return Err(VoiceError::AgentUnreachable { endpoint: ep.addr(), detail: format!("read: {e}") }),
        }
        let line = line.trim_end_matches(['\r', '\n']);
        if !line.is_empty() {
            if let Some(rest) = line.strip_prefix("data:") {
                if !data.is_empty() {
                    data.push('\n');
                }
                data.push_str(rest.strip_prefix(' ').unwrap_or(rest));
            }
            continue;
        }
        if data.is_empty() {
            continue;
        }
        let frame = std::mem::take(&mut data);
        let ev: serde_json::Value = match serde_json::from_str(&frame) {
            Ok(v) => v,
            Err(e) => {
                emit(j, trace, Payload::AgUi(AgUiEvent::RunError { message: format!("journal line rejected: {e}"), code: Some("bad_journal_line".into()), usage: vec![] }));
                continue;
            }
        };
        let payload = ev.get("payload").cloned().unwrap_or(serde_json::Value::Null);
        let kind = payload.get("kind").and_then(|k| k.as_str()).unwrap_or("");
        let ptype = payload.get("type").and_then(|t| t.as_str()).unwrap_or("");
        if kind != "ag_ui" {
            continue;
        }
        let ev_run = payload.get("runId").and_then(|r| r.as_str()).unwrap_or("");
        match ptype {
            "RUN_STARTED" => {
                if ev_run == run_id {
                    in_ours = true;
                    if payload.get("input").and_then(|i| i.get("text")).and_then(|t| t.as_str()) == Some(request) {
                        confirmed = true;
                    }
                } else {
                    foreign_open += 1;
                }
            }
            "TEXT_MESSAGE_START" => {
                let mid = payload.get("messageId").and_then(|m| m.as_str()).unwrap_or("");
                if mid == run_id {
                    in_ours = true;
                }
                let role = payload.get("role").and_then(|r| r.as_str()).unwrap_or("assistant").to_string();
                let collect = mid == run_id || (in_ours && foreign_open == 0);
                cur = Some((role, collect));
                buffer.clear();
            }
            "TEXT_MESSAGE_CONTENT" => {
                if cur.as_ref().map(|(_, c)| *c).unwrap_or(false) {
                    buffer.push_str(payload.get("delta").and_then(|d| d.as_str()).unwrap_or(""));
                }
            }
            "TEXT_MESSAGE_END" => {
                if let Some((role, collect)) = cur.take() {
                    if collect {
                        if role == "user" {
                            if buffer == request {
                                confirmed = true;
                            }
                        } else if role == "assistant" {
                            answer = Some(std::mem::take(&mut buffer));
                        }
                    }
                }
                buffer.clear();
            }
            "RUN_FINISHED" => {
                if ev_run == run_id {
                    finished = true;
                    break 'stream;
                }
                foreign_open = foreign_open.saturating_sub(1);
            }
            "RUN_ERROR" => {
                // A run error carries no run id of its own; it is ours
                // only while our bracket is the one open.
                if in_ours && foreign_open == 0 {
                    let msg = payload.get("message").and_then(|m| m.as_str()).unwrap_or("run error").to_string();
                    emit(j, trace, Payload::Op6(Op6Event::Handoff { destination: format!("agentd@{}", ep.addr()), reason: format!("run failed: {}", truncate(&msg, 120)), status: HandoffStatus::Failed }));
                    return Err(VoiceError::NoAnswer { endpoint: ep.addr(), detail: msg });
                }
            }
            _ => {}
        }
    }

    if !finished && answer.is_none() {
        // The stream ended (or our run never appeared) with no completed
        // answer message from our run: there is nothing this turn may
        // speak, and saying so beats guessing.
        emit(j, trace, Payload::Op6(Op6Event::Handoff { destination: format!("agentd@{}", ep.addr()), reason: format!("run {run_id} never finished in the journal"), status: HandoffStatus::Failed }));
        return Err(VoiceError::NoAnswer { endpoint: ep.addr(), detail: format!("run {run_id} never finished in the journal") });
    }
    if !confirmed {
        // Not a failure, but worth saying: the run journalled no user
        // message and no request text on its start record, so the
        // request side of the correlation rests on the run id alone.
        emit(j, trace, Payload::Op6(Op6Event::JournalDropped { count: 0, reason: "the server journals no user message for this run: the answer was correlated by the run id from the POST response, not by the request text".into() }));
    }
    match answer.filter(|a| !a.trim().is_empty()) {
        Some(a) => {
            emit(j, trace, Payload::Op6(Op6Event::Handoff { destination: format!("agentd@{}", ep.addr()), reason: "answer received".into(), status: HandoffStatus::Completed }));
            Ok(a)
        }
        None => {
            emit(j, trace, Payload::Op6(Op6Event::Handoff { destination: format!("agentd@{}", ep.addr()), reason: "the run produced no assistant message".into(), status: HandoffStatus::Failed }));
            Err(VoiceError::NoAnswer { endpoint: ep.addr(), detail: "the run produced no assistant message".into() })
        }
    }
}

/// Connect the `/events` tail and read its response head with full
/// discipline — a status line that parses, a 200, and the terminating
/// blank line — retrying transport failures that happen before the
/// stream exists. Nothing has been accepted from the stream at this
/// point, so reconnecting cannot double-accept an event; once the head
/// is validated the stream is established and its failures belong to
/// `tail_answer`'s state machine, loud and unretried.
fn tail_establish(ep: &Endpoint, j: &mut JournalOut, trace: trace_events::TraceId, timeout: Duration, after: u64) -> Result<BufReader<TcpStream>, VoiceError> {
    let mut attempt = 1;
    loop {
        match tail_connect_once(ep, j, trace, timeout, after) {
            Ok(reader) => return Ok(reader),
            Err(fail) => {
                if !fail.retryable || attempt >= MAX_ATTEMPTS {
                    return Err(fail.err);
                }
                note_retry(j, trace, ep, "GET /events", attempt, &fail.err);
                std::thread::sleep(RETRY_BACKOFF[(attempt - 1) as usize]);
                attempt += 1;
            }
        }
    }
}

fn tail_connect_once(ep: &Endpoint, j: &mut JournalOut, trace: trace_events::TraceId, timeout: Duration, after: u64) -> Result<BufReader<TcpStream>, AttemptFail> {
    let mut sock = connect(ep, j, trace, "GET /events connect").map_err(AttemptFail::retryable)?;
    sock.set_read_timeout(Some(timeout)).ok();
    // No header at all means "I have seen nothing" (agentd's L1-D6 rule);
    // `last-event-id: 0` would mean "skip seq 0".
    let resume = if after > 0 { format!("last-event-id: {after}\r\n") } else { String::new() };
    sock.write_all(format!("GET /events HTTP/1.1\r\nHost: {}\r\naccept: text/event-stream\r\n{resume}connection: close\r\n\r\n", ep.addr()).as_bytes())
        .map_err(|e| AttemptFail::retryable(VoiceError::AgentUnreachable { endpoint: ep.addr(), detail: format!("write: {e}") }))?;

    let mut reader = BufReader::new(sock);
    let mut status_line = String::new();
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => {
                // The peer hung up before (or in the middle of) the
                // head: the stream was never established. The old code
                // treated this EOF as "head complete" and went on to
                // report the run as never finished.
                let detail = if status_line.is_empty() {
                    "empty reply: /events closed before its response head".to_string()
                } else {
                    "response head cut short: /events closed mid-head".to_string()
                };
                return Err(AttemptFail::retryable(VoiceError::AgentUnreachable { endpoint: ep.addr(), detail }));
            }
            Ok(_) => {
                if status_line.is_empty() {
                    status_line = line.trim_end().to_string();
                    match status_code(&status_line) {
                        None => {
                            return Err(AttemptFail::retryable(VoiceError::AgentUnreachable { endpoint: ep.addr(), detail: "no HTTP status line".into() }));
                        }
                        Some(200) => {}
                        Some(status) => {
                            return Err(AttemptFail::fatal(VoiceError::AgentRefused { endpoint: ep.addr(), status, detail: "GET /events did not open a stream".into() }));
                        }
                    }
                }
                if line.trim().is_empty() {
                    break;
                }
            }
            Err(e) => {
                return Err(AttemptFail::retryable(VoiceError::AgentUnreachable { endpoint: ep.addr(), detail: format!("read head: {e}") }));
            }
        }
    }
    Ok(reader)
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
}

fn status_code(head: &str) -> Option<u16> {
    let first = head.lines().next()?;
    let mut parts = first.split_whitespace();
    parts.next()?;
    parts.next()?.parse().ok()
}

fn body_of(head: &str) -> String {
    match head.split_once("\r\n\r\n") {
        Some((_, b)) => b.to_string(),
        None => String::new(),
    }
}

/// The `content-length` a response head promises, if it promises one.
fn promised_body(head: &str) -> Option<usize> {
    head.lines().find_map(|l| {
        l.to_ascii_lowercase()
            .strip_prefix("content-length:")
            .and_then(|v| v.trim().parse::<usize>().ok())
    })
}

/// Read a whole non-streaming response: the head, then exactly as much
/// of the body as its `content-length` promises. The read is honest
/// about how it ended — agentd writes the head and the body in two
/// separate writes, so stopping at the end of the head (the old, old
/// behaviour) usually meant reading no body at all, and swallowing a
/// read error (the behaviour after that) silently truncated whatever
/// had arrived. Now a response that ends early — mid-head, or short of
/// its promised body — is an `Err` naming the byte counts, and only a
/// complete response (or one whose head promised no length, ended by
/// the peer's close) is `Ok`.
fn read_full_response(sock: &mut TcpStream) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut buf = [0u8; 1024];
    loop {
        let head_end = out.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 4);
        if let Some(he) = head_end {
            if let Some(want) = promised_body(&String::from_utf8_lossy(&out[..he])) {
                if out.len() - he >= want {
                    return Ok(out);
                }
            }
        }
        match sock.read(&mut buf) {
            Ok(0) => {
                return match head_end {
                    None => Err(if out.is_empty() {
                        "empty reply: the connection closed with no bytes".to_string()
                    } else {
                        format!("response ended before its head completed ({} bytes read)", out.len())
                    }),
                    Some(he) => match promised_body(&String::from_utf8_lossy(&out[..he])) {
                        Some(want) => Err(format!("response body cut short: {} of {want} promised bytes arrived", out.len() - he)),
                        None => Ok(out),
                    },
                };
            }
            Ok(n) => out.extend_from_slice(&buf[..n]),
            Err(e) => return Err(format!("read: {e}")),
        }
    }
}



// ---------------------------------------------------------------------
// CV3: the streaming tail. Same journal, same run-id bracket, same
// correlation discipline as `tail_answer` — but the tail starts
// *before* the POST returns (the run id is caller-minted and posted
// with the request, so the bracket is known in advance), the POST
// resolves on its own thread through `signals`, and each
// `answer_chunk` record of our run goes straight to the sink:
// speech starts while the brain is still generating, instead of
// after the whole answer exists.
//
// The POST's word is still the run's identity. Three resolutions:
// - It adopts the minted id (a CV3 agentd): the live bracket was
//   the right one all along; nothing changes.
// - It names a DIFFERENT run (a pre-CV3 agentd, which ignores the
//   minted id): the tail rebinds — the frames it stepped over are
//   replayed from its own buffer under the adopted id (no second
//   connection, no re-tail), and the answer comes back whole, the
//   classic way. If the adopted run's bracket was left open by a
//   stream that died, the turn fails loudly instead: the hardening
//   contract — a bracket whose state was in flux is never
//   re-entered. If the adopted run never appeared in the stream,
//   the classic tail runs on its id as the last resort.
// - It refuses (AgentRefused / NoAnswer): the server has spoken for
//   this run; the tail stops at once. It fails in transport
//   (AgentUnreachable): the response may have been lost while the
//   run lives on — the tail keeps its bracket to the deadline, and
//   a stream that ends early waits for the POST's word before
//   concluding anything.
//
// An `answer_stream` record with mode `preamble` for our run tells
// the sink the chunks it spoke were a preamble, not the answer;
// the caller speaks the final answer whole as well.

/// The request thread's resolution, delivered while the tail runs.
pub enum PostSignal {
    /// The POST returned this run id.
    Adopted(String),
    /// The POST failed with this error.
    Failed(VoiceError),
}

/// The streaming tail's side effects, owned by the caller.
pub trait ChunkSink {
    /// A complete sentence of this run's answer, in order.
    fn on_chunk(&mut self, text: &str);
    /// The chunks spoken so far were a preamble, not the answer.
    fn on_preamble(&mut self);
}

/// What the POST has said so far, as the tail knows it.
#[derive(Default)]
struct PostState {
    adopted: Option<String>,
    /// A transport failure: recorded, not immediately fatal.
    err: Option<VoiceError>,
    /// A server-spoken failure: the tail stops on it at once.
    refusal: Option<VoiceError>,
}

fn absorb_signal(state: &mut PostState, signal: PostSignal) {
    match signal {
        PostSignal::Adopted(id) => {
            if state.adopted.is_none() {
                state.adopted = Some(id);
            }
        }
        PostSignal::Failed(e) => match e {
            VoiceError::AgentUnreachable { .. } => {
                if state.err.is_none() {
                    state.err = Some(e);
                }
            }
            other => {
                if state.refusal.is_none() {
                    state.refusal = Some(other);
                }
            }
        },
    }
}

fn drain_post_signals(signals: &std::sync::mpsc::Receiver<PostSignal>, state: &mut PostState) {
    while let Ok(signal) = signals.try_recv() {
        absorb_signal(state, signal);
    }
}

enum BracketStep {
    Continue,
    Finished,
    /// Our run errored; the error is the turn's.
    Failed(VoiceError),
}

/// The run-id bracket machine: frames in, the run's own answer out.
/// Shared by the live tail (bound to the minted id, dispatching
/// chunks) and the handoff rebind (bound to an adopted id, dispatch
/// off). Correlation is by identity, never by position — the
/// cce8a80 discipline, in one place.
struct Bracket {
    run_id: String,
    request: String,
    in_ours: bool,
    saw_start: bool,
    foreign_open: u32,
    confirmed: bool,
    cur: Option<(String, bool)>,
    text: String,
    answer: Option<String>,
    finished: bool,
}

impl Bracket {
    fn new(run_id: &str, request: &str) -> Bracket {
        Bracket {
            run_id: run_id.to_string(),
            request: request.to_string(),
            in_ours: false,
            saw_start: false,
            foreign_open: 0,
            confirmed: false,
            cur: None,
            text: String::new(),
            answer: None,
            finished: false,
        }
    }

    fn apply(&mut self, ev: &serde_json::Value, sink: Option<&mut dyn ChunkSink>) -> BracketStep {
        let payload = ev.get("payload").cloned().unwrap_or(serde_json::Value::Null);
        let kind = payload.get("kind").and_then(|k| k.as_str()).unwrap_or("");
        let ptype = payload.get("type").and_then(|t| t.as_str()).unwrap_or("");
        if kind != "ag_ui" {
            return BracketStep::Continue;
        }
        let ev_run = payload.get("runId").and_then(|r| r.as_str()).unwrap_or("");
        match ptype {
            "RUN_STARTED" => {
                if ev_run == self.run_id {
                    self.in_ours = true;
                    self.saw_start = true;
                    if payload.get("input").and_then(|i| i.get("text")).and_then(|t| t.as_str())
                        == Some(self.request.as_str())
                    {
                        self.confirmed = true;
                    }
                } else {
                    self.foreign_open += 1;
                }
            }
            "TEXT_MESSAGE_START" => {
                let mid = payload.get("messageId").and_then(|m| m.as_str()).unwrap_or("");
                if mid == self.run_id {
                    self.in_ours = true;
                    self.saw_start = true;
                }
                let role = payload.get("role").and_then(|r| r.as_str()).unwrap_or("assistant").to_string();
                let collect = mid == self.run_id || (self.in_ours && self.foreign_open == 0);
                self.cur = Some((role, collect));
                self.text.clear();
            }
            "TEXT_MESSAGE_CONTENT" => {
                if self.cur.as_ref().map(|(_, c)| *c).unwrap_or(false) {
                    self.text.push_str(payload.get("delta").and_then(|d| d.as_str()).unwrap_or(""));
                }
            }
            "TEXT_MESSAGE_END" => {
                if let Some((role, collect)) = self.cur.take() {
                    if collect {
                        if role == "user" {
                            if self.text == self.request {
                                self.confirmed = true;
                            }
                        } else if role == "assistant" {
                            self.answer = Some(std::mem::take(&mut self.text));
                        }
                    }
                }
                self.text.clear();
            }
            "CUSTOM" => {
                // CV3 chunk records, accepted only inside our own
                // bracket — the same discipline as every other frame.
                if let Some(sink) = sink {
                    if self.in_ours && self.foreign_open == 0 {
                        let name = payload.get("name").and_then(|n| n.as_str()).unwrap_or("");
                        let value = payload.get("value").cloned().unwrap_or(serde_json::Value::Null);
                        let vrun = value.get("run_id").and_then(|r| r.as_str()).unwrap_or("");
                        if vrun == self.run_id {
                            if name == "answer_chunk" {
                                if let Some(text) = value.get("text").and_then(|t| t.as_str()) {
                                    if !text.is_empty() {
                                        sink.on_chunk(text);
                                    }
                                }
                            } else if name == "answer_stream"
                                && value.get("mode").and_then(|m| m.as_str()) == Some("preamble")
                            {
                                sink.on_preamble();
                            }
                        }
                    }
                }
            }
            "RUN_FINISHED" => {
                if ev_run == self.run_id {
                    self.finished = true;
                    return BracketStep::Finished;
                }
                self.foreign_open = self.foreign_open.saturating_sub(1);
            }
            "RUN_ERROR" => {
                // A run error carries no run id of its own; it is ours
                // only while our bracket is the one open.
                if self.in_ours && self.foreign_open == 0 {
                    let msg = payload.get("message").and_then(|m| m.as_str()).unwrap_or("run error").to_string();
                    return BracketStep::Failed(VoiceError::NoAnswer {
                        endpoint: String::new(),
                        detail: msg,
                    });
                }
            }
            _ => {}
        }
        BracketStep::Continue
    }
}

fn emit_handoff(j: &mut JournalOut, trace: trace_events::TraceId, ep: &Endpoint, reason: String, status: HandoffStatus) {
    emit(j, trace, Payload::Op6(Op6Event::Handoff { destination: format!("agentd@{}", ep.addr()), reason, status }));
}

/// The classic tail's ending, verbatim in spirit: a finished run
/// classifies by its answer; a stream that ended with a completed
/// answer message but no finish still speaks it; anything else is
/// the loud "never finished" failure.
fn classify_bracket(ep: &Endpoint, j: &mut JournalOut, trace: trace_events::TraceId, bracket: &Bracket) -> Result<String, VoiceError> {
    if !bracket.finished && bracket.answer.is_none() {
        emit_handoff(j, trace, ep, format!("run {} never finished in the journal", bracket.run_id), HandoffStatus::Failed);
        return Err(VoiceError::NoAnswer {
            endpoint: ep.addr(),
            detail: format!("run {} never finished in the journal", bracket.run_id),
        });
    }
    if !bracket.confirmed {
        emit(j, trace, Payload::Op6(Op6Event::JournalDropped { count: 0, reason: "the server journals no user message for this run: the answer was correlated by the run id from the POST response, not by the request text".into() }));
    }
    match bracket.answer.clone().filter(|a| !a.trim().is_empty()) {
        Some(a) => {
            emit_handoff(j, trace, ep, "answer received".into(), HandoffStatus::Completed);
            Ok(a)
        }
        None => {
            emit_handoff(j, trace, ep, "the run produced no assistant message".into(), HandoffStatus::Failed);
            Err(VoiceError::NoAnswer { endpoint: ep.addr(), detail: "the run produced no assistant message".into() })
        }
    }
}

/// How many frames the tail buffers for a possible rebind. A run's
/// own segment of the journal is small; the cap only guards the
/// backlog replay a `since` hint can pull in.
const REBIND_BUFFER_CAP: usize = 16384;

fn buffer_has_finish(buffer: &[serde_json::Value], run_id: &str) -> bool {
    buffer.iter().rev().take(1024).any(|ev| {
        let payload = ev.get("payload").cloned().unwrap_or(serde_json::Value::Null);
        payload.get("type").and_then(|t| t.as_str()) == Some("RUN_FINISHED")
            && payload.get("runId").and_then(|r| r.as_str()) == Some(run_id)
    })
}

pub fn tail_answer_streaming(
    ep: &Endpoint,
    request: &str,
    run_id: &str,
    j: &mut JournalOut,
    timeout: Duration,
    after: u64,
    signals: &std::sync::mpsc::Receiver<PostSignal>,
    sink: &mut dyn ChunkSink,
) -> Result<String, VoiceError> {
    let trace = j.new_trace();
    let mut reader = tail_establish(ep, j, trace, timeout, after)?;
    // Poll-sized reads: the whole budget lives in `deadline`, not in
    // the socket, so POST signals are seen while the stream is quiet.
    reader.get_mut().set_read_timeout(Some(Duration::from_millis(200))).ok();
    let deadline = std::time::Instant::now() + timeout;
    let mut state = PostState::default();
    let mut bracket = Bracket::new(run_id, request);
    let mut buffer: Vec<serde_json::Value> = Vec::new();
    let mut data = String::new();

    'stream: loop {
        drain_post_signals(signals, &mut state);
        if let Some(e) = state.refusal.take() {
            return Err(e);
        }
        if let Some(adopted) = &state.adopted {
            if adopted != run_id && buffer_has_finish(&buffer, adopted) {
                break 'stream; // the adopted run is complete in the buffer: rebind
            }
        }
        if std::time::Instant::now() >= deadline {
            if bracket.finished {
                break 'stream;
            }
            if let Some(e) = state.err.take() {
                return Err(e);
            }
            emit_handoff(j, trace, ep, format!("no run finished within {} s", timeout.as_secs()), HandoffStatus::Failed);
            return Err(VoiceError::NoAnswer { endpoint: ep.addr(), detail: format!("timed out after {} s", timeout.as_secs()) });
        }
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => break 'stream,
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut || e.kind() == std::io::ErrorKind::WouldBlock => {
                continue;
            }
            Err(e) => return Err(VoiceError::AgentUnreachable { endpoint: ep.addr(), detail: format!("read: {e}") }),
        }
        let line = line.trim_end_matches(['\r', '\n']);
        if !line.is_empty() {
            if let Some(rest) = line.strip_prefix("data:") {
                if !data.is_empty() {
                    data.push('\n');
                }
                data.push_str(rest.strip_prefix(' ').unwrap_or(rest));
            }
            continue;
        }
        if data.is_empty() {
            continue;
        }
        let frame = std::mem::take(&mut data);
        let ev: serde_json::Value = match serde_json::from_str(&frame) {
            Ok(v) => v,
            Err(e) => {
                emit(j, trace, Payload::AgUi(AgUiEvent::RunError { message: format!("journal line rejected: {e}"), code: Some("bad_journal_line".into()), usage: vec![] }));
                continue;
            }
        };
        if buffer.len() < REBIND_BUFFER_CAP {
            buffer.push(ev.clone());
        }
        match bracket.apply(&ev, Some(&mut *sink)) {
            BracketStep::Finished => break 'stream,
            BracketStep::Failed(e) => {
                let detail = match &e {
                    VoiceError::NoAnswer { detail, .. } => detail.clone(),
                    other => other.to_string(),
                };
                emit_handoff(j, trace, ep, format!("run failed: {}", truncate(&detail, 120)), HandoffStatus::Failed);
                return Err(VoiceError::NoAnswer { endpoint: ep.addr(), detail });
            }
            BracketStep::Continue => {}
        }
    }

    // Our run completed in the stream: the classic ending.
    if bracket.finished || bracket.answer.is_some() {
        return classify_bracket(ep, j, trace, &bracket);
    }
    if let Some(e) = state.refusal.take() {
        return Err(e);
    }
    // The stream ended (or went quiet past the point of use) without
    // our run, and the POST may not have resolved yet: its word
    // decides what this ending means, so wait for it inside the
    // remaining budget before concluding anything.
    if state.adopted.is_none() && state.err.is_none() {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if !remaining.is_zero() {
            match signals.recv_timeout(remaining) {
                Ok(signal) => {
                    absorb_signal(&mut state, signal);
                    drain_post_signals(signals, &mut state);
                    if let Some(e) = state.refusal.take() {
                        return Err(e);
                    }
                }
                Err(_) => {}
            }
        }
    }
    if let Some(adopted) = state.adopted.clone() {
        if adopted != run_id {
            // Rebind: replay the buffered frames under the adopted
            // id. Chunks are not re-dispatched (a pre-CV3 agentd
            // sends none); the answer returns whole.
            let mut rebound = Bracket::new(&adopted, request);
            for ev in &buffer {
                match rebound.apply(ev, None) {
                    BracketStep::Finished => break,
                    BracketStep::Failed(e) => {
                        let detail = match &e {
                            VoiceError::NoAnswer { detail, .. } => detail.clone(),
                            other => other.to_string(),
                        };
                        emit_handoff(j, trace, ep, format!("run failed: {}", truncate(&detail, 120)), HandoffStatus::Failed);
                        return Err(VoiceError::NoAnswer { endpoint: ep.addr(), detail });
                    }
                    BracketStep::Continue => {}
                }
            }
            if rebound.finished || rebound.answer.is_some() {
                return classify_bracket(ep, j, trace, &rebound);
            }
            if rebound.saw_start {
                // Its bracket opened in a stream that then died:
                // fail loudly — a bracket whose state was in flux is
                // never re-entered (the hardening contract).
                emit_handoff(j, trace, ep, format!("run {adopted} never finished in the journal"), HandoffStatus::Failed);
                return Err(VoiceError::NoAnswer {
                    endpoint: ep.addr(),
                    detail: format!("run {adopted} never finished in the journal"),
                });
            }
            // The adopted run never appeared in this stream: the
            // classic tail on its id is the last resort (a live
            // agentd whose run started after our stream was read,
            // or a rebind buffer that overflowed).
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                emit_handoff(j, trace, ep, format!("no run finished within {} s", timeout.as_secs()), HandoffStatus::Failed);
                return Err(VoiceError::NoAnswer { endpoint: ep.addr(), detail: format!("timed out after {} s", timeout.as_secs()) });
            }
            return tail_answer(ep, request, &adopted, j, remaining, after);
        }
        // The POST adopted our minted id, but its frames never
        // completed here: the stream died under a live run.
        emit_handoff(j, trace, ep, format!("run {run_id} never finished in the journal"), HandoffStatus::Failed);
        return Err(VoiceError::NoAnswer {
            endpoint: ep.addr(),
            detail: format!("run {run_id} never finished in the journal"),
        });
    }
    if let Some(e) = state.err.take() {
        return Err(e);
    }
    emit_handoff(j, trace, ep, format!("run {run_id} never finished in the journal"), HandoffStatus::Failed);
    Err(VoiceError::NoAnswer {
        endpoint: ep.addr(),
        detail: format!("run {run_id} never finished in the journal"),
    })
}
