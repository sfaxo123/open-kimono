//! Ping/pong keepalive events (upstream `wyoming/ping.py`).

use crate::error::Error;
use crate::event::Event;
use crate::events::{from_data, to_map, Eventable};

/// Request pong message (`ping`).
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct Ping {
    /// Text to copy to response (optional).
    pub text: Option<String>,
}

impl Eventable for Ping {
    const TYPE: &'static str = "ping";

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

/// Response to ping message (`pong`).
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct Pong {
    /// Text copied from request (optional).
    pub text: Option<String>,
}

impl Eventable for Pong {
    const TYPE: &'static str = "pong";

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
