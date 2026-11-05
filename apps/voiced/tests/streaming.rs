//! CV3 streaming: voiced starts the tail before the POST returns,
//! speaks each `answer_chunk` of its own run as it lands, and keeps
//! every pre-CV3 behavior when no chunks arrive. The fake agentd
//! adopts the run id voiced posts (the CV3 wire), streams chunk
//! records with real delays, and records the exact moment the run
//! finished — so the headline assertion is an ordering fact: the
//! first sentence was synthesized *before* the run finished.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use voice_bridge::wav::{read_pcm, write_pcm};
use voiced::capture::JournalOut;
use voiced::config::{Config, SttEngine, TtsEngine};
use voiced::{run, Input, Request};

const REQ: &str = "open maps and give me directions to San Francisco";
const FULL: &str = "First sentence here. Second sentence follows.";
const CHUNK0: &str = "First sentence here.";
const CHUNK1: &str = "Second sentence follows.";

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Stream,
    Plain,
    ErrorAfterChunk,
}

struct Fake {
    port: u16,
}

fn tmp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("voiced-cv3-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("tmp dir");
    dir
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

fn tone_wav(path: &Path, seconds: f32) {
    let n = (16_000.0 * seconds) as usize;
    let mut bytes = Vec::with_capacity(n * 2);
    for i in 0..n {
        let v: i16 = if (i / 40) % 2 == 0 { 6000 } else { -6000 };
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    write_pcm(path.to_str().unwrap(), 16_000, 2, 1, &bytes).expect("write tone wav");
}

fn base_config(dir: &Path) -> Config {
    Config {
        stt: SttEngine::Stub { text: REQ.to_string() },
        tts: TtsEngine::None,
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

fn frame(seq: u64, payload: serde_json::Value) -> String {
    let line = serde_json::json!({
        "seq": seq,
        "ts_us": 1,
        "service": "agentd",
        "payload": payload,
    });
    format!("data: {line}\n\n")
}

fn run_started(rid: &str) -> serde_json::Value {
    serde_json::json!({"kind": "ag_ui", "type": "RUN_STARTED", "threadId": "phone", "runId": rid, "input": {"text": REQ}})
}

fn chunk(rid: &str, index: usize, text: &str) -> serde_json::Value {
    serde_json::json!({"kind": "ag_ui", "type": "CUSTOM", "name": "answer_chunk", "value": {"run_id": rid, "index": index, "text": text}})
}

fn spawn_fake(mode: Mode, order_file: PathBuf) -> Fake {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake agentd");
    let port = listener.local_addr().unwrap().port();
    let adopted: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let adopted = Arc::clone(&adopted);
            let order_file = order_file.clone();
            std::thread::spawn(move || handle_conn(&mut stream, mode, &adopted, &order_file));
        }
    });
    Fake { port }
}

/// Append one line to the shared order log (the TTS script appends
/// its synth lines to the same file, so the file is a single true
/// timeline of the turn).
fn order_push(path: &Path, line: &str) {
    use std::fs::OpenOptions;
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{line}");
    }
}

fn read_request(stream: &mut TcpStream) -> (String, String) {
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    let head_end = loop {
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break pos;
        }
        if stream.read(&mut byte).unwrap_or(0) == 0 {
            return (String::new(), String::new());
        }
        buf.extend_from_slice(&byte);
        if buf.len() > 200_000 {
            return (String::new(), String::new());
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let first_line = head.lines().next().unwrap_or("").to_string();
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
    (first_line, String::from_utf8_lossy(&body).to_string())
}

fn respond(stream: &mut TcpStream, status: &str, content_type: &str, body: &str) {
    let _ = stream.write_all(
        format!("HTTP/1.1 {status}\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len())
            .as_bytes(),
    );
    let _ = stream.flush();
}

fn handle_conn(stream: &mut TcpStream, mode: Mode, adopted: &Arc<Mutex<Option<String>>>, order_file: &Path) {
    let (first_line, body) = read_request(stream);
    if first_line.starts_with("GET /health") {
        respond(stream, "200 OK", "application/json", "{\"ok\":true,\"events\":0,\"dropped\":0,\"brains\":[\"fake\"]}");
        return;
    }
    if first_line.starts_with("POST /request") {
        let rid = extract_run_id(&body).unwrap_or_else(|| "run-fake-1".to_string());
        *adopted.lock().unwrap() = Some(rid.clone());
        let resp = serde_json::json!({"run_id": rid, "answer": FULL, "error": null, "brain": "fake"});
        respond(stream, "200 OK", "application/json", &resp.to_string());
        return;
    }
    if first_line.starts_with("GET /events") {
        // The tail may connect before the POST lands; wait for the id.
        let rid = loop {
            if let Some(r) = adopted.lock().unwrap().clone() {
                break r;
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        let _ = stream.write_all(b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n");
        let _ = stream.flush();
        let mut seq = 1u64;
        let mut send = |stream: &mut TcpStream, payload: serde_json::Value| {
            let f = frame(seq, payload);
            seq += 1;
            let _ = stream.write_all(f.as_bytes());
            let _ = stream.flush();
        };
        send(stream, run_started(&rid));
        match mode {
            Mode::Stream => {
                std::thread::sleep(Duration::from_millis(300));
                send(stream, chunk(&rid, 0, CHUNK0));
                std::thread::sleep(Duration::from_millis(600));
                send(stream, chunk(&rid, 1, CHUNK1));
            }
            Mode::ErrorAfterChunk => {
                std::thread::sleep(Duration::from_millis(200));
                send(stream, chunk(&rid, 0, CHUNK0));
                std::thread::sleep(Duration::from_millis(100));
                send(stream, serde_json::json!({"kind": "ag_ui", "type": "RUN_ERROR", "message": "brain died"}));
                return;
            }
            Mode::Plain => {}
        }
        send(stream, serde_json::json!({"kind": "ag_ui", "type": "TEXT_MESSAGE_START", "messageId": rid, "role": "assistant"}));
        send(stream, serde_json::json!({"kind": "ag_ui", "type": "TEXT_MESSAGE_CONTENT", "messageId": rid, "delta": FULL}));
        send(stream, serde_json::json!({"kind": "ag_ui", "type": "TEXT_MESSAGE_END", "messageId": rid}));
        send(stream, serde_json::json!({"kind": "ag_ui", "type": "RUN_FINISHED", "threadId": "phone", "runId": rid}));
        order_push(order_file, "finished-sent");
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn extract_run_id(body: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    v.get("run_id").and_then(|r| r.as_str()).map(str::to_string)
}

fn setup(tag: &str, mode: Mode) -> (PathBuf, Config, Fake, PathBuf) {
    let dir = tmp_dir(tag);
    let input = dir.join("ask.wav");
    tone_wav(&input, 0.5);
    let src = dir.join("tone.wav");
    tone_wav(&src, 0.2);
    let order_file = dir.join("order.log");
    let tts = script(
        &dir,
        "tts.sh",
        &format!(
            "text=$(cat); printf 'synth:%s\\n' \"$text\" >> {}; cp {} \"$1\"",
            order_file.display(),
            src.display()
        ),
    );
    let fake = spawn_fake(mode, order_file.clone());
    let mut cfg = base_config(&dir);
    cfg.tts = TtsEngine::Cmd { cmd: tts };
    cfg.agentd = format!("http://127.0.0.1:{}", fake.port);
    (dir, cfg, fake, input)
}

fn run_turn(cfg: &Config, input: &Path, out: &Path) -> (Result<voiced::Outcome, voiced::error::VoiceError>, JournalOut) {
    let mut journal = JournalOut::with_service(Box::new(Vec::<u8>::new()), "voiced".to_string());
    let req = Request {
        input: Input::Wav { path: input.display().to_string() },
        out_wav: out.display().to_string(),
        answer_timeout: Duration::from_secs(15),
        session: None,
        follow_up: false,
    };
    let outcome = run(cfg, &req, &mut journal);
    (outcome, journal)
}

fn synth_log(dir: &Path) -> Vec<String> {
    std::fs::read_to_string(dir.join("order.log"))
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

#[test]
fn chunks_are_spoken_before_the_run_finishes() {
    let (dir, cfg, _fake, input) = setup("stream", Mode::Stream);
    let out = dir.join("answer.wav");
    let (outcome, _journal) = run_turn(&cfg, &input, &out);
    let outcome = outcome.expect("the streamed turn succeeds");
    assert_eq!(outcome.answer, FULL);

    let log = synth_log(&dir);
    let synths: Vec<&String> = log.iter().filter(|l| l.starts_with("synth:")).collect();
    assert_eq!(
        synths,
        vec![&format!("synth:{CHUNK0}"), &format!("synth:{CHUNK1}")],
        "exactly the two chunks were synthesized — never the whole answer: {log:?}"
    );
    // The headline ordering fact, from the single shared timeline:
    // the first sentence was synthesized BEFORE the fake recorded
    // the run's finish — speech overlapped generation.
    let pos = |needle: &str| log.iter().position(|l| l == needle).expect("line present");
    assert!(
        pos(&format!("synth:{CHUNK0}")) < pos("finished-sent"),
        "first chunk synthesized before the run finished: {log:?}"
    );

    // The combined answer file is the two chunk WAVs concatenated.
    let combined = read_pcm(&out).expect("combined answer is real audio");
    let src = read_pcm(dir.join("tone.wav")).expect("tone");
    assert_eq!(combined.samples.len(), src.samples.len() * 2, "two chunks of audio");
    // And the chunk files exist beside it.
    assert!(Path::new(&voiced::stream::chunk_path(&out.display().to_string(), 0)).exists());
    assert!(Path::new(&voiced::stream::chunk_path(&out.display().to_string(), 1)).exists());
}

#[test]
fn no_chunks_means_the_classic_whole_answer_path() {
    let (dir, cfg, _fake, input) = setup("plain", Mode::Plain);
    let out = dir.join("answer.wav");
    let (outcome, _journal) = run_turn(&cfg, &input, &out);
    let outcome = outcome.expect("the plain turn succeeds");
    assert_eq!(outcome.answer, FULL);

    let log = synth_log(&dir);
    let synths: Vec<&String> = log.iter().filter(|l| l.starts_with("synth:")).collect();
    assert_eq!(synths, vec![&format!("synth:{FULL}")], "one whole-answer synthesis, as before CV3: {log:?}");
}

#[test]
fn a_run_error_after_a_chunk_fails_the_turn_with_only_the_chunk_spoken() {
    let (dir, cfg, _fake, input) = setup("error", Mode::ErrorAfterChunk);
    let out = dir.join("answer.wav");
    let (outcome, _journal) = run_turn(&cfg, &input, &out);
    assert!(outcome.is_err(), "the turn must fail when its run errors: {outcome:?}");

    let log = synth_log(&dir);
    let synths: Vec<&String> = log.iter().filter(|l| l.starts_with("synth:")).collect();
    assert_eq!(synths, vec![&format!("synth:{CHUNK0}")], "the chunk was spoken; nothing else was: {log:?}");
}
