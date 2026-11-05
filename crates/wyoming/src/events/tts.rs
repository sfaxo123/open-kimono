//! Text-to-speech events (upstream `wyoming/tts.py`).

use serde::de::Visitor;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};

use crate::error::Error;
use crate::event::Event;
use crate::events::{from_data, to_map, Eventable};

/// Format of text to synthesize.
///
/// Upstream is a `str` enum; unknown values are kept verbatim
/// ([`SynthesizeTextFormat::Custom`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SynthesizeTextFormat {
    Text,
    Ssml,
    /// A value outside the known vocabulary, kept as-is.
    Custom(String),
}

impl SynthesizeTextFormat {
    /// Wire string value.
    pub fn as_str(&self) -> &str {
        match self {
            SynthesizeTextFormat::Text => "text",
            SynthesizeTextFormat::Ssml => "ssml",
            SynthesizeTextFormat::Custom(s) => s,
        }
    }
}

impl Serialize for SynthesizeTextFormat {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for SynthesizeTextFormat {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = SynthesizeTextFormat;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a text format string")
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
                Ok(match v {
                    "text" => SynthesizeTextFormat::Text,
                    "ssml" => SynthesizeTextFormat::Ssml,
                    other => SynthesizeTextFormat::Custom(other.to_string()),
                })
            }
        }
        deserializer.deserialize_str(V)
    }
}

/// Desired voice for synthesis (upstream `SynthesizeVoice`).
///
/// Upstream `to_dict`/`from_dict` quirks are mirrored: `name` wins over
/// `language`, `speaker` is only serialized together with `name`, and an
/// empty voice object round-trips to `None` on the way back.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SynthesizeVoice {
    /// Voice name from tts info (overrides language).
    pub name: Option<String>,
    /// Voice language from tts info.
    pub language: Option<String>,
    /// Voice speaker from tts info.
    pub speaker: Option<String>,
}

impl SynthesizeVoice {
    /// Mirror of upstream `to_dict`.
    pub(crate) fn to_value(&self) -> Map<String, Value> {
        let mut voice = Map::new();
        if let Some(name) = &self.name {
            voice.insert("name".to_string(), Value::String(name.clone()));
            if let Some(speaker) = &self.speaker {
                voice.insert("speaker".to_string(), Value::String(speaker.clone()));
            }
        } else if let Some(language) = &self.language {
            voice.insert("language".to_string(), Value::String(language.clone()));
        }
        voice
    }

    /// Mirror of upstream `from_dict` (empty object → `None`).
    pub(crate) fn from_value(value: &Value) -> Option<SynthesizeVoice> {
        let voice = value.as_object()?;
        if let Some(name) = voice.get("name").and_then(Value::as_str) {
            return Some(SynthesizeVoice {
                name: Some(name.to_string()),
                language: None,
                speaker: voice.get("speaker").and_then(Value::as_str).map(String::from),
            });
        }
        if let Some(language) = voice.get("language").and_then(Value::as_str) {
            return Some(SynthesizeVoice {
                name: None,
                language: Some(language.to_string()),
                speaker: None,
            });
        }
        None
    }
}

/// Request to synthesize audio from text (`synthesize`).
#[derive(Debug, Clone, PartialEq)]
pub struct Synthesize {
    /// Text to synthesize.
    pub text: String,
    /// Voice to use during synthesis (optional).
    pub voice: Option<SynthesizeVoice>,
    /// Format of text (optional).
    pub text_format: Option<SynthesizeTextFormat>,
    /// Context for next interaction (optional).
    pub context: Option<Map<String, Value>>,
}

impl Eventable for Synthesize {
    const TYPE: &'static str = "synthesize";

    fn to_event(&self) -> Event {
        let mut data = Map::new();
        data.insert("text".to_string(), Value::String(self.text.clone()));
        if let Some(voice) = &self.voice {
            data.insert("voice".to_string(), Value::Object(voice.to_value()));
        }
        if let Some(fmt) = &self.text_format {
            data.insert("text_format".to_string(), Value::String(fmt.as_str().to_string()));
        }
        if let Some(context) = &self.context {
            data.insert("context".to_string(), Value::Object(context.clone()));
        }
        Event {
            type_: Self::TYPE.to_string(),
            data,
            payload: None,
        }
    }

    fn from_event(event: &Event) -> Result<Self, Error> {
        if event.type_ != Self::TYPE {
            return Err(Error::InvalidField(format!(
                "expected event type \"{}\", got \"{}\"",
                Self::TYPE,
                event.type_
            )));
        }
        Ok(Synthesize {
            text: event
                .data
                .get("text")
                .and_then(Value::as_str)
                .ok_or_else(|| Error::MissingField("text".to_string()))?
                .to_string(),
            voice: event.data.get("voice").and_then(SynthesizeVoice::from_value),
            text_format: event
                .data
                .get("text_format")
                .and_then(Value::as_str)
                .map(|s| match s {
                    "text" => SynthesizeTextFormat::Text,
                    "ssml" => SynthesizeTextFormat::Ssml,
                    other => SynthesizeTextFormat::Custom(other.to_string()),
                }),
            context: event
                .data
                .get("context")
                .and_then(Value::as_object)
                .cloned(),
        })
    }
}

/// Start of streaming request to synthesize audio from text (`synthesize-start`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SynthesizeStart {
    /// Voice to use during synthesis (optional).
    pub voice: Option<SynthesizeVoice>,
    /// Format of text (optional).
    pub text_format: Option<SynthesizeTextFormat>,
    /// Context for next interaction (optional).
    pub context: Option<Map<String, Value>>,
}

impl Eventable for SynthesizeStart {
    const TYPE: &'static str = "synthesize-start";

    fn to_event(&self) -> Event {
        let mut data = Map::new();
        if let Some(voice) = &self.voice {
            data.insert("voice".to_string(), Value::Object(voice.to_value()));
        }
        if let Some(fmt) = &self.text_format {
            data.insert("text_format".to_string(), Value::String(fmt.as_str().to_string()));
        }
        if let Some(context) = &self.context {
            data.insert("context".to_string(), Value::Object(context.clone()));
        }
        Event {
            type_: Self::TYPE.to_string(),
            data,
            payload: None,
        }
    }

    fn from_event(event: &Event) -> Result<Self, Error> {
        if event.type_ != Self::TYPE {
            return Err(Error::InvalidField(format!(
                "expected event type \"{}\", got \"{}\"",
                Self::TYPE,
                event.type_
            )));
        }
        Ok(SynthesizeStart {
            voice: event.data.get("voice").and_then(SynthesizeVoice::from_value),
            text_format: event
                .data
                .get("text_format")
                .and_then(Value::as_str)
                .map(|s| match s {
                    "text" => SynthesizeTextFormat::Text,
                    "ssml" => SynthesizeTextFormat::Ssml,
                    other => SynthesizeTextFormat::Custom(other.to_string()),
                }),
            context: event
                .data
                .get("context")
                .and_then(Value::as_object)
                .cloned(),
        })
    }
}

/// Text chunk from streaming synthesize request (`synthesize-chunk`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SynthesizeChunk {
    /// Chunk of text to synthesize.
    pub text: String,
}

impl Eventable for SynthesizeChunk {
    const TYPE: &'static str = "synthesize-chunk";

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

/// End of streaming synthesize request (`synthesize-stop`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SynthesizeStop;

impl Eventable for SynthesizeStop {
    const TYPE: &'static str = "synthesize-stop";

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
        Ok(SynthesizeStop)
    }
}

/// End of streaming synthesize response (`synthesize-stopped`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SynthesizeStopped;

impl Eventable for SynthesizeStopped {
    const TYPE: &'static str = "synthesize-stopped";

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
        Ok(SynthesizeStopped)
    }
}
