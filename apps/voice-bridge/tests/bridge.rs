//! Integration tests against in-process fake Wyoming servers.
//!
//! Each test binds an ephemeral loopback listener, plays a scripted
//! Wyoming exchange on a thread, and drives `run_stt`/`run_tts` against
//! it. Assertions cover both the returned result and the journal: the
//! JSONL on the sink must parse back through `from_jsonl` with gapless
//! seq, and failures must produce explicit error events — never silence.

use std::io::BufReader;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::thread;

use trace_events::{
    from_jsonl, AgUiEvent, Event, GenAiEvent, HandoffStatus, Op6Event, Payload, SpanStatus,
};
use wyoming::events::{asr::*, audio::*, error::ErrorEvent, tts::*, Eventable};
use wyoming::{decode, encode_into};

use voice_bridge::error::BridgeError;
use voice_bridge::journal::Journal;
use voice_bridge::stt::{run_stt, SttOptions, CHUNK_SAMPLES};
use voice_bridge::tts::{run_tts, TtsOptions};
use voice_bridge::wav::{read_pcm, Pcm};

/// What the fake server observed, shared back to the test thread.
#[derive(Default)]
struct Seen {
    audio_start: Option<(u32, u8, u8)>, // rate, width, channels
    audio: Vec<u8>,
    transcribe_language: Option<String>,
    synthesize_text: Option<String>,
    synthesize_voice: Option<String>,
}

type SeenRef = Arc<Mutex<Seen>>;

/// Spawn a fake Wyoming server that runs `script` on one accepted
/// connection. Returns the address and a channel that fires once the
/// script has finished (so tests can join deterministically).
fn spawn_server<F>(script: F) -> (SocketAddr, Receiver<()>)
where
    F: FnOnce(&mut BufReader<TcpStream>, &mut TcpStream) -> Result<(), wyoming::Error>
        + Send
        + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept");
        let mut writer = stream.try_clone().expect("clone socket");
        let mut reader = BufReader::new(stream);
        let _ = script(&mut reader, &mut writer);
        let _ = tx.send(());
    });
    (addr, rx)
}

/// Connect the client side and split into reader/writer.
fn client_pair(addr: SocketAddr) -> (BufReader<TcpStream>, TcpStream) {
    let stream = TcpStream::connect(addr).expect("connect fake server");
    let writer = stream.try_clone().unwrap();
    (BufReader::new(stream), writer)
}

fn pcm16k(rate: u32, samples: usize) -> Pcm {
    Pcm {
        rate,
        channels: 1,
        width: 2,
        samples: (0..samples as i16).flat_map(|s| s.to_le_bytes()).collect(),
    }
}

/// One-line summary of each journal payload, for order assertions.
fn kinds(events: &[Event]) -> Vec<String> {
    events
        .iter()
        .map(|e| match &e.payload {
            Payload::Op6(Op6Event::Handoff { status, .. }) => match status {
                HandoffStatus::Started => "handoff-started",
                HandoffStatus::Completed => "handoff-completed",
                HandoffStatus::Failed => "handoff-failed",
            }
            .to_string(),
            Payload::Op6(Op6Event::MicFrame { .. }) => "mic-frame".to_string(),
            Payload::Op6(Op6Event::SttPartial { .. }) => "stt-partial".to_string(),
            Payload::Op6(Op6Event::SttFinal { .. }) => "stt-final".to_string(),
            Payload::GenAi(GenAiEvent::SpanStart { .. }) => "span-start".to_string(),
            Payload::GenAi(GenAiEvent::SpanEnd { status, .. }) => match status {
                SpanStatus::Ok => "span-end-ok",
                SpanStatus::Error => "span-end-error",
            }
            .to_string(),
            Payload::AgUi(AgUiEvent::RunError { .. }) => "run-error".to_string(),
            other => format!("other:{other:?}"),
        })
        .collect()
}

/// Assert the sink bytes parse back through from_jsonl, gapless seq,
/// and match the in-memory ring snapshot.
fn assert_journal_consistent(journal: &Journal<Vec<u8>>) -> Vec<Event> {
    let text = String::from_utf8(journal.sink().clone()).expect("utf8 jsonl");
    let parsed = from_jsonl(&text).expect("jsonl parses");
    assert_eq!(parsed, journal.events(), "sink and ring must agree");
    for (i, e) in parsed.iter().enumerate() {
        assert_eq!(e.seq, i as u64, "seq must be gapless");
    }
    parsed
}

// ---------------------------------------------------------------- STT

#[test]
fn stt_happy_path_streams_audio_and_journals_every_step() {
    let seen: SeenRef = Arc::new(Mutex::new(Seen::default()));
    let seen_server = Arc::clone(&seen);
    let (addr, done) = spawn_server(move |reader, writer| {
        loop {
            let Some(event) = decode(reader)? else { break };
            match event.type_.as_str() {
                <Transcribe as Eventable>::TYPE => {
                    let t = Transcribe::from_event(&event)?;
                    seen_server.lock().unwrap().transcribe_language = t.language;
                }
                <AudioStart as Eventable>::TYPE => {
                    let a = AudioStart::from_event(&event)?;
                    seen_server.lock().unwrap().audio_start = Some((a.rate, a.width, a.channels));
                }
                <AudioChunk as Eventable>::TYPE => {
                    let c = AudioChunk::from_event(&event)?;
                    seen_server.lock().unwrap().audio.extend_from_slice(&c.audio);
                }
                <AudioStop as Eventable>::TYPE => break,
                _ => {}
            }
        }
        let chunk = TranscriptChunk { text: "hel".into() }.to_event();
        encode_into(&chunk, writer)?;
        let final_ev = Transcript {
            text: "hello world".into(),
            language: Some("en".into()),
            context: None,
        }
        .to_event();
        encode_into(&final_ev, writer)?;
        Ok(())
    });

    let pcm = pcm16k(16000, CHUNK_SAMPLES * 2 + 952); // 3 chunks
    let expected_chunks = 3;
    let mut journal = Journal::new(Vec::<u8>::new());
    let (mut reader, mut writer) = client_pair(addr);
    let dest = addr.to_string();
    let text = run_stt(
        &mut reader,
        &mut writer,
        &mut journal,
        &pcm,
        &SttOptions { destination: &dest, source: "test.wav", language: Some("en") },
    )
    .expect("stt should succeed");
    done.recv_timeout(std::time::Duration::from_secs(5)).unwrap();

    assert_eq!(text, "hello world");
    let seen = seen.lock().unwrap();
    assert_eq!(seen.audio_start, Some((16000, 2, 1)));
    assert_eq!(seen.audio, pcm.samples, "server must receive every audio byte");
    assert_eq!(seen.transcribe_language.as_deref(), Some("en"));
    drop(seen);

    let events = assert_journal_consistent(&journal);
    assert_eq!(
        kinds(&events),
        [
            "handoff-started",
            "span-start",
            "mic-frame",
            "mic-frame",
            "mic-frame",
            "stt-partial",
            "stt-final",
            "span-end-ok",
            "handoff-completed",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect::<Vec<_>>(),
        "expected one mic-frame per audio chunk, then partial, final, close"
    );
    assert_eq!(expected_chunks, 3);

    // The final event carries the transcript and a measured latency.
    for e in &events {
        if let Payload::Op6(Op6Event::SttFinal { text, latency_us, engine, .. }) = &e.payload {
            assert_eq!(text, "hello world");
            assert!(latency_us.is_some(), "final transcript must carry latency");
            assert!(engine.contains(&dest));
        }
    }
    // All events share one trace id.
    let trace = events[0].trace_id;
    assert!(trace.is_some());
    assert!(events.iter().all(|e| e.trace_id == trace));
}

#[test]
fn stt_server_closes_mid_stream_yields_explicit_error_never_silence() {
    let (addr, done) = spawn_server(move |reader, writer| {
        loop {
            let Some(event) = decode(reader)? else { break };
            if event.type_ == <AudioStop as Eventable>::TYPE {
                break;
            }
        }
        // Send one partial, then hang up without a final transcript.
        let chunk = TranscriptChunk { text: "hel".into() }.to_event();
        encode_into(&chunk, writer)?;
        Ok(()) // returning drops the socket: clean EOF for the client
    });

    let pcm = pcm16k(16000, CHUNK_SAMPLES);
    let mut journal = Journal::new(Vec::<u8>::new());
    let (mut reader, mut writer) = client_pair(addr);
    let dest = addr.to_string();
    let result = run_stt(
        &mut reader,
        &mut writer,
        &mut journal,
        &pcm,
        &SttOptions { destination: &dest, source: "test.wav", language: None },
    );

    let err = match result {
        Err(e) => e,
        Ok(text) => panic!("must fail when the server closes early, got {text:?}"),
    };
    assert!(
        matches!(err, BridgeError::Protocol(_)) && err.to_string().contains("closed the stream"),
        "unexpected error: {err}"
    );
    done.recv_timeout(std::time::Duration::from_secs(5)).unwrap();

    let events = assert_journal_consistent(&journal);
    let k = kinds(&events);
    assert!(k.contains(&"stt-partial".to_string()), "the partial must still be journaled");
    assert!(!k.contains(&"stt-final".to_string()), "no final transcript may be invented");
    assert_eq!(
        &k[k.len() - 3..],
        &["span-end-error".to_string(), "run-error".to_string(), "handoff-failed".to_string()],
        "failure must close the span, raise RunError, and mark the handoff failed"
    );
}

#[test]
fn stt_server_error_event_surfaces_server_message() {
    let (addr, done) = spawn_server(move |reader, writer| {
        loop {
            let Some(event) = decode(reader)? else { break };
            if event.type_ == <AudioStop as Eventable>::TYPE {
                break;
            }
        }
        let err = ErrorEvent { text: "model not loaded".into(), code: Some("no-model".into()) }
            .to_event();
        encode_into(&err, writer)?;
        Ok(())
    });

    let pcm = pcm16k(16000, CHUNK_SAMPLES);
    let mut journal = Journal::new(Vec::<u8>::new());
    let (mut reader, mut writer) = client_pair(addr);
    let dest = addr.to_string();
    let result = run_stt(
        &mut reader,
        &mut writer,
        &mut journal,
        &pcm,
        &SttOptions { destination: &dest, source: "test.wav", language: None },
    );

    match result {
        Err(BridgeError::Server { message, code }) => {
            assert_eq!(message, "model not loaded");
            assert_eq!(code.as_deref(), Some("no-model"));
        }
        other => panic!("expected server error, got {other:?}"),
    }
    done.recv_timeout(std::time::Duration::from_secs(5)).unwrap();

    let events = assert_journal_consistent(&journal);
    let run_errors: Vec<&String> = events
        .iter()
        .filter_map(|e| match &e.payload {
            Payload::AgUi(AgUiEvent::RunError { message, .. }) => Some(message),
            _ => None,
        })
        .collect();
    assert_eq!(run_errors.len(), 1);
    assert!(run_errors[0].contains("model not loaded"));
    assert!(kinds(&events).contains(&"handoff-failed".to_string()));
}

// ---------------------------------------------------------------- TTS

fn temp_path(name: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("vb-bridge-{}-{name}", std::process::id()));
    p
}

#[test]
fn tts_happy_path_writes_wav_and_journals_timings() {
    let seen: SeenRef = Arc::new(Mutex::new(Seen::default()));
    let seen_server = Arc::clone(&seen);
    let (addr, done) = spawn_server(move |reader, writer| {
        let event = decode(reader)?.expect("synthesize expected");
        let synth = Synthesize::from_event(&event)?;
        {
            let mut s = seen_server.lock().unwrap();
            s.synthesize_text = Some(synth.text.clone());
            s.synthesize_voice = synth.voice.and_then(|v| v.name);
        }
        let start = AudioStart { rate: 22050, width: 2, channels: 1, timestamp: None }.to_event();
        encode_into(&start, writer)?;
        for payload in [vec![1u8; 4], vec![2u8; 4]] {
            let chunk = AudioChunk {
                rate: 22050,
                width: 2,
                channels: 1,
                audio: payload,
                timestamp: None,
            }
            .to_event();
            encode_into(&chunk, writer)?;
        }
        let stop = AudioStop { timestamp: None }.to_event();
        encode_into(&stop, writer)?;
        let stopped = SynthesizeStopped.to_event();
        encode_into(&stopped, writer)?;
        Ok(())
    });

    let out = temp_path("tts-out.wav");
    std::fs::remove_file(&out).ok();
    let mut journal = Journal::new(Vec::<u8>::new());
    let (mut reader, mut writer) = client_pair(addr);
    let dest = addr.to_string();
    let result = run_tts(
        &mut reader,
        &mut writer,
        &mut journal,
        &TtsOptions {
            destination: &dest,
            text: "turn on the lights",
            voice: Some("piper"),
            language: None,
            out_path: out.to_str().unwrap(),
        },
    )
    .expect("tts should succeed");
    done.recv_timeout(std::time::Duration::from_secs(5)).unwrap();

    assert_eq!(result.bytes, 8);
    assert_eq!(result.rate, 22050);
    let seen = seen.lock().unwrap();
    assert_eq!(seen.synthesize_text.as_deref(), Some("turn on the lights"));
    assert_eq!(seen.synthesize_voice.as_deref(), Some("piper"));
    drop(seen);

    // The WAV on disk must contain exactly the streamed audio.
    let pcm = read_pcm(&out).expect("written wav parses");
    assert_eq!(pcm.rate, 22050);
    assert_eq!(pcm.channels, 1);
    let mut expected = vec![1u8; 4];
    expected.extend_from_slice(&[2u8; 4]);
    assert_eq!(pcm.samples, expected);
    std::fs::remove_file(&out).ok();

    let events = assert_journal_consistent(&journal);
    assert_eq!(
        kinds(&events),
        ["handoff-started", "span-start", "span-end-ok", "handoff-completed"]
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
    );
}

#[test]
fn tts_server_closes_mid_audio_fails_writes_nothing_and_journals_error() {
    let (addr, done) = spawn_server(move |reader, writer| {
        let _ = decode(reader)?.expect("synthesize expected");
        let start = AudioStart { rate: 22050, width: 2, channels: 1, timestamp: None }.to_event();
        encode_into(&start, writer)?;
        let chunk = AudioChunk {
            rate: 22050,
            width: 2,
            channels: 1,
            audio: vec![9u8; 6],
            timestamp: None,
        }
        .to_event();
        encode_into(&chunk, writer)?;
        Ok(()) // drop the socket mid-stream: no audio-stop
    });

    let out = temp_path("tts-partial.wav");
    std::fs::remove_file(&out).ok();
    let mut journal = Journal::new(Vec::<u8>::new());
    let (mut reader, mut writer) = client_pair(addr);
    let dest = addr.to_string();
    let result = run_tts(
        &mut reader,
        &mut writer,
        &mut journal,
        &TtsOptions {
            destination: &dest,
            text: "hello",
            voice: None,
            language: None,
            out_path: out.to_str().unwrap(),
        },
    );

    match result {
        Err(BridgeError::Protocol(m)) => assert!(m.contains("closed the stream"), "{m}"),
        other => panic!("expected protocol error, got {other:?}"),
    }
    done.recv_timeout(std::time::Duration::from_secs(5)).unwrap();

    assert!(!out.exists(), "a truncated stream must never write a WAV");
    let events = assert_journal_consistent(&journal);
    let k = kinds(&events);
    assert!(!k.contains(&"handoff-completed".to_string()));
    assert_eq!(
        &k[k.len() - 3..],
        &["span-end-error".to_string(), "run-error".to_string(), "handoff-failed".to_string()]
    );
}

#[test]
fn tts_odd_byte_total_is_rejected_as_half_sample_corruption() {
    // A complete stream whose total is an odd byte count ends mid-sample;
    // writing it would produce a WAV our own reader (rightly) refuses.
    let (addr, done) = spawn_server(move |reader, writer| {
        let _ = decode(reader)?.expect("synthesize expected");
        let start = AudioStart { rate: 22050, width: 2, channels: 1, timestamp: None }.to_event();
        encode_into(&start, writer)?;
        for payload in [vec![1u8; 4], vec![2u8; 3]] {
            let chunk = AudioChunk {
                rate: 22050,
                width: 2,
                channels: 1,
                audio: payload,
                timestamp: None,
            }
            .to_event();
            encode_into(&chunk, writer)?;
        }
        let stop = AudioStop { timestamp: None }.to_event();
        encode_into(&stop, writer)?;
        Ok(())
    });

    let out = temp_path("tts-odd.wav");
    std::fs::remove_file(&out).ok();
    let mut journal = Journal::new(Vec::<u8>::new());
    let (mut reader, mut writer) = client_pair(addr);
    let dest = addr.to_string();
    let result = run_tts(
        &mut reader,
        &mut writer,
        &mut journal,
        &TtsOptions {
            destination: &dest,
            text: "hello",
            voice: None,
            language: None,
            out_path: out.to_str().unwrap(),
        },
    );

    match result {
        Err(BridgeError::Protocol(m)) => assert!(m.contains("whole number of 16-bit samples"), "{m}"),
        other => panic!("expected protocol error, got {other:?}"),
    }
    done.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
    assert!(!out.exists(), "corrupt audio must never be written");
    assert!(kinds(&journal.events()).contains(&"handoff-failed".to_string()));
}
