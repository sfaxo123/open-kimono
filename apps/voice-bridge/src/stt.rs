//! Wyoming STT client: stream 16-bit PCM to an ASR server and journal every
//! step.
//!
//! Wire flow (upstream `wyoming/asr.py`): optional `transcribe` (only when a
//! language is given), then `audio-start` / `audio-chunk`* / `audio-stop`;
//! the server answers with `transcript-chunk`* (each replaces the previous),
//! a final `transcript`, or an `error`.
//!
//! Journal mapping:
//! - `Handoff Started/Completed/Failed` — the phone→homelab hand-off is
//!   announced and closed, never silent (VISION §5).
//! - one `MicFrame {level}` per audio chunk — the level meter the UI draws
//!   (VISION §3); the bridge stands in for the phone mic here.
//! - `SttPartial` / `SttFinal {latency_us}` — live transcript, then final.
//! - a GenAI `execute_tool` span pair around the remote call, so timings are
//!   machine-readable (`latency_us` on `SttFinal` is the human-facing copy).
//! - on any failure: `SpanEnd error` + `AgUi RunError` + `Handoff Failed`,
//!   then `Err` — an aborted stream is an explicit error event, never a
//!   silent short return.

use std::io::{BufRead, Write};

use trace_events::{
    genai_keys, AgUiEvent, AttrMap, AttrValue, GenAiEvent, GenAiOperation, HandoffStatus,
    Op6Event, Payload, SpanStatus,
};
use wyoming::events::asr::{Transcript, TranscriptChunk, Transcribe};
use wyoming::events::audio::{AudioChunk, AudioStart, AudioStop};
use wyoming::events::error::ErrorEvent;
use wyoming::events::Eventable;
use wyoming::{decode, encode_into};

use crate::error::BridgeError;
use crate::journal::{now_us, Journal};
use crate::wav::{chunk_rms_level, Pcm};

/// Samples per `audio-chunk` frame (matches the wyoming Python default).
pub const CHUNK_SAMPLES: usize = 1024;

/// Encode one typed event onto the wire and flush. (`Eventable::from_event`
/// makes the trait not dyn-compatible, so this is generic, not `&dyn`.)
fn send_ev<Wr: Write, E: Eventable>(writer: &mut Wr, ev: &E) -> Result<(), BridgeError> {
    let event = ev.to_event();
    encode_into(&event, writer)?;
    writer.flush()?;
    Ok(())
}

pub struct SttOptions<'a> {
    /// `host:port` of the STT server (journal `destination`).
    pub destination: &'a str,
    /// Human label for the audio source, e.g. the WAV path (journal `reason`).
    pub source: &'a str,
    /// Spoken language, sent as a `transcribe` preface when set.
    pub language: Option<&'a str>,
}

/// Stream `pcm` to the server behind `reader`/`writer` and return the final
/// transcript. Every step lands in `journal`.
pub fn run_stt<R: BufRead, Wr: Write, J: Write>(
    reader: &mut R,
    writer: &mut Wr,
    journal: &mut Journal<J>,
    pcm: &Pcm,
    opts: &SttOptions,
) -> Result<String, BridgeError> {
    let trace = journal.new_trace();
    let span = journal.new_span();
    let engine = format!("wyoming-stt@{}", opts.destination);

    journal.emit(
        trace,
        None,
        None,
        Payload::Op6(Op6Event::Handoff {
            destination: engine.clone(),
            reason: format!("transcribe {} via wyoming stt", opts.source),
            status: HandoffStatus::Started,
        }),
    )?;

    let mut start_attrs: AttrMap = AttrMap::new();
    start_attrs.insert(genai_keys::TOOL_NAME.into(), AttrValue::Str("wyoming-stt".into()));
    start_attrs.insert("op6.destination".into(), AttrValue::Str(opts.destination.into()));
    start_attrs.insert("op6.audio_rate".into(), AttrValue::I64(pcm.rate as i64));
    start_attrs.insert("op6.audio_channels".into(), AttrValue::I64(pcm.channels as i64));
    if let Some(lang) = opts.language {
        start_attrs.insert("op6.language".into(), AttrValue::Str(lang.into()));
    }
    journal.emit(
        trace,
        Some(span),
        None,
        Payload::GenAi(GenAiEvent::SpanStart {
            name: GenAiOperation::ExecuteTool.span_name("wyoming-stt"),
            attributes: start_attrs,
        }),
    )?;

    let t_start = now_us();
    let mut chunks_sent = 0u64;

    let outcome: Result<String, BridgeError> = (|| {
        if let Some(language) = opts.language {
            send_ev(
                writer,
                &Transcribe {
                    language: Some(language.to_string()),
                    ..Transcribe::default()
                },
            )?;
        }
        send_ev(
            writer,
            &AudioStart {
                rate: pcm.rate,
                width: pcm.width,
                channels: pcm.channels as u8,
                timestamp: None,
            },
        )?;
        for chunk in pcm.samples.chunks(CHUNK_SAMPLES * pcm.width as usize) {
            send_ev(
                writer,
                &AudioChunk {
                    rate: pcm.rate,
                    width: pcm.width,
                    channels: pcm.channels as u8,
                    audio: chunk.to_vec(),
                    timestamp: None,
                },
            )?;
            chunks_sent += 1;
            journal.emit(
                trace,
                None,
                None,
                Payload::Op6(Op6Event::MicFrame {
                    level: chunk_rms_level(chunk),
                }),
            )?;
        }
        send_ev(writer, &AudioStop { timestamp: None })?;
        let t_stop_sent = now_us();

        // Read until the final transcript (or an error / closed stream).
        loop {
            let Some(event) = decode(reader)? else {
                return Err(BridgeError::Protocol(format!(
                    "stt server {engine} closed the stream before sending a transcript"
                )));
            };
            match event.type_.as_str() {
                <TranscriptChunk as Eventable>::TYPE => {
                    let tc = TranscriptChunk::from_event(&event)?;
                    journal.emit(
                        trace,
                        Some(span),
                        None,
                        Payload::Op6(Op6Event::SttPartial {
                            engine: engine.clone(),
                            text: tc.text,
                            confidence: None,
                        }),
                    )?;
                }
                <Transcript as Eventable>::TYPE => {
                    let tr = Transcript::from_event(&event)?;
                    let latency_us = now_us().saturating_sub(t_stop_sent);
                    journal.emit(
                        trace,
                        Some(span),
                        None,
                        Payload::Op6(Op6Event::SttFinal {
                            engine: engine.clone(),
                            text: tr.text.clone(),
                            confidence: None,
                            latency_us: Some(latency_us),
                        }),
                    )?;
                    return Ok(tr.text);
                }
                <ErrorEvent as Eventable>::TYPE => {
                    let err = ErrorEvent::from_event(&event)?;
                    return Err(BridgeError::Server {
                        message: err.text,
                        code: err.code,
                    });
                }
                // transcript-start / transcript-stop and unknown future
                // types are ignored, like the Python client.
                _ => {}
            }
        }
    })();

    let total_us = now_us().saturating_sub(t_start);
    match outcome {
        Ok(text) => {
            let mut end_attrs: AttrMap = AttrMap::new();
            end_attrs.insert("op6.latency_us".into(), AttrValue::I64(total_us as i64));
            end_attrs.insert("op6.audio_chunks".into(), AttrValue::I64(chunks_sent as i64));
            end_attrs.insert("op6.audio_bytes".into(), AttrValue::I64(pcm.samples.len() as i64));
            journal.emit(
                trace,
                Some(span),
                None,
                Payload::GenAi(GenAiEvent::SpanEnd {
                    name: GenAiOperation::ExecuteTool.span_name("wyoming-stt"),
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
                    reason: format!("transcript received ({} chars)", text.chars().count()),
                    status: HandoffStatus::Completed,
                }),
            )?;
            Ok(text)
        }
        Err(e) => {
            let message = e.to_string();
            let mut end_attrs: AttrMap = AttrMap::new();
            end_attrs.insert("op6.error".into(), AttrValue::Str(message.clone()));
            journal.emit(
                trace,
                Some(span),
                None,
                Payload::GenAi(GenAiEvent::SpanEnd {
                    name: GenAiOperation::ExecuteTool.span_name("wyoming-stt"),
                    status: SpanStatus::Error,
                    attributes: end_attrs,
                }),
            )?;
            journal.emit(
                trace,
                None,
                None,
                Payload::AgUi(AgUiEvent::RunError {
                    message: message.clone(),
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
                    reason: message,
                    status: HandoffStatus::Failed,
                }),
            )?;
            Err(e)
        }
    }
}
