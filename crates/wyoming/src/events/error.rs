//! Error event (upstream `wyoming/error.py`).

use crate::error::Error;
use crate::event::Event;
use crate::events::{from_data, to_map, Eventable};

/// Error has occurred (`error`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ErrorEvent {
    /// Human-readable error message.
    pub text: String,
    /// Error code, if any (omitted when `None`, like upstream
    /// `if self.code is not None:`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}

impl Eventable for ErrorEvent {
    const TYPE: &'static str = "error";

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
