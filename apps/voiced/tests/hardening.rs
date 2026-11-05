//! Regression tests for the read-path hardening (the CV1 isolation
//! incident: a spoken answer lost while agentd completed the run
//! correctly). The fake agentd here can fail the way the real one was
//! failed BY the old client — and the way transports fail on their
//! own: slow synchronous POSTs, connects closed with no reply, bodies
//! cut short, stream heads that never arrive, and streams that die
//! mid-run. The contract under test:
//!   - a run slower than the old baked-in 10 s POST read timeout, but
//!     inside the caller's answer budget, succeeds;
//!   - transport failures before anything is established (empty reply,
//!     short body, head EOF) are retried a bounded number of times;
//!   - a refusal is the server's word and is never retried;
//!   - a stream that dies mid-run fails the turn loudly and is never
//!     silently re-tailed — the run_id bracket is not re-entered.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use voiced::capture::JournalOut;
use voiced::config::Config;
use voiced::error::VoiceError;
use voiced::{run, Input, Request};
use voice_bridge::wav::write_pcm;

fn tmp_dir(tag: &str) -> PathBuf {
    let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().subsec_nanos();
    let p = std::env::temp_dir().join(format!("voiced-hardening-{tag}-{}-{nonce}", std::process::id()));
    std::fs::create_dir_all(&p).expect("create temp dir");
    p
}

fn write_tone_wav(path: &Path) {
    write_pcm(path, 16_000, 2, 1, &vec![0u8; 16_000]).expect("write wav");
}

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

fn new_journal() -> (JournalOut, Sink) {
    let sink = Sink(Arc::new(Mutex::new(Vec::new())));
    (JournalOut::with_service(Box::new(sink.clone()), "voiced".into()), sink)
}

fn journal_text(sink: &Sink) -> String {
    String::from_utf8(sink.0.lock().unwrap().clone()).expect("journal is utf8")
}

fn base_config(dir: &Path, addr: &str) -> Config {
    Config {
        stt: voiced::config::SttEngine::Stub { text: "what time is it".into() },
        tts: voiced::config::TtsEngine::None,
        agentd: format!("http://{addr}"),
        play_cmd: None,
        quiet: true,
        lock: dir.join("mic.lock").display().to_string(),
        mic_state_file: dir.join("mic.state").display().to_string(),
        language: None,
        voice: None,
        clean_cmd: None,
        ack_clips: Vec::new(),
        ack_delay_ms: voiced::config::DEFAULT_ACK_DELAY_MS,
        smart_turn_cmd: None,
    }
}

fn frame(payload: serde_json::Value) -> String {
    serde_json::to_string(&payload).unwrap()
}

/// A complete run answering `answer`, journal-shaped the way agentd
/// journals it (the answer message carries the run id as messageId).
fn run_frames(answer: &str) -> Vec<String> {
    let mut seq = 0u64;
    let mut next = |v: serde_json::Value| {
        let mut v = v;
        v.as_object_mut().unwrap().insert("seq".into(), serde_json::json!(seq));
        seq += 1;
        frame(v)
    };
    vec![
        next(serde_json::json!({"payload": {"kind": "ag_ui", "type": "RUN_STARTED", "threadId": "t1", "runId": "r1", "input": {"text": "what time is it"}}})),
        next(serde_json::json!({"payload": {"kind": "ag_ui", "type": "TEXT_MESSAGE_START", "messageId": "r1", "role": "assistant"}})),
        next(serde_json::json!({"payload": {"kind": "ag_ui", "type": "TEXT_MESSAGE_CONTENT", "messageId": "r1", "delta": answer}})),
        next(serde_json::json!({"payload": {"kind": "ag_ui", "type": "TEXT_MESSAGE_END", "messageId": "r1"}})),
        next(serde_json::json!({"payload": {"kind": "ag_ui", "type": "RUN_FINISHED", "threadId": "t1", "runId": "r1"}})),
    ]
}

#[derive(Default, Clone)]
struct Knobs {
    /// The POST handler waits this long before writing anything,
    /// exactly like agentd running the loop synchronously.
    post_delay: Duration,
    /// The first K POST accepts are closed with no reply at all.
    post_close_first: usize,
    /// The first K POST accepts get a head promising the full body but
    /// only half of it, then a close.
    post_short_first: usize,
    /// When non-200, the POST is answered with this status instead.
    post_status: u16,
    /// The first K /events accepts are closed before the head.
    events_close_first: usize,
    /// When non-200, /events is answered with this status instead.
    events_status: u16,
    /// /events serves only the first N frames, then closes mid-run.
    events_cut_after: Option<usize>,
}

struct Fake {
    addr: String,
    posts: Arc<AtomicUsize>,
    events: Arc<AtomicUsize>,
}

fn fake(knobs: Knobs, frames: Vec<String>) -> Fake {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake");
    let addr = listener.local_addr().expect("addr").to_string();
    let posts = Arc::new(AtomicUsize::new(0));
    let events = Arc::new(AtomicUsize::new(0));
    let (pc, ec) = (posts.clone(), events.clone());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut buf = [0u8; 8192];
            let n = stream.read(&mut buf).unwrap_or(0);
            let head = String::from_utf8_lossy(&buf[..n]).to_string();
            if head.starts_with("GET /health") {
                let body = "{\"ok\":true,\"events\":5}";
                let _ = stream.write_all(format!("HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len()).as_bytes());
                continue;
            }
            if head.starts_with("POST") {
                let k = pc.fetch_add(1, Ordering::SeqCst) + 1;
                if knobs.post_status != 0 && knobs.post_status != 200 {
                    let body = "{\"error\":\"brain down\"}";
                    let _ = stream.write_all(format!("HTTP/1.1 {} Nope\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", knobs.post_status, body.len()).as_bytes());
                    continue;
                }
                if k <= knobs.post_close_first {
                    drop(stream);
                    continue;
                }
                if !knobs.post_delay.is_zero() {
                    std::thread::sleep(knobs.post_delay);
                }
                let body = "{\"run_id\":\"r1\",\"answer\":null,\"error\":null,\"brain\":\"fake\"}";
                if k <= knobs.post_close_first + knobs.post_short_first {
                    let half = &body[..body.len() / 2];
                    let _ = stream.write_all(format!("HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{half}", body.len()).as_bytes());
                    let _ = stream.flush();
                    drop(stream);
                    continue;
                }
                let _ = stream.write_all(format!("HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len()).as_bytes());
                continue;
            }
            if head.starts_with("GET /events") {
                let k = ec.fetch_add(1, Ordering::SeqCst) + 1;
                if knobs.events_status != 0 && knobs.events_status != 200 {
                    let body = "no stream today";
                    let _ = stream.write_all(format!("HTTP/1.1 {} Nope\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", knobs.events_status, body.len()).as_bytes());
                    continue;
                }
                if k <= knobs.events_close_first {
                    drop(stream);
                    continue;
                }
                let keep = knobs.events_cut_after.unwrap_or(frames.len());
                let sse = frames.iter().take(keep).map(|f| format!("data: {f}\n\n")).collect::<String>();
                let _ = stream.write_all(format!("HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n\r\n{sse}").as_bytes());
                let _ = stream.shutdown(std::net::Shutdown::Both);
                continue;
            }
        }
    });
    Fake { addr, posts, events }
}

fn drive(tag: &str, knobs: Knobs, frames: Vec<String>) -> (Result<voiced::Outcome, VoiceError>, Fake, Sink, PathBuf) {
    let dir = tmp_dir(tag);
    let wav = dir.join("ask.wav");
    write_tone_wav(&wav);
    let f = fake(knobs, frames);
    let cfg = base_config(&dir, &f.addr);
    let (mut journal, sink) = new_journal();
    let req = Request {
        input: Input::Wav { path: wav.display().to_string() },
        out_wav: dir.join("a.wav").display().to_string(),
        answer_timeout: Duration::from_secs(30),
        session: None,
        follow_up: false,
    };
    let res = run(&cfg, &req, &mut journal);
    (res, f, sink, dir)
}

/// The CV1 incident itself: agentd answers POST /request only when the
/// run finishes. A run slower than the client's old 10 s read timeout
/// lost its answer while the server completed it. Inside the caller's
/// answer budget, the turn must now succeed.
#[test]
fn a_run_slower_than_the_old_post_timeout_still_answers() {
    let t0 = Instant::now();
    let (res, f, _sink, _dir) = drive("slowpost", Knobs { post_delay: Duration::from_secs(11), ..Default::default() }, run_frames("It is noon."));
    let outcome = res.expect("an 11 s run inside a 30 s budget must succeed");
    assert_eq!(outcome.answer, "It is noon.");
    assert!(t0.elapsed() >= Duration::from_secs(11), "the client really waited for the run");
    assert_eq!(f.posts.load(Ordering::SeqCst), 1, "a slow healthy run is not retried — it is waited for");
}

/// An accept closed with no reply is a transport failure at connect
/// time: bounded retries, and the turn succeeds on the third attempt.
#[test]
fn post_empty_replies_are_retried_and_the_turn_succeeds() {
    let (res, f, sink, _dir) = drive("postclose", Knobs { post_close_first: 2, ..Default::default() }, run_frames("It is noon."));
    let outcome = res.expect("empty replies at connect time must be retried");
    assert_eq!(outcome.answer, "It is noon.");
    assert_eq!(f.posts.load(Ordering::SeqCst), 3, "two dead connects, one good one");
    assert!(journal_text(&sink).contains("retrying"), "the recovery is journalled, not silent");
}

/// A response whose body is cut short of its content-length is not a
/// response: it is retried like the empty reply, and the retried
/// attempt's complete body is the one that counts.
#[test]
fn a_post_body_cut_short_is_retried_and_the_turn_succeeds() {
    let (res, f, _sink, _dir) = drive("postshort", Knobs { post_short_first: 1, ..Default::default() }, run_frames("It is noon."));
    let outcome = res.expect("a short body must be retried");
    assert_eq!(outcome.answer, "It is noon.");
    assert_eq!(f.posts.load(Ordering::SeqCst), 2, "one short body, one whole one");
}

/// A refusal is the server speaking, not a transport failure: the turn
/// fails as refused on the first attempt, with no retry.
#[test]
fn a_post_refusal_is_not_retried() {
    let (res, f, _sink, _dir) = drive("postrefused", Knobs { post_status: 502, ..Default::default() }, run_frames("It is noon."));
    let err = res.expect_err("a refused request must fail the turn");
    assert!(matches!(err, VoiceError::AgentRefused { status: 502, .. }), "got {err:?}");
    assert_eq!(f.posts.load(Ordering::SeqCst), 1, "a refusal is never retried");
}

/// A tail whose peer hangs up before the response head was never
/// established: reconnect (same run id, same bracket), and the turn
/// succeeds on the third connect.
#[test]
fn a_tail_that_never_establishes_is_reconnected() {
    let (res, f, sink, _dir) = drive("eventsclose", Knobs { events_close_first: 2, ..Default::default() }, run_frames("It is noon."));
    let outcome = res.expect("a tail that never establishes must be re-connected");
    assert_eq!(outcome.answer, "It is noon.");
    assert_eq!(f.events.load(Ordering::SeqCst), 3, "two dead connects, one good stream");
    assert!(journal_text(&sink).contains("retrying"), "the recovery is journalled, not silent");
}

/// A non-200 /events answer is a refusal, not a stream: loud, unretried.
#[test]
fn an_events_refusal_is_not_retried() {
    let (res, f, _sink, _dir) = drive("eventsrefused", Knobs { events_status: 500, ..Default::default() }, run_frames("It is noon."));
    let err = res.expect_err("a refused stream must fail the turn");
    assert!(matches!(err, VoiceError::AgentRefused { status: 500, .. }), "got {err:?}");
    assert_eq!(f.events.load(Ordering::SeqCst), 1, "a refusal is never retried");
}

/// The other side of the bargain: a stream that establishes and then
/// dies mid-run — our run started, its answer half-arrived, no finish —
/// fails the turn loudly and is NEVER re-tailed. The one /events
/// connection is the proof: no silent second chance at a bracket whose
/// state was already in flux.
#[test]
fn a_stream_that_dies_mid_run_fails_loudly_and_never_retails() {
    // RUN_STARTED + TEXT_MESSAGE_START + a partial CONTENT, then EOF.
    let (res, f, _sink, dir) = drive("midstream", Knobs { events_cut_after: Some(3), ..Default::default() }, run_frames("It is noon."));
    let err = res.expect_err("a run that never finished has no answer to speak");
    assert!(matches!(err, VoiceError::NoAnswer { .. }), "got {err:?}");
    assert_eq!(f.events.load(Ordering::SeqCst), 1, "an established stream is never re-tailed");
    assert!(!dir.join("a.wav").exists(), "nothing is spoken for a dead run");
}
