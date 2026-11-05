//! Voice activity detection events (upstream `wyoming/vad.py`).

use crate::error::Error;
use crate::event::Event;
use crate::events::{from_data, to_map, Eventable};

/// Voice has started (`voice-started`).
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct VoiceStarted {
    /// Milliseconds (optional).
    pub timestamp: Option<u64>,
}

impl Eventable for VoiceStarted {
    const TYPE: &'static str = "voice-started";

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

/// Voice has stopped (`voice-stopped`).
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct VoiceStopped {
    /// Milliseconds (optional).
    pub timestamp: Option<u64>,
}

impl Eventable for VoiceStopped {
    const TYPE: &'static str = "voice-stopped";

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
