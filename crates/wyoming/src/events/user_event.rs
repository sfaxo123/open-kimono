//! User-defined event (upstream `wyoming/user_event.py`).

use serde_json::Map;

use crate::error::Error;
use crate::event::Event;
use crate::events::{from_data, to_map, Eventable};

/// User-defined event (`user-event`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct UserEvent {
    /// Name of user event type.
    pub name: String,
    /// User event data (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Map<String, serde_json::Value>>,
    /// Context from previous interactions (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<Map<String, serde_json::Value>>,
}

impl Eventable for UserEvent {
    const TYPE: &'static str = "user-event";

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
