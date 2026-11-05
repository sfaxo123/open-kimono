//! Host tests for voiced (PLAN A4): a full voice turn driven by WAV fixtures,
//! a fake agentd, and command adapters standing in for the phone's engines.
//! Every negative is named: the failure must say what broke, and the journal
//! must show it.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use trace_events::{from_json_line, Event, Payload};
use voiced::capture::JournalOut;
use voiced::config::Config;
use voiced::error::VoiceError;
use voiced::{run, Input, Request};
use voice_bridge::wav::{read_pcm, write_pcm};

// ---------------------------------------------------------------- fixtures

fn tmp_dir(tag: &str) -> PathBuf {
    let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().subsec_nanos();
    let p = std::env::temp_dir().join(format!("voiced-test-{tag}-{}-{nonce}", std::process::id()));
    std::fs::create_dir_all(&p).expect("create temp dir");
    p
}

/// A 1.2 s 16 kHz mono tone: a stand-in for a spoken utterance. The STT
/// adapters in these tests are scripts, so the content only has to be audio.
fn write_tone_wav(path: &Path, secs: f32) {
    let rate = 16_000u32;
    let n = (rate as f32 * secs) as usize;
    let mut bytes = Vec::with_capacity(n * 2);
    for i in 0..n {
        let t = i as f32 / rate as f32;
        let env = (std::f32::consts::PI * 2.0 * 2.2 * t).sin().abs();
        let s = (std::f32::consts::PI * 2.0 * 440.0 * t).sin() * 9000.0 * env;
        bytes.extend_from_slice(&(s as i16).to_le_bytes());
    }
    write_pcm(path, rate, 2, 1, &bytes).expect("write tone wav");
}

/// A WAV of pure zeros — the silence fixture.
fn write_silence_wav(path: &Path) {
    write_pcm(path, 16_000, 2, 1, &vec![0u8; 16_000]).expect("write silence wav");
}

fn script(dir: &Path, name: &str, body: &str) -> String {
    let p = dir.join(name);
    std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).expect("write script");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }
    p.display().to_string()
}

// ------------------------------------------------------------ journal sink

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

fn events(sink: &Sink) -> Vec<Event> {
    let bytes = sink.0.lock().unwrap().clone();
    let text = String::from_utf8(bytes).expect("journal is utf8");
    text.lines().filter(|l| !l.trim().is_empty()).map(|l| from_json_line(l).unwrap_or_else(|e| panic!("journal line rejected: {e}\n{l}"))).collect()
}

fn types(sink: &Sink) -> Vec<String> {
    events(sink)
        .iter()
        .map(|e| match &e.payload {
            Payload::Op6(o) => serde_json::to_value(o).unwrap()["type"].as_str().unwrap().to_string(),
            Payload::AgUi(a) => serde_json::to_value(a).unwrap()["type"].as_str().unwrap().to_string(),
            Payload::GenAi(g) => serde_json::to_value(g).unwrap()["event"].as_str().unwrap().to_string(),
        })
        .collect()
}

// ------------------------------------------------------------ fake agentd

/// Answers `GET /health` (journal length), `POST /request`, and `GET /events`
/// (SSE, honouring `last-event-id` the way agentd does). `connections` counts
/// accepts and `last_event_id` records what the client asked to resume after.
fn fake_agentd(frames: Vec<String>, status: u16, health_events: u64) -> (String, Arc<AtomicUsize>, Arc<Mutex<String>>, Arc<Mutex<Vec<String>>>, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake agentd");
    let addr = listener.local_addr().expect("addr").to_string();
    let connections = Arc::new(AtomicUsize::new(0));
    let counter = connections.clone();
    let seen_id = Arc::new(Mutex::new(String::new()));
    let seen = seen_id.clone();
    let posts = Arc::new(Mutex::new(Vec::<String>::new()));
    let posted = posts.clone();
    let handle = std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            counter.fetch_add(1, Ordering::SeqCst);
            let mut buf = [0u8; 4096];
            let n = stream.read(&mut buf).unwrap_or(0);
            let head = String::from_utf8_lossy(&buf[..n]).to_string();
            if head.starts_with("POST") {
                if let Some((_, body)) = head.split_once("\r\n\r\n") {
                    posted.lock().unwrap().push(body.to_string());
                }
                let body = format!("{{\"run_id\":\"{}\",\"answer\":null,\"error\":null,\"brain\":\"fake\"}}", last_run_id(&frames));
                let _ = stream.write_all(format!("HTTP/1.1 {status} Accepted\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len()).as_bytes());
                let _ = stream.flush();
                continue;
            }
            if head.starts_with("GET /health") {
                let body = format!("{{\"ok\":true,\"events\":{health_events}}}");
                let _ = stream.write_all(format!("HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len()).as_bytes());
                let _ = stream.flush();
                continue;
            }
            let resume: Option<u64> = head
                .lines()
                .find(|l| l.to_ascii_lowercase().starts_with("last-event-id:"))
                .and_then(|l| l.split_once(':'))
                .and_then(|(_, v)| v.trim().parse::<u64>().ok());
            *seen.lock().unwrap() = resume.map(|r| r.to_string()).unwrap_or_default();
            // Frames carry a `seq`; a resuming client only gets the newer ones.
            let keep: Vec<&String> = frames
                .iter()
                .filter(|f| {
                    let seq = serde_json::from_str::<serde_json::Value>(f).ok().and_then(|v| v.get("seq").and_then(|s| s.as_u64())).unwrap_or(0);
                    resume.map(|r| seq > r).unwrap_or(true)
                })
                .collect();
            let sse = keep.iter().map(|f| format!("data: {f}\n\n")).collect::<String>();
            let _ = stream.write_all(format!("HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n\r\n{sse}").as_bytes());
            let _ = stream.flush();
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
    });
    (addr, connections, seen_id, posts, handle)
}

/// The run id of the last RUN_STARTED in a frame list — the run a POST
/// would have started, and the id the real server names in its response.
fn last_run_id(frames: &[String]) -> String {
    frames
        .iter()
        .rev()
        .find_map(|f| {
            let v: serde_json::Value = serde_json::from_str(f).ok()?;
            let p = v.get("payload")?;
            if p.get("type").and_then(|t| t.as_str()) == Some("RUN_STARTED") {
                p.get("runId").and_then(|r| r.as_str()).map(str::to_string)
            } else {
                None
            }
        })
        .unwrap_or_else(|| "fake-1".into())
}

fn frame(payload: serde_json::Value) -> String {
    serde_json::to_string(&payload).unwrap()
}

/// The journal frames a run that answers `answer` to `ask`, numbered from `from`.
fn run_frames(ask: &str, answer: &str, from: u64) -> Vec<String> {
    let mut seq = from;
    let mut next = |v: serde_json::Value| {
        let mut v = v;
        v.as_object_mut().unwrap().insert("seq".into(), serde_json::json!(seq));
        seq += 1;
        frame(v)
    };
    vec![
        next(serde_json::json!({"payload": {"kind": "ag_ui", "type": "RUN_STARTED", "threadId": "t1", "runId": "r1"}})),
        next(serde_json::json!({"payload": {"kind": "ag_ui", "type": "TEXT_MESSAGE_START", "messageId": "m1", "role": "user"}})),
        next(serde_json::json!({"payload": {"kind": "ag_ui", "type": "TEXT_MESSAGE_CONTENT", "messageId": "m1", "delta": ask}})),
        next(serde_json::json!({"payload": {"kind": "ag_ui", "type": "TEXT_MESSAGE_END", "messageId": "m1"}})),
        next(serde_json::json!({"payload": {"kind": "ag_ui", "type": "TEXT_MESSAGE_START", "messageId": "m2", "role": "assistant"}})),
        next(serde_json::json!({"payload": {"kind": "ag_ui", "type": "TEXT_MESSAGE_CONTENT", "messageId": "m2", "delta": answer}})),
        next(serde_json::json!({"payload": {"kind": "ag_ui", "type": "TEXT_MESSAGE_END", "messageId": "m2"}})),
        next(serde_json::json!({"payload": {"kind": "ag_ui", "type": "RUN_FINISHED", "threadId": "t1", "runId": "r1"}})),
    ]
}

/// The shape agentd journals today: no user message at all, so correlation
/// has to come from the resume position rather than the request text.
fn agentd_like_frames(answer: &str, from: u64) -> Vec<String> {
    let mut seq = from;
    let mut next = |v: serde_json::Value| {
        let mut v = v;
        v.as_object_mut().unwrap().insert("seq".into(), serde_json::json!(seq));
        seq += 1;
        frame(v)
    };
    vec![
        next(serde_json::json!({"payload": {"kind": "ag_ui", "type": "RUN_STARTED", "threadId": "phone", "runId": "r"}})),
        next(serde_json::json!({"payload": {"kind": "ag_ui", "type": "TEXT_MESSAGE_START", "messageId": "a", "role": "assistant"}})),
        next(serde_json::json!({"payload": {"kind": "ag_ui", "type": "TEXT_MESSAGE_CONTENT", "messageId": "a", "delta": answer}})),
        next(serde_json::json!({"payload": {"kind": "ag_ui", "type": "TEXT_MESSAGE_END", "messageId": "a"}})),
        next(serde_json::json!({"payload": {"kind": "ag_ui", "type": "RUN_FINISHED", "threadId": "phone", "runId": "r"}})),
    ]
}

fn base_config(dir: &Path) -> Config {
    Config {
        stt: voiced::config::SttEngine::Stub { text: String::new() },
        tts: voiced::config::TtsEngine::None,
        agentd: "http://127.0.0.1:1".into(),
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

// ------------------------------------------------------------------ tests

#[test]
fn a_full_voice_turn_hears_asks_speaks_and_journals_every_step() {
    let dir = tmp_dir("turn");
    let wav = dir.join("ask.wav");
    write_tone_wav(&wav, 1.2);
    let out_wav = dir.join("answer.wav");

    let stt = script(&dir, "stt.sh", "cat > /dev/null; printf 'remind me to drink water\\n'");
    // The TTS adapter must produce a real WAV, not just exit 0.
    let tts = script(&dir, "tts.sh", "cat > /dev/null; cp \"$VOICED_TTS_SRC\" \"$1\"");
    std::env::set_var("VOICED_TTS_SRC", wav.display().to_string());

    let (addr, connections, _seen, _posts, _h) = fake_agentd(run_frames("remind me to drink water", "Set: water every 2 minutes.", 0), 202, 0);

    let mut cfg = base_config(&dir);
    cfg.stt = voiced::config::SttEngine::Cmd { cmd: stt };
    cfg.tts = voiced::config::TtsEngine::Cmd { cmd: tts };
    cfg.agentd = format!("http://{addr}");

    let (mut journal, sink) = new_journal();
    let req = Request { input: Input::Wav { path: wav.display().to_string() }, out_wav: out_wav.display().to_string(), answer_timeout: Duration::from_secs(5), session: None, follow_up: false };
    let outcome = run(&cfg, &req, &mut journal).expect("the voice turn should succeed");

    assert_eq!(outcome.cleaned, "remind me to drink water");
    assert_eq!(outcome.answer, "Set: water every 2 minutes.");
    assert_eq!(connections.load(Ordering::SeqCst), 3, "one health probe, one POST and one GET");
    read_pcm(&out_wav).expect("the answer WAV is real audio");

    let seen = types(&sink);
    for expected in ["mic_started", "mic_frame", "mic_stopped", "stt_final", "power_change", "wake_lock_acquired", "handoff", "wake_lock_released"] {
        assert!(seen.iter().any(|t| t == expected), "journal is missing {expected}: {seen:?}");
    }
    // The mic closed with a reason, and the answer was spoken, not invented.
    let evs = events(&sink);
    let closed = evs.iter().find_map(|e| match &e.payload {
        Payload::Op6(trace_events::Op6Event::MicStopped { reason }) => Some(reason.clone()),
        _ => None,
    });
    assert_eq!(closed.as_deref(), Some("file input consumed to the end"));
    assert!(evs.iter().all(|e| e.service.as_deref() == Some("voiced")), "every record is stamped voiced");
    let seqs: Vec<u64> = evs.iter().map(|e| e.seq).collect();
    assert!(seqs.windows(2).all(|w| w[1] == w[0] + 1), "journal seq is gapless: {seqs:?}");
}

#[test]
fn negative_a_failing_stt_engine_is_reported_and_no_request_is_sent() {
    let dir = tmp_dir("sttfail");
    let wav = dir.join("ask.wav");
    write_tone_wav(&wav, 0.5);
    let stt = script(&dir, "stt.sh", "cat > /dev/null; echo 'moonshine export absent' >&2; exit 75");
    let (addr, connections, _seen, _posts, _h) = fake_agentd(run_frames("x", "y", 0), 202, 0);

    let mut cfg = base_config(&dir);
    cfg.stt = voiced::config::SttEngine::Cmd { cmd: stt };
    cfg.agentd = format!("http://{addr}");

    let (mut journal, sink) = new_journal();
    let req = Request { input: Input::Wav { path: wav.display().to_string() }, out_wav: dir.join("a.wav").display().to_string(), answer_timeout: Duration::from_secs(5), session: None, follow_up: false };
    let err = run(&cfg, &req, &mut journal).expect_err("a dead STT engine must fail the turn");
    assert!(matches!(err, VoiceError::SttUnavailable { .. }), "got {err:?}");
    assert!(err.to_string().contains("moonshine export absent"), "the engine's own words must survive: {err}");
    assert_eq!(connections.load(Ordering::SeqCst), 0, "no request may be sent on a failed transcript");

    let evs = events(&sink);
    let code = evs.iter().find_map(|e| match &e.payload {
        Payload::AgUi(trace_events::AgUiEvent::RunError { code, .. }) => code.clone(),
        _ => None,
    });
    assert_eq!(code.as_deref(), Some("stt_failed"));
    let failed = evs.iter().any(|e| matches!(&e.payload, Payload::Op6(trace_events::Op6Event::Handoff { status: trace_events::HandoffStatus::Failed, .. })));
    assert!(failed, "the hand-off must be closed as failed, not left open");
}

#[test]
fn negative_silence_produces_no_request_and_says_so() {
    let dir = tmp_dir("silence");
    let wav = dir.join("quiet.wav");
    write_silence_wav(&wav);
    let stt = script(&dir, "stt.sh", "cat > /dev/null; exit 3");
    let (addr, connections, _seen, _posts, _h) = fake_agentd(run_frames("x", "y", 0), 202, 0);

    let mut cfg = base_config(&dir);
    cfg.stt = voiced::config::SttEngine::Cmd { cmd: stt };
    cfg.agentd = format!("http://{addr}");

    let (mut journal, sink) = new_journal();
    let req = Request { input: Input::Wav { path: wav.display().to_string() }, out_wav: dir.join("a.wav").display().to_string(), answer_timeout: Duration::from_secs(5), session: None, follow_up: false };
    let err = run(&cfg, &req, &mut journal).expect_err("silence is not a request");
    assert!(matches!(err, VoiceError::EmptyTranscript { .. }), "got {err:?}");
    assert_eq!(connections.load(Ordering::SeqCst), 0, "silence must not reach agentd");
    assert!(types(&sink).iter().any(|t| t == "mic_stopped"), "the mic still closed honestly");
}

#[test]
fn negative_an_unreachable_agentd_names_the_address_it_tried() {
    let dir = tmp_dir("noreply");
    let wav = dir.join("ask.wav");
    write_tone_wav(&wav, 0.4);

    // A port nobody is listening on.
    let free = TcpListener::bind("127.0.0.1:0").expect("bind").local_addr().expect("addr").to_string();

    let mut cfg = base_config(&dir);
    cfg.stt = voiced::config::SttEngine::Stub { text: "what time is it".into() };
    cfg.agentd = format!("http://{free}");

    let (mut journal, sink) = new_journal();
    let req = Request { input: Input::Wav { path: wav.display().to_string() }, out_wav: dir.join("a.wav").display().to_string(), answer_timeout: Duration::from_secs(5), session: None, follow_up: false };
    let err = run(&cfg, &req, &mut journal).expect_err("an unreachable agentd must fail the turn");
    assert!(matches!(err, VoiceError::AgentUnreachable { .. }), "got {err:?}");
    assert!(err.to_string().contains(&free), "the address must be named: {err}");
    assert!(!dir.join("a.wav").exists(), "nothing is spoken when there is no answer");
    assert!(events(&sink).iter().any(|e| matches!(&e.payload, Payload::AgUi(trace_events::AgUiEvent::RunError { .. }))), "the failure is journalled");
}

#[test]
fn negative_a_run_that_errors_is_not_spoken() {
    let dir = tmp_dir("runerror");
    let wav = dir.join("ask.wav");
    write_tone_wav(&wav, 0.4);
    let frames = vec![
        frame(serde_json::json!({"payload": {"kind": "ag_ui", "type": "RUN_STARTED", "threadId": "t", "runId": "r"}})),
        frame(serde_json::json!({"payload": {"kind": "ag_ui", "type": "TEXT_MESSAGE_START", "messageId": "m1", "role": "user"}})),
        frame(serde_json::json!({"payload": {"kind": "ag_ui", "type": "TEXT_MESSAGE_CONTENT", "messageId": "m1", "delta": "open the pod bay doors"}})),
        frame(serde_json::json!({"payload": {"kind": "ag_ui", "type": "TEXT_MESSAGE_END", "messageId": "m1"}})),
        frame(serde_json::json!({"payload": {"kind": "ag_ui", "type": "RUN_ERROR", "message": "gateway timeout", "code": "upstream_timeout"}})),
    ];
    let (addr, _c, _seen, _posts, _h) = fake_agentd(frames, 202, 0);

    let mut cfg = base_config(&dir);
    cfg.stt = voiced::config::SttEngine::Stub { text: "open the pod bay doors".into() };
    cfg.agentd = format!("http://{addr}");

    let (mut journal, _sink) = new_journal();
    let out = dir.join("a.wav");
    let req = Request { input: Input::Wav { path: wav.display().to_string() }, out_wav: out.display().to_string(), answer_timeout: Duration::from_secs(5), session: None, follow_up: false };
    let err = run(&cfg, &req, &mut journal).expect_err("a failed run has no answer to speak");
    assert!(matches!(err, VoiceError::NoAnswer { .. }), "got {err:?}");
    assert!(err.to_string().contains("gateway timeout"), "the run's own message must survive: {err}");
    assert!(!out.exists(), "no audio is produced for a failed run");
}

#[test]
fn negative_a_tts_engine_that_writes_a_truncated_wav_is_not_a_success() {
    let dir = tmp_dir("badwav");
    let wav = dir.join("ask.wav");
    write_tone_wav(&wav, 0.4);
    let out = dir.join("a.wav");
    std::fs::write(&out, b"RIFF\x10\x00\x00\x00WAVEjunk").expect("write truncated wav");
    let tts = script(&dir, "tts.sh", "cat > /dev/null"); // exits 0, writes nothing new
    let (addr, _c, _seen, _posts, _h) = fake_agentd(run_frames("hello", "hi there", 0), 202, 0);

    let mut cfg = base_config(&dir);
    cfg.stt = voiced::config::SttEngine::Stub { text: "hello".into() };
    cfg.tts = voiced::config::TtsEngine::Cmd { cmd: tts };
    cfg.agentd = format!("http://{addr}");

    let (mut journal, _sink) = new_journal();
    let req = Request { input: Input::Wav { path: wav.display().to_string() }, out_wav: out.display().to_string(), answer_timeout: Duration::from_secs(5), session: None, follow_up: false };
    let err = run(&cfg, &req, &mut journal).expect_err("a truncated WAV is not audio");
    assert!(matches!(err, VoiceError::TtsFailed { .. }), "got {err:?}");
    assert!(err.to_string().contains("not usable audio"), "the reason must be specific: {err}");
}

#[test]
fn negative_the_microphone_is_owned_by_one_service() {
    let dir = tmp_dir("micbusy");
    let wav = dir.join("ask.wav");
    write_tone_wav(&wav, 0.3);
    let lock = dir.join("mic.lock");

    let held = voiced::capture::acquire_mic(&lock, "voiced pid 1").expect("first acquire");
    let mut cfg = base_config(&dir);
    cfg.stt = voiced::config::SttEngine::Stub { text: "hello".into() };

    let (mut journal, _sink) = new_journal();
    let req = Request { input: Input::Wav { path: wav.display().to_string() }, out_wav: dir.join("a.wav").display().to_string(), answer_timeout: Duration::from_secs(5), session: None, follow_up: false };
    let err = run(&cfg, &req, &mut journal).expect_err("a second holder must be refused");
    assert!(matches!(err, VoiceError::MicBusy { .. }), "got {err:?}");
    assert!(err.to_string().contains("voiced pid 1"), "the holder must be named: {err}");

    drop(held);
    // After the holder releases, the next turn is allowed.
    assert!(voiced::capture::acquire_mic(&lock, "voiced pid 2").is_ok(), "the lock must be free after drop");
}

#[test]
fn quiet_mode_writes_the_answer_but_makes_no_sound() {
    let dir = tmp_dir("quiet");
    let wav = dir.join("ask.wav");
    write_tone_wav(&wav, 0.4);
    let out = dir.join("a.wav");
    let marker = dir.join("played");
    let tts = script(&dir, "tts.sh", "cat > /dev/null; cp \"$VOICED_TTS_SRC\" \"$1\"");
    let play = script(&dir, "play.sh", "touch \"$VOICED_PLAY_MARKER\"");
    std::env::set_var("VOICED_PLAY_MARKER", marker.display().to_string());
    std::env::set_var("VOICED_TTS_SRC", wav.display().to_string());
    let (addr, _c, _seen, _posts, _h) = fake_agentd(run_frames("hello", "hi there", 0), 202, 0);

    let mut cfg = base_config(&dir);
    cfg.stt = voiced::config::SttEngine::Stub { text: "hello".into() };
    cfg.tts = voiced::config::TtsEngine::Cmd { cmd: tts };
    cfg.play_cmd = Some(play);
    cfg.quiet = true;
    cfg.agentd = format!("http://{addr}");

    let (mut journal, sink) = new_journal();
    let req = Request { input: Input::Wav { path: wav.display().to_string() }, out_wav: out.display().to_string(), answer_timeout: Duration::from_secs(5), session: None, follow_up: false };
    run(&cfg, &req, &mut journal).expect("a quiet turn still completes");
    read_pcm(&out).expect("the answer WAV exists");
    assert!(!marker.exists(), "quiet mode must not run the play command");
    assert!(types(&sink).iter().any(|t| t == "handoff"), "the refusal is journalled");
}

#[test]
fn negative_config_refuses_an_engine_spec_it_cannot_use() {
    let e = Config::parse("stt = wyoming 127.0.0.1").expect_err("host without a port");
    assert!(e.to_string().contains("host:port"), "{e}");
    let e = Config::parse("tts = whisper").expect_err("an invented engine");
    assert!(e.to_string().contains("tts must be"), "{e}");
    let e = Config::parse("agentd = https://example.invalid").expect_err("https is out of scope");
    assert!(e.to_string().contains("http://"), "{e}");
    let e = Config::parse("microphone = yes").expect_err("an invented key");
    assert!(e.to_string().contains("unknown key"), "{e}");
    let ok = Config::parse("stt = wyoming 127.0.0.1:10300\ntts = cmd /usr/local/bin/say.sh\nquiet = true").expect("a valid config");
    assert_eq!(ok.stt, voiced::config::SttEngine::Wyoming { destination: "127.0.0.1:10300".into() });
    assert!(ok.quiet);
}

#[test]
fn negative_an_agentd_url_with_a_path_or_no_port_is_refused() {
    use voiced::agentd::Endpoint;
    let e = Endpoint::parse("http://127.0.0.1:8787/v1").expect_err("a path is not a base URL");
    assert!(e.to_string().contains("path"), "{e}");
    let e = Endpoint::parse("http://127.0.0.1").expect_err("a port is required");
    assert!(e.to_string().contains("port"), "{e}");
    let e = Endpoint::parse("127.0.0.1:8787").expect_err("the scheme is required");
    assert!(e.to_string().contains("http://"), "{e}");
    assert_eq!(Endpoint::parse("http://MESH_HOST:8787/").expect("valid").addr(), "MESH_HOST:8787");
}

#[test]
fn negative_a_malformed_journal_line_from_agentd_is_rejected_not_guessed() {
    let dir = tmp_dir("badline");
    let wav = dir.join("ask.wav");
    write_tone_wav(&wav, 0.4);
    let mut frames = vec!["{\"seq\":1,\"payload\":{\"kind\":\"ag_ui\",\"type\":\"RUN_STARTED\"".to_string()];
    frames.extend(run_frames("hello", "hi", 0));
    let (addr, _c, _seen, _posts, _h) = fake_agentd(frames, 202, 0);

    let mut cfg = base_config(&dir);
    cfg.stt = voiced::config::SttEngine::Stub { text: "hello".into() };
    cfg.agentd = format!("http://{addr}");

    let (mut journal, sink) = new_journal();
    let req = Request { input: Input::Wav { path: wav.display().to_string() }, out_wav: dir.join("a.wav").display().to_string(), answer_timeout: Duration::from_secs(5), session: None, follow_up: false };
    let outcome = run(&cfg, &req, &mut journal).expect("a bad line must not stop the turn");
    assert_eq!(outcome.answer, "hi");
    let msgs: Vec<String> = events(&sink)
        .iter()
        .filter_map(|e| match &e.payload {
            Payload::AgUi(trace_events::AgUiEvent::RunError { message, .. }) => Some(message.clone()),
            _ => None,
        })
        .collect();
    assert!(msgs.iter().any(|m| m.contains("journal line rejected")), "the bad line must be visible: {msgs:?}");
}

#[test]
fn negative_an_older_run_in_the_backlog_is_never_the_answer() {
    // The phone's agentd journals no user message, so voiced correlates by
    // resuming the SSE stream after the journal length it saw before posting.
    // Without that, an older run's answer would be spoken.
    let dir = tmp_dir("backlog");
    let wav = dir.join("ask.wav");
    write_tone_wav(&wav, 0.4);
    let mut frames = run_frames("older question", "the old answer", 0);
    frames.extend(agentd_like_frames("Battery is at 100 percent.", 8));

    let mut cfg = base_config(&dir);
    cfg.stt = voiced::config::SttEngine::Stub { text: "what is the battery level".into() };
    let (mut journal, sink) = new_journal();

    let (addr, _c, seen, _posts, _h) = fake_agentd(frames, 202, 8);
    cfg.agentd = format!("http://{addr}");
    let req = Request { input: Input::Wav { path: wav.display().to_string() }, out_wav: dir.join("a.wav").display().to_string(), answer_timeout: Duration::from_secs(5), session: None, follow_up: false };
    let outcome = run(&cfg, &req, &mut journal).expect("the turn should complete");
    assert_eq!(outcome.answer, "Battery is at 100 percent.", "the backlog's older answer must not be spoken");
    assert_eq!(*seen.lock().unwrap(), "7", "voiced must resume after the last seq it saw at /health (8 events = last id 7)");
    let msgs: Vec<String> = events(&sink)
        .iter()
        .filter_map(|e| match &e.payload {
            Payload::Op6(trace_events::Op6Event::JournalDropped { reason, .. }) => Some(reason.clone()),
            _ => None,
        })
        .collect();
    assert!(msgs.iter().any(|m| m.contains("no user message")), "the missing user message must be visible in the journal: {msgs:?}");
}

// ---------------------------------------------------------------- the wire

/// The reviewer's mutation: `post_request(&endpoint, "", j)` must go red.
/// Nothing else in the pipeline cares what the body says, so this is the only
/// place that proves the transcript actually reaches agentd.
#[test]
fn the_transcript_is_what_agentd_is_asked() {
    let dir = tmp_dir("body");
    let wav = dir.join("ask.wav");
    write_tone_wav(&wav, 0.4);
    let asked = "remind me to drink water every 2 hours";

    let mut cfg = base_config(&dir);
    cfg.stt = voiced::config::SttEngine::Stub { text: asked.into() };
    let (addr, _c, _seen, posts, _h) = fake_agentd(run_frames(asked, "Set: water every 2 hours.", 0), 202, 0);
    cfg.agentd = format!("http://{addr}");

    let (mut journal, _sink) = new_journal();
    let req = Request { input: Input::Wav { path: wav.display().to_string() }, out_wav: dir.join("a.wav").display().to_string(), answer_timeout: Duration::from_secs(5), session: None, follow_up: false };
    run(&cfg, &req, &mut journal).expect("the turn should complete");

    let bodies = posts.lock().unwrap();
    assert_eq!(bodies.len(), 1, "exactly one request must reach agentd");
    let sent: serde_json::Value = serde_json::from_str(&bodies[0]).expect("the POST body must be valid JSON");
    assert_eq!(sent["text"].as_str(), Some(asked), "agentd must be asked what was heard, not something else");
}

/// What is posted is the *cleaned* transcript, not the raw one.
#[test]
fn the_cleaned_transcript_is_what_agentd_is_asked() {
    let dir = tmp_dir("cleaned");
    let wav = dir.join("ask.wav");
    write_tone_wav(&wav, 0.4);

    let mut cfg = base_config(&dir);
    cfg.stt = voiced::config::SttEngine::Stub { text: "um,   what is the battery level".into() };
    cfg.clean_cmd = Some("tr -s ' '".into());
    let (addr, _c, _seen, posts, _h) = fake_agentd(run_frames("um, what is the battery level", "Battery is at 80 percent.", 0), 202, 0);
    cfg.agentd = format!("http://{addr}");

    let (mut journal, _sink) = new_journal();
    let req = Request { input: Input::Wav { path: wav.display().to_string() }, out_wav: dir.join("a.wav").display().to_string(), answer_timeout: Duration::from_secs(5), session: None, follow_up: false };
    let outcome = run(&cfg, &req, &mut journal).expect("the turn should complete");

    let bodies = posts.lock().unwrap();
    assert_eq!(bodies.len(), 1, "one cleaned request, not a raw one per attempt");
    let sent: serde_json::Value = serde_json::from_str(&bodies[0]).expect("valid JSON");
    assert_eq!(sent["text"].as_str(), Some(outcome.cleaned.as_str()), "the cleaned text is what agentd is asked");
    assert!(!sent["text"].as_str().unwrap().contains("  "), "the collapse must have happened before the POST: {:?}", sent["text"]);
}

/// A transcript with quotes and a backslash must survive the wire intact.
#[test]
fn negative_a_transcript_with_quotes_reaches_agentd_as_valid_json() {
    let dir = tmp_dir("quotes");
    let wav = dir.join("ask.wav");
    write_tone_wav(&wav, 0.4);
    let asked = "turn on \"the kitchen lights\" and set a timer for 5 \\ minutes";

    let mut cfg = base_config(&dir);
    cfg.stt = voiced::config::SttEngine::Stub { text: asked.into() };
    let (addr, _c, _seen, posts, _h) = fake_agentd(run_frames(asked, "Done.", 0), 202, 0);
    cfg.agentd = format!("http://{addr}");

    let (mut journal, _sink) = new_journal();
    let req = Request { input: Input::Wav { path: wav.display().to_string() }, out_wav: dir.join("a.wav").display().to_string(), answer_timeout: Duration::from_secs(5), session: None, follow_up: false };
    run(&cfg, &req, &mut journal).expect("the turn should complete");

    let bodies = posts.lock().unwrap();
    let sent: serde_json::Value = serde_json::from_str(&bodies[0]).expect("quotes must not break the body");
    assert_eq!(sent["text"].as_str(), Some(asked), "the transcript must arrive byte for byte");
}

/// Silence that the engine reports as success still must not become a request.
#[test]
fn negative_a_blank_transcript_is_refused_before_the_post() {
    let dir = tmp_dir("blank");
    let wav = dir.join("ask.wav");
    write_tone_wav(&wav, 0.4);

    let mut cfg = base_config(&dir);
    cfg.stt = voiced::config::SttEngine::Stub { text: "   ".into() };
    let (addr, connections, _seen, posts, _h) = fake_agentd(run_frames("x", "y", 0), 202, 0);
    cfg.agentd = format!("http://{addr}");

    let (mut journal, sink) = new_journal();
    let req = Request { input: Input::Wav { path: wav.display().to_string() }, out_wav: dir.join("a.wav").display().to_string(), answer_timeout: Duration::from_secs(5), session: None, follow_up: false };
    let err = run(&cfg, &req, &mut journal).expect_err("whitespace is not a request");
    assert!(matches!(err, VoiceError::EmptyTranscript { .. }), "got {err:?}");
    assert_eq!(connections.load(Ordering::SeqCst), 0, "a blank transcript must not touch agentd at all");
    assert!(posts.lock().unwrap().is_empty(), "no POST body may have been sent");
    assert!(types(&sink).iter().any(|t| t == "mic_stopped"), "the microphone is still released honestly");
}

// ------------------------------------------------------------- mic state

#[test]
fn negative_mic_off_refuses_capture_and_sends_no_request() {
    let dir = tmp_dir("micoff");
    let wav = dir.join("ask.wav");
    write_tone_wav(&wav, 0.4);
    std::fs::write(dir.join("mic.state"), "off\n").expect("write mic state");
    let (addr, connections, _seen, _posts, _h) = fake_agentd(run_frames("hello", "hi", 0), 202, 0);

    let mut cfg = base_config(&dir);
    cfg.stt = voiced::config::SttEngine::Stub { text: "hello".into() };
    cfg.agentd = format!("http://{addr}");

    let (mut journal, sink) = new_journal();
    let req = Request { input: Input::Wav { path: wav.display().to_string() }, out_wav: dir.join("a.wav").display().to_string(), answer_timeout: Duration::from_secs(5), session: None, follow_up: false };
    let err = run(&cfg, &req, &mut journal).expect_err("mic off must refuse the turn");
    assert!(matches!(err, VoiceError::MicOff { .. }), "got {err:?}");
    assert!(err.to_string().contains("mic off by user: capture refused"), "the refusal must say why: {err}");
    assert_eq!(connections.load(Ordering::SeqCst), 0, "nothing may reach agentd when the mic is off");
    assert!(!types(&sink).iter().any(|t| t == "mic_started"), "capture must never start");
    let code = events(&sink).iter().find_map(|e| match &e.payload {
        Payload::AgUi(trace_events::AgUiEvent::RunError { code, .. }) => code.clone(),
        _ => None,
    });
    assert_eq!(code.as_deref(), Some("mic_off"), "the journal must name the refusal");
}

#[test]
fn mic_on_in_the_state_file_allows_the_turn() {
    let dir = tmp_dir("micon");
    let wav = dir.join("ask.wav");
    write_tone_wav(&wav, 0.4);
    std::fs::write(dir.join("mic.state"), "on\n").expect("write mic state");
    let (addr, _c, _seen, _posts, _h) = fake_agentd(run_frames("hello", "hi there", 0), 202, 0);

    let mut cfg = base_config(&dir);
    cfg.stt = voiced::config::SttEngine::Stub { text: "hello".into() };
    cfg.agentd = format!("http://{addr}");

    let (mut journal, _sink) = new_journal();
    let req = Request { input: Input::Wav { path: wav.display().to_string() }, out_wav: dir.join("a.wav").display().to_string(), answer_timeout: Duration::from_secs(5), session: None, follow_up: false };
    let outcome = run(&cfg, &req, &mut journal).expect("mic on must let the turn run");
    assert_eq!(outcome.answer, "hi there");
}

#[test]
fn negative_a_mic_state_file_that_says_neither_on_nor_off_is_refused_by_name() {
    let dir = tmp_dir("micbad");
    let wav = dir.join("ask.wav");
    write_tone_wav(&wav, 0.4);
    std::fs::write(dir.join("mic.state"), "banana\n").expect("write mic state");
    let (addr, connections, _seen, _posts, _h) = fake_agentd(run_frames("hello", "hi", 0), 202, 0);

    let mut cfg = base_config(&dir);
    cfg.stt = voiced::config::SttEngine::Stub { text: "hello".into() };
    cfg.agentd = format!("http://{addr}");

    let (mut journal, _sink) = new_journal();
    let req = Request { input: Input::Wav { path: wav.display().to_string() }, out_wav: dir.join("a.wav").display().to_string(), answer_timeout: Duration::from_secs(5), session: None, follow_up: false };
    let err = run(&cfg, &req, &mut journal).expect_err("an unreadable mic state must fail closed");
    assert!(err.to_string().contains("must say 'on' or 'off'"), "the bad value must be named: {err}");
    assert!(err.to_string().contains("banana"), "verbatim: {err}");
    assert_eq!(connections.load(Ordering::SeqCst), 0, "no request may be sent on an unknown mic state");
}

#[test]
fn negative_an_agentd_url_with_no_host_is_refused() {
    let err = voiced::agentd::Endpoint::parse("http://:8770").expect_err("a URL with no host is not an endpoint");
    match err {
        VoiceError::Config(m) => assert!(m.contains("no host"), "the message must name the missing host: {m}"),
        e => panic!("expected a Config error, got {e:?}"),
    }
    let err = voiced::agentd::Endpoint::parse("http://127.0.0.1:spoken").expect_err("a non-numeric port is not an endpoint");
    match err {
        VoiceError::Config(m) => assert!(m.contains("not a number"), "the message must name the bad port: {m}"),
        e => panic!("expected a Config error, got {e:?}"),
    }
}

// ------------------------------------------------------------------ CV0 ack

/// A fake agentd that answers slowly: GET /events waits `delay`
/// before streaming the run frames, so the ack deadline passes with
/// no answer ready. Health and POST behave like the real thing.
fn fake_agentd_slow(frames: Vec<String>, delay: Duration) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind slow fake agentd");
    let addr = listener.local_addr().expect("addr").to_string();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut buf = [0u8; 4096];
            let n = stream.read(&mut buf).unwrap_or(0);
            let head = String::from_utf8_lossy(&buf[..n]).to_string();
            if head.starts_with("POST") {
                let body = format!("{{\"run_id\":\"{}\",\"answer\":null,\"error\":null,\"brain\":\"fake\"}}", last_run_id(&frames));
                let _ = stream.write_all(format!("HTTP/1.1 202 Accepted\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len()).as_bytes());
                let _ = stream.flush();
                continue;
            }
            if head.starts_with("GET /health") {
                let body = "{\"ok\":true,\"events\":0}";
                let _ = stream.write_all(format!("HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len()).as_bytes());
                let _ = stream.flush();
                continue;
            }
            std::thread::sleep(delay);
            let sse = frames.iter().map(|f| format!("data: {f}\n\n")).collect::<String>();
            let _ = stream.write_all(format!("HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n\r\n{sse}").as_bytes());
            let _ = stream.flush();
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
    });
    addr
}

fn journal_text(sink: &Sink) -> String {
    String::from_utf8(sink.0.lock().unwrap().clone()).expect("journal is utf8")
}

fn ack_config(dir: &Path, agentd: String, delay_ms: u64) -> Config {
    let stt = script(&dir, "stt.sh", "cat > /dev/null; printf 'open maps\\n'");
    let clip = dir.join("ack.wav");
    write_tone_wav(&clip, 0.4);
    let mut cfg = base_config(dir);
    cfg.stt = voiced::config::SttEngine::Cmd { cmd: stt };
    cfg.tts = voiced::config::TtsEngine::None;
    cfg.agentd = agentd;
    cfg.ack_clips = vec![clip.display().to_string()];
    cfg.ack_delay_ms = delay_ms;
    cfg
}

#[test]
fn cv0_ack_fires_exactly_once_when_the_answer_is_slow() {
    let dir = tmp_dir("ackslow");
    let wav = dir.join("ask.wav");
    write_tone_wav(&wav, 0.5);
    let addr = fake_agentd_slow(run_frames("open maps", "Maps is open.", 0), Duration::from_millis(400));
    let cfg = ack_config(&dir, format!("http://{addr}"), 50);

    let (mut journal, sink) = new_journal();
    let req = Request { input: Input::Wav { path: wav.display().to_string() }, out_wav: dir.join("a.wav").display().to_string(), answer_timeout: Duration::from_secs(5), session: None, follow_up: false };
    let outcome = run(&cfg, &req, &mut journal).expect("the slow turn should still succeed");
    assert_eq!(outcome.answer, "Maps is open.");

    let text = journal_text(&sink);
    assert_eq!(text.matches("ack fired").count(), 1, "exactly one ack line, and it fired: {text}");
    assert_eq!(text.matches("ack not fired").count(), 0, "a fired ack is not also not-fired: {text}");
    assert!(text.contains("playback:ack"), "the ack line names its destination: {text}");
}

#[test]
fn cv0_ack_not_fired_when_the_answer_is_fast() {
    let dir = tmp_dir("ackfast");
    let wav = dir.join("ask.wav");
    write_tone_wav(&wav, 0.5);
    let (addr, _c, _seen, _posts, _h) = fake_agentd(run_frames("open maps", "Maps is open.", 0), 202, 0);
    let cfg = ack_config(&dir, format!("http://{addr}"), 600);

    let (mut journal, sink) = new_journal();
    let req = Request { input: Input::Wav { path: wav.display().to_string() }, out_wav: dir.join("a.wav").display().to_string(), answer_timeout: Duration::from_secs(5), session: None, follow_up: false };
    let outcome = run(&cfg, &req, &mut journal).expect("the fast turn should succeed");
    assert_eq!(outcome.answer, "Maps is open.");

    let text = journal_text(&sink);
    assert_eq!(text.matches("ack fired").count(), 0, "a fast answer must not be acked: {text}");
    assert_eq!(text.matches("ack not fired").count(), 1, "the not-fired outcome is journalled too: {text}");
}

// ------------------------------------------------- stale-answer regression
//
// The defect these tests pin (found live by the CV0 proof runs, 2026-11-03):
// voiced correlated the answer it spoke by journal POSITION — it read
// agentd's /health `events` count, POSTed, then tailed /events from that
// position and spoke the first run it found. Two numberings were being
// treated as one: `events` is the count of records the ring currently
// HOLDS, not the last `seq` written; and the count itself was read through
// a headers-only read, while agentd writes its response head and body in
// two separate writes — so the count usually never arrived and the resume
// position silently collapsed to 0 ("send me everything"). Either way the
// tail then began at an older run and voiced spoke that run's answer.
// The fix binds the answer to the run id agentd returns for the POST, so
// position can be wrong in either direction and the spoken answer is
// still this turn's. These fakes model the real server's wire behaviour
// (two-write responses, `len()` health, seq spaces that start above the
// held count after ring evictions) rather than the convenient version.

/// One run in agentd's real journal shape: the request rides on
/// RUN_STARTED's input, the answer message carries the run id as its
/// messageId, RUN_FINISHED carries the run id and the answer again.
fn agentd_run_frames(ask: &str, answer: &str, run_id: &str, from: u64) -> Vec<String> {
    let mut seq = from;
    let mut next = |v: serde_json::Value| {
        let mut v = v;
        v.as_object_mut().unwrap().insert("seq".into(), serde_json::json!(seq));
        seq += 1;
        frame(v)
    };
    vec![
        next(serde_json::json!({"payload": {"kind": "ag_ui", "type": "RUN_STARTED", "threadId": "phone", "runId": run_id, "input": {"text": ask}}})),
        next(serde_json::json!({"payload": {"kind": "ag_ui", "type": "TEXT_MESSAGE_START", "messageId": run_id, "role": "assistant"}})),
        next(serde_json::json!({"payload": {"kind": "ag_ui", "type": "TEXT_MESSAGE_CONTENT", "messageId": run_id, "delta": answer}})),
        next(serde_json::json!({"payload": {"kind": "ag_ui", "type": "TEXT_MESSAGE_END", "messageId": run_id}})),
        next(serde_json::json!({"payload": {"kind": "ag_ui", "type": "RUN_FINISHED", "threadId": "phone", "runId": run_id, "result": {"answer": answer, "brain": "fake"}}})),
    ]
}

struct LiveFake {
    addr: String,
    seen_resume: Arc<Mutex<String>>,
}

/// A stateful fake agentd. The journal is a shared frame list; a POST
/// appends a real-shaped run with the scripted answer for the posted text
/// and replies with that run's id, exactly like the real handler. /health
/// reports the number of frames currently held — the ring's `len()`, not
/// the last seq — and when `split_health` is set it writes the health head
/// and body in two writes with a gap, the way agentd's `write_response`
/// puts them on the wire.
fn fake_agentd_live(seed: Vec<String>, next_seq: u64, script: Vec<(String, String)>, split_health: bool) -> LiveFake {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind live fake agentd");
    let addr = listener.local_addr().expect("addr").to_string();
    let frames = Arc::new(Mutex::new(seed));
    let next_seq = Arc::new(Mutex::new(next_seq));
    let script = Arc::new(Mutex::new(script));
    let runs = Arc::new(AtomicUsize::new(0));
    let seen_resume = Arc::new(Mutex::new(String::new()));
    let seen = seen_resume.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            // Read the head, then the body if the head names one.
            let mut head_end = usize::MAX;
            loop {
                match stream.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(n) => {
                        buf.extend_from_slice(&chunk[..n]);
                        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                            head_end = pos + 4;
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            let head = String::from_utf8_lossy(&buf[..]).to_string();
            if head.starts_with("POST") {
                let want: usize = head
                    .lines()
                    .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse().unwrap_or(0)))
                    .unwrap_or(0);
                while buf.len() - head_end.min(buf.len()) < want {
                    match stream.read(&mut chunk) {
                        Ok(0) => break,
                        Ok(n) => buf.extend_from_slice(&chunk[..n]),
                        Err(_) => break,
                    }
                }
                let body = String::from_utf8_lossy(&buf[head_end.min(buf.len())..]).to_string();
                let ask = serde_json::from_str::<serde_json::Value>(&body).ok().and_then(|v| v.get("text").and_then(|t| t.as_str()).map(str::to_string)).unwrap_or_default();
                let answer = script
                    .lock()
                    .unwrap()
                    .iter()
                    .find(|(a, _)| *a == ask)
                    .map(|(_, ans)| ans.clone())
                    .unwrap_or_else(|| format!("no scripted answer for {ask:?}"));
                let k = runs.fetch_add(1, Ordering::SeqCst);
                let run_id = format!("run-live-{k}");
                {
                    let mut ns = next_seq.lock().unwrap();
                    let new_frames = agentd_run_frames(&ask, &answer, &run_id, *ns);
                    *ns += new_frames.len() as u64;
                    frames.lock().unwrap().extend(new_frames);
                }
                let rbody = serde_json::json!({"run_id": run_id, "answer": answer, "error": null, "brain": "fake"}).to_string();
                let _ = stream.write_all(format!("HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n", rbody.len()).as_bytes());
                let _ = stream.flush();
                std::thread::sleep(Duration::from_millis(20));
                let _ = stream.write_all(rbody.as_bytes());
                let _ = stream.flush();
                continue;
            }
            if head.starts_with("GET /health") {
                let held = frames.lock().unwrap().len();
                let hbody = format!("{{\"ok\":true,\"events\":{held},\"dropped\":0}}");
                let _ = stream.write_all(format!("HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n", hbody.len()).as_bytes());
                let _ = stream.flush();
                if split_health {
                    std::thread::sleep(Duration::from_millis(150));
                }
                let _ = stream.write_all(hbody.as_bytes());
                let _ = stream.flush();
                continue;
            }
            let resume: Option<u64> = head
                .lines()
                .find(|l| l.to_ascii_lowercase().starts_with("last-event-id:"))
                .and_then(|l| l.split_once(':'))
                .and_then(|(_, v)| v.trim().parse::<u64>().ok());
            *seen.lock().unwrap() = resume.map(|r| r.to_string()).unwrap_or_default();
            let held = frames.lock().unwrap().clone();
            let keep: Vec<&String> = held
                .iter()
                .filter(|f| {
                    let seq = serde_json::from_str::<serde_json::Value>(f).ok().and_then(|v| v.get("seq").and_then(|s| s.as_u64())).unwrap_or(0);
                    resume.map(|r| seq > r).unwrap_or(true)
                })
                .collect();
            let sse: String = keep
                .iter()
                .map(|f| {
                    let seq = serde_json::from_str::<serde_json::Value>(f).ok().and_then(|v| v.get("seq").and_then(|s| s.as_u64())).unwrap_or(0);
                    format!("id: {seq}\ndata: {f}\n\n")
                })
                .collect();
            let _ = stream.write_all(format!("HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n\r\n{sse}").as_bytes());
            let _ = stream.flush();
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
    });
    LiveFake { addr, seen_resume }
}

fn live_turn(dir: &Path, addr: &str, ask: &str) -> voiced::Outcome {
    let wav = dir.join(format!("ask-{}.wav", ask.len()));
    write_tone_wav(&wav, 0.4);
    let mut cfg = base_config(dir);
    cfg.stt = voiced::config::SttEngine::Stub { text: ask.into() };
    cfg.agentd = format!("http://{addr}");
    let (mut journal, _sink) = new_journal();
    let req = Request { input: Input::Wav { path: wav.display().to_string() }, out_wav: dir.join("a.wav").display().to_string(), answer_timeout: Duration::from_secs(5), session: None, follow_up: false };
    run(&cfg, &req, &mut journal).expect("the turn should complete")
}

#[test]
fn repro_stale_answer_when_the_health_body_loses_the_race() {
    // The mechanism, live on the phone (59 of 60 sampled /health reads):
    // agentd writes the response head and body separately; voiced read
    // only to the end of the head, so the `events` count never arrived,
    // the resume position silently became 0, and the tail replayed the
    // whole journal — the oldest run in it was spoken. Here the fake puts
    // a 150 ms gap between the health head and body, the losing branch of
    // that race, deterministically.
    let dir = tmp_dir("stale-race");
    let seed = agentd_run_frames("open maps and give me directions", "STALE: Maps is ready to open — where should I take you?", "run-old", 0);
    let fake = fake_agentd_live(seed, 5, vec![("what is the battery level".into(), "FRESH: the battery is at 42 percent.".into())], true);
    let outcome = live_turn(&dir, &fake.addr, "what is the battery level");
    assert_eq!(outcome.answer, "FRESH: the battery is at 42 percent.", "the turn must speak its own run's answer, not the journal's oldest");
}

#[test]
fn repro_stale_answer_when_the_ring_has_dropped_events() {
    // The second disagreement: after the ring evicts, the held count and
    // the seq space part company. The journal below holds 5 frames whose
    // seqs are 40..=44, so /health honestly reports 5 — and a resume
    // position computed as "count - 1" lands at 4, before the entire held
    // backlog, old run included. Position alone cannot survive this; the
    // run id has to decide.
    let dir = tmp_dir("stale-dropped");
    let seed = agentd_run_frames("open maps and give me directions", "STALE: Maps is ready to open — where should I take you?", "run-old", 40);
    let fake = fake_agentd_live(seed, 45, vec![("what is the battery level".into(), "FRESH: the battery is at 42 percent.".into())], false);
    let outcome = live_turn(&dir, &fake.addr, "what is the battery level");
    assert_eq!(outcome.answer, "FRESH: the battery is at 42 percent.", "a resume position inside the backlog must not resurrect an older run's answer");
}

#[test]
fn back_to_back_turns_speak_their_own_answers() {
    let dir = tmp_dir("back-to-back");
    let fake = fake_agentd_live(
        Vec::new(),
        0,
        vec![
            ("what colour is the sky".into(), "Answer one: the sky is blue.".into()),
            ("what colour is grass".into(), "Answer two: grass is green.".into()),
        ],
        true,
    );
    let one = live_turn(&dir, &fake.addr, "what colour is the sky");
    let two = live_turn(&dir, &fake.addr, "what colour is grass");
    assert_eq!(one.answer, "Answer one: the sky is blue.");
    assert_eq!(two.answer, "Answer two: grass is green.");
    assert_ne!(one.answer, two.answer, "distinct questions must not collapse onto one answer");
}

// ------------------------------------------------------------------ CV2 smart turn

/// Raw s16le PCM: a loud sine segment reads as speech to the
/// endpointer, a zero segment as silence.
fn raw_pcm(segs: &[(bool, f32)]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut t = 0usize;
    for (loud, secs) in segs {
        let n = (secs * 16_000.0) as usize;
        for i in 0..n {
            let v: i16 = if *loud {
                (2600.0 * (2.0 * std::f32::consts::PI * 440.0 * ((t + i) as f32) / 16_000.0).sin()) as i16
            } else {
                0
            };
            out.extend_from_slice(&v.to_le_bytes());
        }
        t += n;
    }
    out
}

/// The CV0 defect stream: "open maps and give me directions to" —
/// 1.5 s mid-sentence pause — "San Francisco" — trailing silence.
/// Unjudged, the capture dies in the pause (1.7 s kept); judged, it
/// must survive whole (~3.8 s kept).
fn pause_stream(dir: &Path) -> String {
    let pcm = raw_pcm(&[(true, 1.0), (false, 1.5), (true, 1.0), (false, 0.8)]);
    let path = dir.join("pause.raw");
    std::fs::write(&path, pcm).expect("write raw stream");
    format!("cat {}", path.display())
}

#[test]
fn cv2_judged_capture_survives_the_mid_sentence_pause() {
    let dir = tmp_dir("cv2pause");
    // A judge that judges CONTENT the way the real model does on this
    // fixture (incomplete until the destination has been spoken):
    // verdicts keyed on how much audio it was shown.
    // Stateful judge: the FIRST consultation of the first pause says
    // incomplete; every later one says a confident complete. Under
    // the first-verdict anchor the later completes of that pause may
    // NOT end the capture - only the fresh first verdict of the
    // final pause (confident) ends it.
    let count = dir.join("judge-count");
    let judge = script(
        &dir,
        "judge.sh",
        &format!(
            "c=$(cat {} 2>/dev/null || echo 0); echo $((c+1)) > {}; cat > /dev/null; if [ \"$c\" -eq 0 ]; then printf 'incomplete 0.08\\n'; else printf 'complete 0.99\\n'; fi",
            count.display(),
            count.display()
        ),
    );
    let (mut journal, sink) = new_journal();
    let pcm = voiced::capture::capture_mic(
        &pause_stream(&dir),
        8,
        Some(&voiced::turn::TurnJudge::new(&judge)),
        &mut journal,
    )
    .expect("judged capture succeeds");
    let secs = pcm.samples.len() as f32 / 32_000.0;
    assert!(secs > 3.6 && secs < 4.0, "the whole utterance survives: {secs:.2} s captured");
    let text = journal_text(&sink);
    assert!(text.contains("still listening"), "the incomplete verdict is journalled: {text}");
    assert!(text.contains("first-verdict anchor"), "the anchored hold is journalled: {text}");
    assert!(text.contains("smart turn complete (p=0.99)"), "the judged stop names itself: {text}");
}

#[test]
fn cv2_unjudged_capture_still_cuts_at_the_pause() {
    // The control: same stream, no judge configured — the CV0 rule
    // ends the capture in the pause. This is also the fallback's
    // promised behaviour, pinned at the capture level.
    let dir = tmp_dir("cv2control");
    let (mut journal, sink) = new_journal();
    let pcm = voiced::capture::capture_mic(&pause_stream(&dir), 8, None, &mut journal).expect("capture succeeds");
    let secs = pcm.samples.len() as f32 / 32_000.0;
    assert!(secs > 1.6 && secs < 1.85, "CV0 cut at 1.7 s: {secs:.2} s captured");
    assert!(journal_text(&sink).contains(voiced::capture::CV0_STOP_REASON), "the CV0 reason, word for word");
}

#[test]
fn cv2_broken_judge_falls_back_to_cv0_endpointing() {
    // FAIL-OPERATIONAL: the adapter is down (exits 1) — the capture
    // must end exactly as CV0 would end it, with the fallback
    // journalled loudly.
    let dir = tmp_dir("cv2fallback");
    let judge = script(&dir, "judge-down.sh", "cat > /dev/null; exit 1");
    let (mut journal, sink) = new_journal();
    let pcm = voiced::capture::capture_mic(
        &pause_stream(&dir),
        8,
        Some(&voiced::turn::TurnJudge::new(&judge)),
        &mut journal,
    )
    .expect("capture succeeds despite the dead judge");
    let secs = pcm.samples.len() as f32 / 32_000.0;
    assert!(secs > 1.6 && secs < 1.85, "fallback cut at the CV0 endpoint: {secs:.2} s");
    let text = journal_text(&sink);
    assert!(text.contains("smart turn unavailable"), "the fallback is loud in the journal: {text}");
    assert!(text.contains("cv0 endpointing fallback"), "the fallback names its rule: {text}");
    assert!(text.contains(voiced::capture::CV0_STOP_REASON), "the stop reason is the CV0 one");
}

#[test]
fn cv2_middling_complete_needs_a_confident_confirmation() {
    // A judge that always answers a MIDDLING complete (0.7): the
    // first such verdict leans complete, but no later verdict ever
    // confirms CONFIDENTLY, so the capture must ride to the 2.5 s
    // trailing cap instead of ending on middling evidence.
    // Stream: 1.0 s speech, then 3.0 s silence: end at ~3.5 s.
    let dir = tmp_dir("cv2middling");
    let judge = script(&dir, "judge-mid.sh", "cat > /dev/null; printf 'complete 0.70\\n'");
    let pcm = raw_pcm(&[(true, 1.0), (false, 3.0)]);
    let path = dir.join("stream.raw");
    std::fs::write(&path, pcm).expect("write stream");
    let (mut journal, sink) = new_journal();
    let got = voiced::capture::capture_mic(
        &format!("cat {}", path.display()),
        8,
        Some(&voiced::turn::TurnJudge::new(&judge)),
        &mut journal,
    )
    .expect("capture succeeds");
    let secs = got.samples.len() as f32 / 32_000.0;
    assert!(secs > 3.4 && secs < 3.65, "ends at the trailing cap, not on middling verdicts: {secs:.2} s");
    let text = journal_text(&sink);
    assert!(text.contains("pause locked"), "the lock is journalled: {text}");
    assert!(text.contains("no confident end by 2.5 s"), "the cap end names itself: {text}");
}
