//! STT engines. Wyoming when a Wyoming server exists; otherwise a thin
//! adapter to whatever the phone actually installs (PLAN A4). Both paths
//! journal the same way: an announced hand-off, a `execute_tool` span, and a
//! final transcript — or a named failure.

use std::io::{BufReader, Write};
use std::net::TcpStream;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use trace_events::{AgUiEvent, GenAiEvent, GenAiOperation, HandoffStatus, Op6Event, Payload, SpanStatus};
use voice_bridge::stt::{run_stt, SttOptions};
use voice_bridge::wav::{chunk_rms_level, write_pcm, Pcm};

use crate::capture::JournalOut;
use crate::config::{Config, SttEngine};
use crate::error::VoiceError;

fn emit(j: &mut JournalOut, trace: trace_events::TraceId, span: Option<trace_events::SpanId>, payload: Payload) -> Result<(), VoiceError> {
    j.emit(trace, span, None, payload).map_err(|e| VoiceError::Io { op: "journal".into(), detail: e.to_string() })
}

/// Announce the hand-off and open the tool span, the way voice-bridge does.
fn begin(j: &mut JournalOut, engine: &str, reason: &str) -> (trace_events::TraceId, trace_events::SpanId) {
    let trace = j.new_trace();
    let span = j.new_span();
    let _ = emit(j, trace, None, Payload::Op6(Op6Event::Handoff { destination: engine.into(), reason: reason.into(), status: HandoffStatus::Started }));
    let _ = emit(j, trace, Some(span), Payload::GenAi(GenAiEvent::SpanStart { name: GenAiOperation::ExecuteTool.span_name(engine), attributes: Default::default() }));
    (trace, span)
}

fn fail(j: &mut JournalOut, trace: trace_events::TraceId, span: trace_events::SpanId, engine: &str, detail: &str) {
    let _ = emit(j, trace, Some(span), Payload::GenAi(GenAiEvent::SpanEnd { name: GenAiOperation::ExecuteTool.span_name(engine), status: SpanStatus::Error, attributes: Default::default() }));
    let _ = emit(j, trace, None, Payload::AgUi(AgUiEvent::RunError { message: format!("{engine}: {detail}"), code: Some("stt_failed".into()), usage: vec![] }));
    let _ = emit(j, trace, None, Payload::Op6(Op6Event::Handoff { destination: engine.into(), reason: detail.into(), status: HandoffStatus::Failed }));
}

fn finish(j: &mut JournalOut, trace: trace_events::TraceId, span: trace_events::SpanId, engine: &str, text: &str, latency_us: u64, confidence: Option<f32>) -> Result<(), VoiceError> {
    let _ = emit(j, trace, Some(span), Payload::GenAi(GenAiEvent::SpanEnd { name: GenAiOperation::ExecuteTool.span_name(engine), status: SpanStatus::Ok, attributes: Default::default() }));
    emit(j, trace, None, Payload::Op6(Op6Event::SttFinal { engine: engine.into(), text: text.into(), confidence, latency_us: Some(latency_us) }))?;
    emit(j, trace, None, Payload::Op6(Op6Event::Handoff { destination: engine.into(), reason: "transcript delivered".into(), status: HandoffStatus::Completed }))
}

/// Pipe the audio to a command on stdin; the transcript comes back on stdout.
/// Exit 3 means silence, matching the phone's existing STT driver contract.
fn run_cmd(cmd: &str, pcm: &Pcm, engine: &str, j: &mut JournalOut) -> Result<(String, Option<f32>), VoiceError> {
    let trace_span = begin(j, engine, "transcribe captured audio");
    let (trace, span) = trace_span;
    let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0);
    let tmp = std::env::temp_dir().join(format!("voiced-stt-{}-{nonce}.wav", std::process::id()));
    write_pcm(&tmp, pcm.rate, pcm.width, pcm.channels, &pcm.samples)
        .map_err(|e| {
            fail(j, trace, span, engine, &format!("cannot stage audio: {e}"));
            VoiceError::SttUnavailable { engine: engine.into(), detail: format!("cannot stage audio: {e}") }
        })?;
    let wav = std::fs::read(&tmp).unwrap_or_default();
    let started = Instant::now();

    let mut child = match Command::new("/bin/sh").arg("-c").arg(cmd).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn() {
        Ok(c) => c,
        Err(e) => {
            let detail = format!("cannot start {cmd}: {e}");
            fail(j, trace, span, engine, &detail);
            return Err(VoiceError::SttUnavailable { engine: engine.into(), detail });
        }
    };
    child.stdin.take().expect("piped stdin").write_all(&wav).ok();
    let out = child.wait_with_output().map_err(|e| {
        let detail = e.to_string();
        fail(j, trace, span, engine, &detail);
        VoiceError::SttUnavailable { engine: engine.into(), detail }
    })?;
    let _ = std::fs::remove_file(&tmp);

    if out.status.code() == Some(3) {
        fail(j, trace, span, engine, "silence: empty transcript");
        return Err(VoiceError::EmptyTranscript { engine: engine.into() });
    }
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        let detail = if err.is_empty() { format!("exit {}", out.status.code().unwrap_or(-1)) } else { err };
        fail(j, trace, span, engine, &detail);
        return Err(VoiceError::SttUnavailable { engine: engine.into(), detail });
    }
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if text.is_empty() {
        fail(j, trace, span, engine, "empty transcript");
        return Err(VoiceError::EmptyTranscript { engine: engine.into() });
    }
    let level = chunk_rms_level(&pcm.samples);
    let confidence = if level > 0.05 { Some(0.8) } else { Some(0.4) };
    finish(j, trace, span, engine, &text, started.elapsed().as_micros() as u64, confidence)?;
    Ok((text, confidence))
}

fn run_wyoming(destination: &str, pcm: &Pcm, cfg: &Config, j: &mut JournalOut) -> Result<(String, Option<f32>), VoiceError> {
    let engine = format!("wyoming-stt@{destination}");
    let started = Instant::now();
    let stream = TcpStream::connect(destination).map_err(|e| {
        let trace = j.new_trace();
        let _ = emit(j, trace, None, Payload::Op6(Op6Event::Handoff { destination: engine.clone(), reason: format!("connect failed: {e}"), status: HandoffStatus::Failed }));
        VoiceError::SttUnavailable { engine: engine.clone(), detail: e.to_string() }
    })?;
    stream.set_read_timeout(Some(Duration::from_secs(30))).ok();
    stream.set_write_timeout(Some(Duration::from_secs(30))).ok();
    let mut reader = BufReader::new(stream.try_clone().expect("clone"));
    let mut writer = stream;
    let opts = SttOptions { destination, source: "voiced capture", language: cfg.language.as_deref() };
    // The Wyoming contract carries no confidence: the turn reports
    // no signal rather than inventing one (CV2 strata treat an
    // absent signal as "behave as before", never as doubt).
    run_stt(&mut reader, &mut writer, j, pcm, &opts).map_err(|e| VoiceError::SttUnavailable { engine, detail: e.to_string() }).map(|t| {
        let _ = started;
        (t, None)
    })
}

/// Transcribe with the configured engine. The stub exists so the pipeline can
/// be exercised on the phone without a live microphone.
///
/// Returns the transcript and the confidence the turn may report to
/// agentd (CV2). HONESTLY COARSE: for the command engine the value
/// is an acoustic-level proxy — the whole utterance's RMS mapped to
/// 0.8 (clearly audible) or 0.4 (quiet) — not a decoder score; the
/// Moonshine driver contract exposes no confidence at all. Engines
/// with no value report `None`, which agentd treats as "no signal",
/// never as doubt.
pub fn transcribe(cfg: &Config, pcm: &Pcm, j: &mut JournalOut) -> Result<(String, Option<f32>), VoiceError> {
    match &cfg.stt {
        SttEngine::Wyoming { destination } => run_wyoming(destination, pcm, cfg, j),
        SttEngine::Cmd { cmd } => run_cmd(cmd, pcm, "cmd-stt", j),
        SttEngine::Stub { text } => {
            let (trace, span) = begin(j, "stub-stt", "test transcript, no engine contacted");
            if text.trim().is_empty() {
                fail(j, trace, span, "stub-stt", "empty transcript");
                return Err(VoiceError::EmptyTranscript { engine: "stub-stt".into() });
            }
            finish(j, trace, span, "stub-stt", text, 0, Some(1.0))?;
            Ok((text.clone(), Some(1.0)))
        }
    }
}

/// Optional raw→cleaned text step, journalled as `text_cleaned` (VISION §3:
/// the user sees what was heard and what was understood).
pub fn clean(cfg: &Config, raw: &str, j: &mut JournalOut) -> String {
    let Some(cmd) = cfg.clean_cmd.as_ref() else { return raw.to_string() };
    let engine = "cmd-clean";
    let (trace, _span) = begin(j, engine, "clean the transcript");
    let spawned = Command::new("/bin/sh").arg("-c").arg(cmd).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn();
    let out = spawned.and_then(|mut c| {
        if let Some(mut si) = c.stdin.take() {
            si.write_all(raw.as_bytes()).ok();
        }
        c.wait_with_output()
    });
    match out {
        Ok(o) if o.status.success() => {
            let cleaned = String::from_utf8_lossy(&o.stdout).trim().to_string();
            let cleaned = if cleaned.is_empty() { raw.to_string() } else { cleaned };
            let _ = emit(j, trace, None, Payload::Op6(Op6Event::TextCleaned { raw: raw.into(), cleaned: cleaned.clone() }));
            cleaned
        }
        Ok(o) => {
            let detail = String::from_utf8_lossy(&o.stderr).trim().to_string();
            let _ = emit(j, trace, None, Payload::Op6(Op6Event::Handoff { destination: engine.into(), reason: format!("cleaner failed: {}", if detail.is_empty() { "non-zero exit".into() } else { detail }), status: HandoffStatus::Failed }));
            raw.to_string()
        }
        Err(e) => {
            let _ = emit(j, trace, None, Payload::Op6(Op6Event::Handoff { destination: engine.into(), reason: format!("cleaner failed: {e}"), status: HandoffStatus::Failed }));
            raw.to_string()
        }
    }
}
