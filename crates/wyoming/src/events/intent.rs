//! Intent recognition events (upstream `wyoming/intent.py`).

use serde_json::{json, Map, Value};

use crate::error::Error;
use crate::event::Event;
use crate::events::{from_data, to_map, Eventable};

/// Named entity with a value (upstream `Entity`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Entity {
    /// Entity name.
    pub name: String,
    /// Entity value (optional; always serialized, may be null).
    pub value: Option<Value>,
}

/// Request to recognize an intent from text (`recognize`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Recognize {
    /// Text with intent in natural language.
    pub text: String,
    /// Context from previous interactions (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<Map<String, Value>>,
}

impl Eventable for Recognize {
    const TYPE: &'static str = "recognize";

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

/// Result of successful intent recognition (`intent`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Intent {
    /// Name of intent.
    pub name: String,
    /// Named entities with values (omitted when empty).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entities: Vec<Entity>,
    /// Human-readable response (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Context for next interaction (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<Map<String, Value>>,
}

impl Eventable for Intent {
    const TYPE: &'static str = "intent";

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

impl Intent {
    /// Rhasspy-compatible intent payload (upstream `to_rhasspy`).
    pub fn to_rhasspy(&self) -> Value {
        json!({
            "intent": { "name": self.name },
            "entities": self.entities.iter().map(|e| json!({
                "entity": e.name,
                "value": e.value,
            })).collect::<Vec<_>>(),
            "slots": self.entities.iter().map(|e| (e.name.clone(), e.value.clone().unwrap_or(Value::Null)))
                .collect::<Map<String, Value>>(),
        })
    }
}

/// Result of intent recognition failure (`not-recognized`).
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct NotRecognized {
    /// Human-readable response (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Context for next interaction (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<Map<String, Value>>,
}

impl Eventable for NotRecognized {
    const TYPE: &'static str = "not-recognized";

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

/// Start of one or more intents (`intents-start`).
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct IntentsStart {
    /// Context for next interaction (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<Map<String, Value>>,
}

impl Eventable for IntentsStart {
    const TYPE: &'static str = "intents-start";

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

/// End of intent series (`intents-stop`).
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct IntentsStop {
    /// Context for next interaction (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<Map<String, Value>>,
}

impl Eventable for IntentsStop {
    const TYPE: &'static str = "intents-stop";

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
