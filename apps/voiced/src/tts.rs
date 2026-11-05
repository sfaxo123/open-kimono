//! TTS and playback. Synthesis is always journalled; playback is a separate,
//! explicit step so a quiet test can prove the audio was made without making
//! any noise (PLAN phone-safety rule: no loud audio, no surprise playback).

use std::net::TcpStream;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use trace_events::{GenAiEvent, GenAiOperation, HandoffStatus, Op6Event, Payload, SpanStatus};
use voice_bridge::tts::{run_tts, TtsOptions};
use voice_bridge::wav::read_pcm;

use crate::capture::JournalOut;
use crate::config::{Config, TtsEngine};
use crate::error::VoiceError;

fn emit(j: &mut JournalOut, trace: trace_events::TraceId, span: Option<trace_events::SpanId>, payload: Payload) {
    let _ = j.emit(trace, span, None, payload);
}

fn preview(text: &str) -> String {
    const MAX: usize = 60;
    if text.chars().count() <= MAX {
        return text.to_string();
    }
    let mut s: String = text.chars().take(MAX).collect();
    s.push('…');
    s
}

/// Synthesize `text` into `out_path`. The file is only trusted after it is
/// read back: a TTS engine that writes half a WAV must not be reported as a
/// success.
pub fn synthesize(cfg: &Config, text: &str, out_path: &str, j: &mut JournalOut) -> Result<(), VoiceError> {
    let trace = j.new_trace();
    let span = j.new_span();
    let engine = match &cfg.tts {
        TtsEngine::Wyoming { destination } => format!("wyoming-tts@{destination}"),
        TtsEngine::Cmd { .. } => "cmd-tts".to_string(),
        TtsEngine::None => "tts:none".to_string(),
    };
    emit(j, trace, None, Payload::Op6(Op6Event::Handoff { destination: engine.clone(), reason: format!("synthesize speech for \"{}\"", preview(text)), status: HandoffStatus::Started }));
    emit(j, trace, Some(span), Payload::GenAi(GenAiEvent::SpanStart { name: GenAiOperation::ExecuteTool.span_name(&engine), attributes: Default::default() }));

    let started = Instant::now();
    let result = match &cfg.tts {
        TtsEngine::None => Ok(()),
        TtsEngine::Wyoming { destination } => wyoming(destination, text, out_path, cfg, j, trace, span),
        TtsEngine::Cmd { cmd } => cmd_synth(cmd, text, out_path, j, trace, span),
    };
    match result {
        Ok(()) => {
            let bytes = std::fs::metadata(out_path).map(|m| m.len()).unwrap_or(0);
            emit(j, trace, Some(span), Payload::GenAi(GenAiEvent::SpanEnd { name: GenAiOperation::ExecuteTool.span_name(&engine), status: SpanStatus::Ok, attributes: Default::default() }));
            emit(j, trace, None, Payload::Op6(Op6Event::Handoff { destination: engine.clone(), reason: format!("{} bytes of speech written to {out_path} in {} ms", bytes, started.elapsed().as_millis()), status: HandoffStatus::Completed }));
            Ok(())
        }
        Err(e) => {
            emit(j, trace, Some(span), Payload::GenAi(GenAiEvent::SpanEnd { name: GenAiOperation::ExecuteTool.span_name(&engine), status: SpanStatus::Error, attributes: Default::default() }));
            emit(j, trace, None, Payload::Op6(Op6Event::Handoff { destination: engine.clone(), reason: e.to_string(), status: HandoffStatus::Failed }));
            Err(e)
        }
    }
}

fn wyoming(destination: &str, text: &str, out_path: &str, cfg: &Config, j: &mut JournalOut, trace: trace_events::TraceId, span: trace_events::SpanId) -> Result<(), VoiceError> {
    let engine = format!("wyoming-tts@{destination}");
    let stream = TcpStream::connect(destination).map_err(|e| VoiceError::TtsFailed { engine: engine.clone(), detail: e.to_string() })?;
    stream.set_read_timeout(Some(Duration::from_secs(60))).ok();
    stream.set_write_timeout(Some(Duration::from_secs(60))).ok();
    let mut reader = std::io::BufReader::new(stream.try_clone().expect("clone"));
    let mut writer = stream;
    let opts = TtsOptions { destination, text, voice: cfg.voice.as_deref(), language: cfg.language.as_deref(), out_path };
    run_tts(&mut reader, &mut writer, j, &opts).map_err(|e| VoiceError::TtsFailed { engine, detail: e.to_string() })?;
    let _ = (trace, span);
    Ok(())
}

fn cmd_synth(cmd: &str, text: &str, out_path: &str, j: &mut JournalOut, trace: trace_events::TraceId, span: trace_events::SpanId) -> Result<(), VoiceError> {
    let engine = "cmd-tts";
    let _ = (j, trace, span);
    let full = format!("{cmd} {out_path}");
    let mut child = Command::new("/bin/sh").arg("-c").arg(&full).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e| VoiceError::TtsFailed { engine: engine.into(), detail: format!("cannot start {cmd}: {e}") })?;
    if let Some(mut si) = child.stdin.take() {
        use std::io::Write;
        si.write_all(text.as_bytes()).ok();
    }
    let out = child.wait_with_output().map_err(|e| VoiceError::TtsFailed { engine: engine.into(), detail: e.to_string() })?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        let detail = if err.is_empty() { format!("exit {}", out.status.code().unwrap_or(-1)) } else { err };
        return Err(VoiceError::TtsFailed { engine: engine.into(), detail });
    }
    // The proof is the audio, not the exit code.
    read_pcm(out_path).map_err(|e| VoiceError::TtsFailed { engine: engine.into(), detail: format!("output is not usable audio: {e}") })?;
    Ok(())
}

/// Play the synthesized answer, or refuse out loud. In quiet mode the WAV is
/// still produced and journalled — the test asserts the bytes, not the sound.
pub fn play(cfg: &Config, wav_path: &str, j: &mut JournalOut) -> Result<(), VoiceError> {
    let trace = j.new_trace();
    if cfg.quiet {
        emit(j, trace, None, Payload::Op6(Op6Event::Handoff { destination: "playback:none".into(), reason: format!("quiet mode: {wav_path} written, not played"), status: HandoffStatus::Completed }));
        return Ok(());
    }
    let Some(cmd) = cfg.play_cmd.as_ref() else {
        emit(j, trace, None, Payload::Op6(Op6Event::Handoff { destination: "playback:none".into(), reason: "no play command configured".into(), status: HandoffStatus::Completed }));
        return Ok(());
    };
    emit(j, trace, None, Payload::Op6(Op6Event::Handoff { destination: format!("playback:{cmd}"), reason: format!("playing {wav_path}"), status: HandoffStatus::Started }));
    let full = format!("{cmd} {wav_path}");
    let out = Command::new("/bin/sh").arg("-c").arg(&full).stdout(Stdio::piped()).stderr(Stdio::piped()).output().map_err(|e| VoiceError::PlaybackRefused { reason: format!("{cmd}: {e}") })?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        let detail = if err.is_empty() { format!("exit {}", out.status.code().unwrap_or(-1)) } else { err };
        emit(j, trace, None, Payload::Op6(Op6Event::Handoff { destination: format!("playback:{cmd}"), reason: detail.clone(), status: HandoffStatus::Failed }));
        return Err(VoiceError::PlaybackRefused { reason: detail });
    }
    emit(j, trace, None, Payload::Op6(Op6Event::Handoff { destination: format!("playback:{cmd}"), reason: "playback finished".into(), status: HandoffStatus::Completed }));
    Ok(())
}
