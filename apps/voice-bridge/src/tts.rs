//! Wyoming TTS client: send text to a TTS server, collect the audio stream,
//! and journal every step.
//!
//! Wire flow (upstream `wyoming/tts.py`): client sends `synthesize`
//! {text, voice?}; the server answers `audio-start`, `audio-chunk`*,
//! `audio-stop`, then `synthesize-stopped` — or an `error`.
//!
//! There is no dedicated TTS variant in `Op6Event`, so the journal mapping
//! is: `Handoff Started/Completed/Failed` for the hand-off itself, a GenAI
//! `execute_tool` span pair carrying the machine-readable timings (total and
//! time-to-first-audio-chunk), and `AgUi RunError` on any failure. The WAV
//! file is written only after the stream completed — a failed synthesis
//! leaves no partial output behind.

use std::io::{BufRead, Write};

use trace_events::{
    genai_keys, AgUiEvent, AttrMap, AttrValue, GenAiEvent, GenAiOperation, HandoffStatus,
    Op6Event, Payload, SpanStatus,
};
use wyoming::events::audio::{AudioChunk, AudioStart};
use wyoming::events::error::ErrorEvent;
use wyoming::events::tts::{Synthesize, SynthesizeStopped, SynthesizeVoice};
use wyoming::events::Eventable;
use wyoming::{decode, encode_into};

use crate::error::BridgeError;
use crate::journal::{now_us, Journal};
use crate::wav::write_pcm;

pub struct TtsOptions<'a> {
    /// `host:port` of the TTS server (journal `destination`).
    pub destination: &'a str,
    /// Text to synthesize.
    pub text: &'a str,
    /// Voice name (optional; overrides `language`).
    pub voice: Option<&'a str>,
    /// Voice language (optional).
    pub language: Option<&'a str>,
    /// Output WAV path. Written only on complete success.
    pub out_path: &'a str,
}

#[derive(Debug)]
pub struct TtsResult {
    pub bytes: usize,
    pub rate: u32,
    pub width: u8,
    pub channels: u8,
    /// Microseconds from `synthesize` sent to the first `audio-chunk`.
    pub time_to_first_audio_us: u64,
    /// Microseconds for the whole exchange.
    pub total_us: u64,
}

/// Truncate `text` for journal `reason` strings without splitting a char.
fn preview(text: &str) -> String {
    const MAX: usize = 60;
    if text.chars().count() <= MAX {
        return text.to_string();
    }
    let mut s: String = text.chars().take(MAX).collect();
    s.push('…');
    s
}

pub fn run_tts<R: BufRead, Wr: Write, J: Write>(
    reader: &mut R,
    writer: &mut Wr,
    journal: &mut Journal<J>,
    opts: &TtsOptions,
) -> Result<TtsResult, BridgeError> {
    let trace = journal.new_trace();
    let span = journal.new_span();
    let engine = format!("wyoming-tts@{}", opts.destination);

    journal.emit(
        trace,
        None,
        None,
        Payload::Op6(Op6Event::Handoff {
            destination: engine.clone(),
            reason: format!("synthesize speech for \"{}\"", preview(opts.text)),
            status: HandoffStatus::Started,
        }),
    )?;

    let mut start_attrs: AttrMap = AttrMap::new();
    start_attrs.insert(genai_keys::TOOL_NAME.into(), AttrValue::Str("wyoming-tts".into()));
    start_attrs.insert("op6.destination".into(), AttrValue::Str(opts.destination.into()));
    if let Some(voice) = opts.voice {
        start_attrs.insert("op6.voice".into(), AttrValue::Str(voice.into()));
    }
    if let Some(lang) = opts.language {
        start_attrs.insert("op6.language".into(), AttrValue::Str(lang.into()));
    }
    journal.emit(
        trace,
        Some(span),
        None,
        Payload::GenAi(GenAiEvent::SpanStart {
            name: GenAiOperation::ExecuteTool.span_name("wyoming-tts"),
            attributes: start_attrs,
        }),
    )?;

    let t_start = now_us();

    let outcome: Result<(Vec<u8>, AudioStart, u64), BridgeError> = (|| {
        let voice = match (opts.voice, opts.language) {
            (Some(name), _) => Some(SynthesizeVoice {
                name: Some(name.to_string()),
                language: None,
                speaker: None,
            }),
            (None, Some(language)) => Some(SynthesizeVoice {
                name: None,
                language: Some(language.to_string()),
                speaker: None,
            }),
            (None, None) => None,
        };
        let event = Synthesize {
            text: opts.text.to_string(),
            voice,
            text_format: None,
            context: None,
        }
        .to_event();
        encode_into(&event, writer)?;
        writer.flush()?;

        let mut audio: Vec<u8> = Vec::new();
        let mut format: Option<AudioStart> = None;
        let mut ttfa_us: Option<u64> = None;

        loop {
            let Some(event) = decode(reader)? else {
                return Err(BridgeError::Protocol(format!(
                    "tts server {engine} closed the stream before audio-stop"
                )));
            };
            match event.type_.as_str() {
                <AudioStart as Eventable>::TYPE => {
                    if format.is_some() {
                        return Err(BridgeError::Protocol(
                            "server sent audio-start twice".into(),
                        ));
                    }
                    format = Some(AudioStart::from_event(&event)?);
                }
                <AudioChunk as Eventable>::TYPE => {
                    let chunk = AudioChunk::from_event(&event)?;
                    if ttfa_us.is_none() {
                        ttfa_us = Some(now_us().saturating_sub(t_start));
                    }
                    audio.extend_from_slice(&chunk.audio);
                }
                <wyoming::events::audio::AudioStop as Eventable>::TYPE => break,
                <ErrorEvent as Eventable>::TYPE => {
                    let err = ErrorEvent::from_event(&event)?;
                    return Err(BridgeError::Server {
                        message: err.text,
                        code: err.code,
                    });
                }
                <SynthesizeStopped as Eventable>::TYPE => {
                    // Some servers end with synthesize-stopped; only valid
                    // once audio has been delivered.
                    if format.is_none() {
                        return Err(BridgeError::Protocol(
                            "server ended synthesis before sending any audio".into(),
                        ));
                    }
                    break;
                }
                _ => {}
            }
        }

        let format = format.ok_or_else(|| {
            BridgeError::Protocol("server sent no audio-start".into())
        })?;
        if audio.is_empty() {
            return Err(BridgeError::Protocol(format!(
                "tts server {engine} returned no audio for the text"
            )));
        }
        if audio.len() % 2 != 0 {
            return Err(BridgeError::Protocol(format!(
                "tts server {engine} sent {} bytes — not a whole number of 16-bit samples",
                audio.len()
            )));
        }
        Ok((audio, format, ttfa_us.unwrap_or(0)))
    })();

    let total_us = now_us().saturating_sub(t_start);
    match outcome {
        Ok((audio, format, ttfa_us)) => {
            if format.width != 2 {
                let message = format!(
                    "tts server {engine} sent {}-bit audio; only 16-bit is supported",
                    format.width as u16 * 8
                );
                fail(journal, trace, span, engine, &message)?;
                return Err(BridgeError::Protocol(message));
            }
            // Complete stream: write the WAV, then close the journal.
            write_pcm(
                opts.out_path,
                format.rate,
                format.width,
                format.channels as u16,
                &audio,
            )?;

            let mut end_attrs: AttrMap = AttrMap::new();
            end_attrs.insert("op6.bytes".into(), AttrValue::I64(audio.len() as i64));
            end_attrs.insert("op6.time_to_first_audio_us".into(), AttrValue::I64(ttfa_us as i64));
            end_attrs.insert("op6.latency_us".into(), AttrValue::I64(total_us as i64));
            journal.emit(
                trace,
                Some(span),
                None,
                Payload::GenAi(GenAiEvent::SpanEnd {
                    name: GenAiOperation::ExecuteTool.span_name("wyoming-tts"),
                    status: SpanStatus::Ok,
                    attributes: end_attrs,
                }),
            )?;
            journal.emit(
                trace,
                None,
                None,
                Payload::Op6(Op6Event::Handoff {
                    destination: engine,
                    reason: format!(
                        "wrote {} ({} bytes, {} Hz)",
                        opts.out_path,
                        audio.len(),
                        format.rate
                    ),
                    status: HandoffStatus::Completed,
                }),
            )?;
            Ok(TtsResult {
                bytes: audio.len(),
                rate: format.rate,
                width: format.width,
                channels: format.channels,
                time_to_first_audio_us: ttfa_us,
                total_us,
            })
        }
        Err(e) => {
            let message = e.to_string();
            fail(journal, trace, span, engine, &message)?;
            Err(e)
        }
    }
}

/// Emit the failure triad: error span end, explicit RunError, failed hand-off.
fn fail<W: Write>(
    journal: &mut Journal<W>,
    trace: trace_events::TraceId,
    span: trace_events::SpanId,
    engine: String,
    message: &str,
) -> Result<(), BridgeError> {
    let mut end_attrs: AttrMap = AttrMap::new();
    end_attrs.insert("op6.error".into(), AttrValue::Str(message.to_string()));
    journal.emit(
        trace,
        Some(span),
        None,
        Payload::GenAi(GenAiEvent::SpanEnd {
            name: GenAiOperation::ExecuteTool.span_name("wyoming-tts"),
            status: SpanStatus::Error,
            attributes: end_attrs,
        }),
    )?;
    journal.emit(
        trace,
        None,
        None,
        Payload::AgUi(AgUiEvent::RunError {
            message: message.to_string(),
            code: None,
            usage: Vec::new(),
        }),
    )?;
    journal.emit(
        trace,
        None,
        None,
        Payload::Op6(Op6Event::Handoff {
            destination: engine,
            reason: message.to_string(),
            status: HandoffStatus::Failed,
        }),
    )?;
    Ok(())
}
