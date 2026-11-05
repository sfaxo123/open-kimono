//! Intent handling events (upstream `wyoming/handle.py`).

use serde_json::{Map, Value};

use crate::error::Error;
use crate::event::Event;
use crate::events::{from_data, to_map, Eventable};

/// Result of successful intent handling (`handled`).
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct Handled {
    /// Human-readable response (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Context for next interaction (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<Map<String, Value>>,
}

impl Eventable for Handled {
    const TYPE: &'static str = "handled";

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

/// Result of failed intent handling (`not-handled`).
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct NotHandled {
    /// Human-readable response (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Context for next interaction (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<Map<String, Value>>,
}

impl Eventable for NotHandled {
    const TYPE: &'static str = "not-handled";

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

/// Start of streaming result of successful intent handling (`handled-start`).
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct HandledStart {
    /// Context for next interaction (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<Map<String, Value>>,
}

impl Eventable for HandledStart {
    const TYPE: &'static str = "handled-start";

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

/// Text chunk of streaming result (`handled-chunk`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HandledChunk {
    /// Chunk of text.
    pub text: String,
}

impl Eventable for HandledChunk {
    const TYPE: &'static str = "handled-chunk";

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

/// End of streaming result of successful intent handling (`handled-stop`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HandledStop;

impl Eventable for HandledStop {
    const TYPE: &'static str = "handled-stop";

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
        Ok(HandledStop)
    }
}
