//! Speech-to-text events (upstream `wyoming/asr.py`).

use serde::de::Visitor;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Map;

use crate::error::Error;
use crate::event::Event;
use crate::events::{from_data, to_map, Eventable};

/// How quickly the end of a voice command is detected.
///
/// Upstream is a `str` enum; unknown values are kept verbatim
/// ([`VadSensitivity::Custom`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VadSensitivity {
    Default,
    Relaxed,
    Aggressive,
    /// A value outside the known vocabulary, kept as-is.
    Custom(String),
}

impl VadSensitivity {
    /// Wire string value.
    pub fn as_str(&self) -> &str {
        match self {
            VadSensitivity::Default => "default",
            VadSensitivity::Relaxed => "relaxed",
            VadSensitivity::Aggressive => "aggressive",
            VadSensitivity::Custom(s) => s,
        }
    }
}

impl Serialize for VadSensitivity {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for VadSensitivity {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = VadSensitivity;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a vad sensitivity string")
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
                Ok(match v {
                    "default" => VadSensitivity::Default,
                    "relaxed" => VadSensitivity::Relaxed,
                    "aggressive" => VadSensitivity::Aggressive,
                    other => VadSensitivity::Custom(other.to_string()),
                })
            }
        }
        deserializer.deserialize_str(V)
    }
}

/// Transcription response from ASR system (`transcript`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Transcript {
    /// Text transcription of spoken audio.
    pub text: String,
    /// Language of the text (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// Context for next interaction (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<Map<String, serde_json::Value>>,
}

impl Eventable for Transcript {
    const TYPE: &'static str = "transcript";

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

/// Transcription request to ASR system (`transcribe`).
///
/// Followed by `AudioStart`, `AudioChunk`+, `AudioStop`.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct Transcribe {
    /// Name of ASR model to use (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Language of spoken audio to follow (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// Context from previous interactions (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<Map<String, serde_json::Value>>,
    /// How quickly the end of a voice command is detected (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vad_sensitivity: Option<VadSensitivity>,
    /// Unique names that should bias the ASR transcription (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transcript_names: Option<Vec<String>>,
    /// Domain-specific terms that should bias the ASR transcription (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transcript_terms: Option<Vec<String>>,
}

impl Eventable for Transcribe {
    const TYPE: &'static str = "transcribe";

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

/// Streaming transcription response from ASR system (`transcript-start`).
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct TranscriptStart {
    /// Language of the text (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// Context for next interaction (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<Map<String, serde_json::Value>>,
}

impl Eventable for TranscriptStart {
    const TYPE: &'static str = "transcript-start";

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

/// Streaming transcription chunk (`transcript-chunk`).
///
/// Each chunk replaces the previous one instead of being appended to it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TranscriptChunk {
    /// Transcript so far.
    pub text: String,
}

impl Eventable for TranscriptChunk {
    const TYPE: &'static str = "transcript-chunk";

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

/// End of streaming transcription response (`transcript-stop`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TranscriptStop;

impl Eventable for TranscriptStop {
    const TYPE: &'static str = "transcript-stop";

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
        Ok(TranscriptStop)
    }
}
