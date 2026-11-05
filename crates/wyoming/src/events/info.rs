//! Service/model information events (upstream `wyoming/info.py`).
//!
//! The upstream `Artifact` base dataclass is inlined into each concrete
//! artifact struct (Rust has no dataclass inheritance); field order matches
//! the upstream `to_dict()` output: base fields first, then the artifact's
//! own fields.

use serde_json::{Map, Value};

use crate::error::Error;
use crate::event::Event;
use crate::events::audio::AudioFormat;
use crate::events::{from_data, to_map, Eventable};

/// Request info message (`describe`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Describe;

impl Eventable for Describe {
    const TYPE: &'static str = "describe";

    fn to_event(&self) -> Event {
        Event::new(Self::TYPE)
    }

    fn from_event(event: &Event) -> Result<Self, Error> {
        if event.type_ != Self::TYPE {
            return Err(Error::InvalidField(format!(
                "expected event type \"{}\", got \"{}\"",
                Self::TYPE,
                event.type_
            )));
        }
        Ok(Describe)
    }
}

/// Select which program handles this connection (`select-program`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SelectProgram {
    /// Name of the program to use (matches a program name in the info message).
    pub name: String,
}

impl Eventable for SelectProgram {
    const TYPE: &'static str = "select-program";

    fn to_event(&self) -> Event {
        Event {
            type_: Self::TYPE.to_string(),
            data: to_map(self),
            payload: None,
        }
    }

    fn from_event(event: &Event) -> Result<Self, Error> {
        from_data(event, Self::TYPE)
    }
}

/// Attribution for an artifact.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Attribution {
    /// Who made it.
    pub name: String,
    /// Where it's from.
    pub url: String,
}

fn default_true() -> bool {
    true
}

/// Speech-to-text model.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AsrModel {
    /// Name/id of artifact.
    pub name: String,
    /// Who made the artifact and where it's from.
    pub attribution: Attribution,
    /// True if the artifact is currently installed.
    pub installed: bool,
    /// Human-readable description of the artifact (may be null).
    pub description: Option<String>,
    /// Version of the artifact (may be null).
    pub version: Option<String>,
    /// List of supported model languages.
    pub languages: Vec<String>,
}

/// Speech-to-text service.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AsrProgram {
    /// Name/id of artifact.
    pub name: String,
    /// Who made the artifact and where it's from.
    pub attribution: Attribution,
    /// True if the artifact is currently installed.
    pub installed: bool,
    /// Human-readable description of the artifact (may be null).
    pub description: Option<String>,
    /// Version of the artifact (may be null).
    pub version: Option<String>,
    /// List of available models.
    pub models: Vec<AsrModel>,
    /// True if transcript streaming events are supported.
    #[serde(default)]
    pub supports_transcript_streaming: bool,
    /// True if ASR program needs an external VAD.
    #[serde(default = "default_true")]
    pub requires_external_vad: bool,
    /// True if input audio should adjust gain automatically.
    #[serde(default = "default_true")]
    pub prefers_auto_gain_enabled: bool,
    /// True if input audio should apply noise reduction.
    #[serde(default = "default_true")]
    pub prefers_noise_reduction_enabled: bool,
}

/// Individual speaker in a multi-speaker voice.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TtsVoiceSpeaker {
    /// Name/id of speaker.
    pub name: String,
}

/// Text-to-speech voice.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TtsVoice {
    /// Name/id of artifact.
    pub name: String,
    /// Who made the artifact and where it's from.
    pub attribution: Attribution,
    /// True if the artifact is currently installed.
    pub installed: bool,
    /// Human-readable description of the artifact (may be null).
    pub description: Option<String>,
    /// Version of the artifact (may be null).
    pub version: Option<String>,
    /// List of languages available in the voice.
    pub languages: Vec<String>,
    /// List of individual speakers in the voice (may be null).
    pub speakers: Option<Vec<TtsVoiceSpeaker>>,
}

/// Text-to-speech service.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TtsProgram {
    /// Name/id of artifact.
    pub name: String,
    /// Who made the artifact and where it's from.
    pub attribution: Attribution,
    /// True if the artifact is currently installed.
    pub installed: bool,
    /// Human-readable description of the artifact (may be null).
    pub description: Option<String>,
    /// Version of the artifact (may be null).
    pub version: Option<String>,
    /// List of available voices.
    pub voices: Vec<TtsVoice>,
    /// True if synthesize text streaming events are supported.
    #[serde(default)]
    pub supports_synthesize_streaming: bool,
}

/// Intent handling model.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct HandleModel {
    /// Name/id of artifact.
    pub name: String,
    /// Who made the artifact and where it's from.
    pub attribution: Attribution,
    /// True if the artifact is currently installed.
    pub installed: bool,
    /// Human-readable description of the artifact (may be null).
    pub description: Option<String>,
    /// Version of the artifact (may be null).
    pub version: Option<String>,
    /// List of supported languages in the model.
    pub languages: Vec<String>,
}

/// Intent handling service.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct HandleProgram {
    /// Name/id of artifact.
    pub name: String,
    /// Who made the artifact and where it's from.
    pub attribution: Attribution,
    /// True if the artifact is currently installed.
    pub installed: bool,
    /// Human-readable description of the artifact (may be null).
    pub description: Option<String>,
    /// Version of the artifact (may be null).
    pub version: Option<String>,
    /// List of available models.
    pub models: Vec<HandleModel>,
    /// True if handled response streaming events are supported.
    #[serde(default)]
    pub supports_handled_streaming: bool,
    /// True if the service may execute home control actions.
    #[serde(default)]
    pub supports_home_control: bool,
}

/// Wake word detection model.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct WakeModel {
    /// Name/id of artifact.
    pub name: String,
    /// Who made the artifact and where it's from.
    pub attribution: Attribution,
    /// True if the artifact is currently installed.
    pub installed: bool,
    /// Human-readable description of the artifact (may be null).
    pub description: Option<String>,
    /// Version of the artifact (may be null).
    pub version: Option<String>,
    /// List of languages supported by the model.
    pub languages: Vec<String>,
    /// Wake up phrase used by the model (may be null).
    pub phrase: Option<String>,
}

/// Wake word detection service.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct WakeProgram {
    /// Name/id of artifact.
    pub name: String,
    /// Who made the artifact and where it's from.
    pub attribution: Attribution,
    /// True if the artifact is currently installed.
    pub installed: bool,
    /// Human-readable description of the artifact (may be null).
    pub description: Option<String>,
    /// Version of the artifact (may be null).
    pub version: Option<String>,
    /// List of available models.
    pub models: Vec<WakeModel>,
}

/// Intent recognition model.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct IntentModel {
    /// Name/id of artifact.
    pub name: String,
    /// Who made the artifact and where it's from.
    pub attribution: Attribution,
    /// True if the artifact is currently installed.
    pub installed: bool,
    /// Human-readable description of the artifact (may be null).
    pub description: Option<String>,
    /// Version of the artifact (may be null).
    pub version: Option<String>,
    /// List of supported languages in the model.
    pub languages: Vec<String>,
}

/// Intent recognition service.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct IntentProgram {
    /// Name/id of artifact.
    pub name: String,
    /// Who made the artifact and where it's from.
    pub attribution: Attribution,
    /// True if the artifact is currently installed.
    pub installed: bool,
    /// Human-readable description of the artifact (may be null).
    pub description: Option<String>,
    /// Version of the artifact (may be null).
    pub version: Option<String>,
    /// List of available models.
    pub models: Vec<IntentModel>,
}

/// Satellite information.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Satellite {
    /// Name/id of artifact.
    pub name: String,
    /// Who made the artifact and where it's from.
    pub attribution: Attribution,
    /// True if the artifact is currently installed.
    pub installed: bool,
    /// Human-readable description of the artifact (may be null).
    pub description: Option<String>,
    /// Version of the artifact (may be null).
    pub version: Option<String>,
    /// Name of the area the satellite is in (may be null).
    pub area: Option<String>,
    /// True if a local VAD will be used (may be null).
    pub has_vad: Option<bool>,
    /// Wake words that are currently being listened for (may be null).
    pub active_wake_words: Option<Vec<String>>,
    /// Maximum number of local wake words that can run simultaneously (may be null).
    pub max_active_wake_words: Option<u64>,
    /// Satellite supports remotely triggering pipeline runs (may be null).
    pub supports_trigger: Option<bool>,
}

/// Microphone information.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MicProgram {
    /// Name/id of artifact.
    pub name: String,
    /// Who made the artifact and where it's from.
    pub attribution: Attribution,
    /// True if the artifact is currently installed.
    pub installed: bool,
    /// Human-readable description of the artifact (may be null).
    pub description: Option<String>,
    /// Version of the artifact (may be null).
    pub version: Option<String>,
    /// Input audio format.
    pub mic_format: AudioFormat,
}

/// Sound output information.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SndProgram {
    /// Name/id of artifact.
    pub name: String,
    /// Who made the artifact and where it's from.
    pub attribution: Attribution,
    /// True if the artifact is currently installed.
    pub installed: bool,
    /// Human-readable description of the artifact (may be null).
    pub description: Option<String>,
    /// Version of the artifact (may be null).
    pub version: Option<String>,
    /// Output audio format.
    pub snd_format: AudioFormat,
}

/// Response to describe with information about available services (upstream
/// `Info`).
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct Info {
    /// Speech-to-text services.
    #[serde(default)]
    pub asr: Vec<AsrProgram>,
    /// Text-to-speech services.
    #[serde(default)]
    pub tts: Vec<TtsProgram>,
    /// Intent handling services.
    #[serde(default)]
    pub handle: Vec<HandleProgram>,
    /// Intent recognition services.
    #[serde(default)]
    pub intent: Vec<IntentProgram>,
    /// Wake word detection services.
    #[serde(default)]
    pub wake: Vec<WakeProgram>,
    /// Audio input services.
    #[serde(default)]
    pub mic: Vec<MicProgram>,
    /// Audio output services.
    #[serde(default)]
    pub snd: Vec<SndProgram>,
    /// Satellite information (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub satellite: Option<Satellite>,
}

impl Eventable for Info {
    const TYPE: &'static str = "info";

    fn to_event(&self) -> Event {
        Event {
            type_: Self::TYPE.to_string(),
            data: to_map(self),
            payload: None,
        }
    }

    fn from_event(event: &Event) -> Result<Self, Error> {
        from_data(event, Self::TYPE)
    }
}

/// Raw data object of an info event (helper for tests).
pub fn info_data(info: &Info) -> Map<String, Value> {
    to_map(info)
}
