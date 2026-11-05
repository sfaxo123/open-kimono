//! Typed Wyoming events, one module per upstream Python module.
//!
//! Every typed event provides:
//! * a `TYPE` constant with the wire type string,
//! * `to_event(&self) -> Event` (mirrors Python `event()`),
//! * `from_event(&Event) -> Result<Self, Error>` (mirrors Python `from_event()`).

pub mod asr;
pub mod audio;
pub mod error;
pub mod handle;
pub mod info;
pub mod intent;
pub mod ping;
pub mod pipeline;
pub mod satellite;
pub mod snd;
pub mod timer;
pub mod tts;
pub mod user_event;
pub mod vad;
pub mod wake;

use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{Map, Value};

use crate::error::Error;
use crate::event::Event;

/// Serialize a struct into the event `data` object (field order preserved).
pub(crate) fn to_map<T: Serialize>(value: &T) -> Map<String, Value> {
    match serde_json::to_value(value) {
        Ok(Value::Object(map)) => map,
        _ => Map::new(),
    }
}

/// Check the event type and deserialize the `data` object into a struct.
pub(crate) fn from_data<T: DeserializeOwned>(event: &Event, want: &str) -> Result<T, Error> {
    if event.type_ != want {
        return Err(Error::InvalidField(format!(
            "expected event type \"{want}\", got \"{}\"",
            event.type_
        )));
    }
    serde_json::from_value(Value::Object(event.data.clone()))
        .map_err(|e| Error::InvalidField(e.to_string()))
}

/// skip_serializing_if helper: omit `None` and empty vectors (upstream
/// `if self.field:` truthiness).
pub(crate) fn is_none_or_empty<T>(value: &Option<Vec<T>>) -> bool {
    value.as_ref().map_or(true, Vec::is_empty)
}

/// Required non-negative integer field.
pub(crate) fn req_u64(data: &Map<String, Value>, key: &str) -> Result<u64, Error> {
    data.get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| Error::MissingField(key.to_string()))
}

/// Optional non-negative integer field (missing or null → `None`).
pub(crate) fn opt_u64(data: &Map<String, Value>, key: &str) -> Option<u64> {
    data.get(key).and_then(Value::as_u64)
}

/// A typed event that converts to/from a raw [`Event`] (mirrors the upstream
/// `Eventable` ABC).
pub trait Eventable: Sized {
    /// Wire type string for this event.
    const TYPE: &'static str;

    /// Convert to a raw event (Python `event()`).
    fn to_event(&self) -> Event;

    /// Convert from a raw event (Python `from_event()`).
    fn from_event(event: &Event) -> Result<Self, Error>;

    /// True if `type_` is this event's wire type (Python `is_type()`).
    fn is_type(type_: &str) -> bool {
        type_ == Self::TYPE
    }
}

/// Every known Wyoming event as a typed variant, dispatched by wire type.
#[derive(Debug, Clone, PartialEq)]
pub enum EventKind {
    AudioChunk(audio::AudioChunk),
    AudioStart(audio::AudioStart),
    AudioStop(audio::AudioStop),
    VoiceStarted(vad::VoiceStarted),
    VoiceStopped(vad::VoiceStopped),
    Ping(ping::Ping),
    Pong(ping::Pong),
    ErrorEvent(error::ErrorEvent),
    TimerStarted(timer::TimerStarted),
    TimerUpdated(timer::TimerUpdated),
    TimerCancelled(timer::TimerCancelled),
    TimerFinished(timer::TimerFinished),
    UserEvent(user_event::UserEvent),
    Played(snd::Played),
    Transcript(asr::Transcript),
    Transcribe(asr::Transcribe),
    TranscriptStart(asr::TranscriptStart),
    TranscriptChunk(asr::TranscriptChunk),
    TranscriptStop(asr::TranscriptStop),
    Detection(wake::Detection),
    Detect(wake::Detect),
    NotDetected(wake::NotDetected),
    Synthesize(tts::Synthesize),
    SynthesizeStart(tts::SynthesizeStart),
    SynthesizeChunk(tts::SynthesizeChunk),
    SynthesizeStop(tts::SynthesizeStop),
    SynthesizeStopped(tts::SynthesizeStopped),
    Recognize(intent::Recognize),
    Intent(intent::Intent),
    NotRecognized(intent::NotRecognized),
    IntentsStart(intent::IntentsStart),
    IntentsStop(intent::IntentsStop),
    Handled(handle::Handled),
    NotHandled(handle::NotHandled),
    HandledStart(handle::HandledStart),
    HandledChunk(handle::HandledChunk),
    HandledStop(handle::HandledStop),
    RunPipeline(pipeline::RunPipeline),
    RunSatellite(satellite::RunSatellite),
    PauseSatellite(satellite::PauseSatellite),
    StreamingStarted(satellite::StreamingStarted),
    StreamingStopped(satellite::StreamingStopped),
    SatelliteConnected(satellite::SatelliteConnected),
    SatelliteDisconnected(satellite::SatelliteDisconnected),
    Describe(info::Describe),
    SelectProgram(info::SelectProgram),
    Info(info::Info),
}

/// Why [`EventKind::from_event`] failed.
#[derive(Debug, Clone, PartialEq)]
pub enum EventKindError {
    /// The wire type is not part of the known vocabulary.
    UnknownType(String),
    /// The type is known but the data/payload did not fit the typed event.
    Invalid(Error),
}

impl std::fmt::Display for EventKindError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EventKindError::UnknownType(t) => write!(f, "unknown event type: {t}"),
            EventKindError::Invalid(err) => write!(f, "invalid event: {err}"),
        }
    }
}

impl EventKind {
    /// Dispatch a raw event to its typed variant by wire type.
    pub fn from_event(event: &Event) -> Result<EventKind, EventKindError> {
        use EventKind::*;
        let t = event.type_.as_str();
        macro_rules! try_kind {
            ($kind:path, $ty:path) => {
                if t == <$ty>::TYPE {
                    return <$ty>::from_event(event)
                        .map($kind)
                        .map_err(EventKindError::Invalid);
                }
            };
        }
        try_kind!(AudioChunk, audio::AudioChunk);
        try_kind!(AudioStart, audio::AudioStart);
        try_kind!(AudioStop, audio::AudioStop);
        try_kind!(VoiceStarted, vad::VoiceStarted);
        try_kind!(VoiceStopped, vad::VoiceStopped);
        try_kind!(Ping, ping::Ping);
        try_kind!(Pong, ping::Pong);
        try_kind!(ErrorEvent, error::ErrorEvent);
        try_kind!(TimerStarted, timer::TimerStarted);
        try_kind!(TimerUpdated, timer::TimerUpdated);
        try_kind!(TimerCancelled, timer::TimerCancelled);
        try_kind!(TimerFinished, timer::TimerFinished);
        try_kind!(UserEvent, user_event::UserEvent);
        try_kind!(Played, snd::Played);
        try_kind!(Transcript, asr::Transcript);
        try_kind!(Transcribe, asr::Transcribe);
        try_kind!(TranscriptStart, asr::TranscriptStart);
        try_kind!(TranscriptChunk, asr::TranscriptChunk);
        try_kind!(TranscriptStop, asr::TranscriptStop);
        try_kind!(Detection, wake::Detection);
        try_kind!(Detect, wake::Detect);
        try_kind!(NotDetected, wake::NotDetected);
        try_kind!(Synthesize, tts::Synthesize);
        try_kind!(SynthesizeStart, tts::SynthesizeStart);
        try_kind!(SynthesizeChunk, tts::SynthesizeChunk);
        try_kind!(SynthesizeStop, tts::SynthesizeStop);
        try_kind!(SynthesizeStopped, tts::SynthesizeStopped);
        try_kind!(Recognize, intent::Recognize);
        try_kind!(Intent, intent::Intent);
        try_kind!(NotRecognized, intent::NotRecognized);
        try_kind!(IntentsStart, intent::IntentsStart);
        try_kind!(IntentsStop, intent::IntentsStop);
        try_kind!(Handled, handle::Handled);
        try_kind!(NotHandled, handle::NotHandled);
        try_kind!(HandledStart, handle::HandledStart);
        try_kind!(HandledChunk, handle::HandledChunk);
        try_kind!(HandledStop, handle::HandledStop);
        try_kind!(RunPipeline, pipeline::RunPipeline);
        try_kind!(RunSatellite, satellite::RunSatellite);
        try_kind!(PauseSatellite, satellite::PauseSatellite);
        try_kind!(StreamingStarted, satellite::StreamingStarted);
        try_kind!(StreamingStopped, satellite::StreamingStopped);
        try_kind!(SatelliteConnected, satellite::SatelliteConnected);
        try_kind!(SatelliteDisconnected, satellite::SatelliteDisconnected);
        try_kind!(Describe, info::Describe);
        try_kind!(SelectProgram, info::SelectProgram);
        try_kind!(Info, info::Info);
        Err(EventKindError::UnknownType(event.type_.clone()))
    }

    /// Convert the typed variant back into a raw event.
    pub fn to_event(&self) -> Event {
        match self {
            EventKind::AudioChunk(e) => e.to_event(),
            EventKind::AudioStart(e) => e.to_event(),
            EventKind::AudioStop(e) => e.to_event(),
            EventKind::VoiceStarted(e) => e.to_event(),
            EventKind::VoiceStopped(e) => e.to_event(),
            EventKind::Ping(e) => e.to_event(),
            EventKind::Pong(e) => e.to_event(),
            EventKind::ErrorEvent(e) => e.to_event(),
            EventKind::TimerStarted(e) => e.to_event(),
            EventKind::TimerUpdated(e) => e.to_event(),
            EventKind::TimerCancelled(e) => e.to_event(),
            EventKind::TimerFinished(e) => e.to_event(),
            EventKind::UserEvent(e) => e.to_event(),
            EventKind::Played(e) => e.to_event(),
            EventKind::Transcript(e) => e.to_event(),
            EventKind::Transcribe(e) => e.to_event(),
            EventKind::TranscriptStart(e) => e.to_event(),
            EventKind::TranscriptChunk(e) => e.to_event(),
            EventKind::TranscriptStop(e) => e.to_event(),
            EventKind::Detection(e) => e.to_event(),
            EventKind::Detect(e) => e.to_event(),
            EventKind::NotDetected(e) => e.to_event(),
            EventKind::Synthesize(e) => e.to_event(),
            EventKind::SynthesizeStart(e) => e.to_event(),
            EventKind::SynthesizeChunk(e) => e.to_event(),
            EventKind::SynthesizeStop(e) => e.to_event(),
            EventKind::SynthesizeStopped(e) => e.to_event(),
            EventKind::Recognize(e) => e.to_event(),
            EventKind::Intent(e) => e.to_event(),
            EventKind::NotRecognized(e) => e.to_event(),
            EventKind::IntentsStart(e) => e.to_event(),
            EventKind::IntentsStop(e) => e.to_event(),
            EventKind::Handled(e) => e.to_event(),
            EventKind::NotHandled(e) => e.to_event(),
            EventKind::HandledStart(e) => e.to_event(),
            EventKind::HandledChunk(e) => e.to_event(),
            EventKind::HandledStop(e) => e.to_event(),
            EventKind::RunPipeline(e) => e.to_event(),
            EventKind::RunSatellite(e) => e.to_event(),
            EventKind::PauseSatellite(e) => e.to_event(),
            EventKind::StreamingStarted(e) => e.to_event(),
            EventKind::StreamingStopped(e) => e.to_event(),
            EventKind::SatelliteConnected(e) => e.to_event(),
            EventKind::SatelliteDisconnected(e) => e.to_event(),
            EventKind::Describe(e) => e.to_event(),
            EventKind::SelectProgram(e) => e.to_event(),
            EventKind::Info(e) => e.to_event(),
        }
    }
}
