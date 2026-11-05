//! Wake word detection events (upstream `wyoming/wake.py`).

use serde_json::Map;

use crate::error::Error;
use crate::event::Event;
use crate::events::{from_data, to_map, Eventable};

/// Wake word was detected (`detection`).
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct Detection {
    /// Name of model (optional).
    pub name: Option<String>,
    /// Timestamp of audio chunk with detection (optional).
    pub timestamp: Option<u64>,
    /// Name of speaker (optional).
    pub speaker: Option<String>,
    /// Context for next interaction (optional, omitted when `None`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<Map<String, serde_json::Value>>,
}

impl Eventable for Detection {
    const TYPE: &'static str = "detection";

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

/// Wake word detection request (`detect`).
///
/// Followed by `AudioStart`, `AudioChunk`+, `AudioStop`.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct Detect {
    /// Names of models to detect (`None` = any; always serialized, may be null).
    pub names: Option<Vec<String>>,
    /// Context for next interaction (optional, omitted when `None`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<Map<String, serde_json::Value>>,
}

impl Eventable for Detect {
    const TYPE: &'static str = "detect";

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

/// Audio stream ended before wake word was detected (`not-detected`).
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct NotDetected {
    /// Context for next interaction (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<Map<String, serde_json::Value>>,
}

impl Eventable for NotDetected {
    const TYPE: &'static str = "not-detected";

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
