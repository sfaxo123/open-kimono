//! A2UI v0.9 message parsing — the server→client half of the protocol.
//!
//! The message shapes are copied from the A2UI spec vendored at
//! `op6-refs/a2ui` @ 409ded2 (Apache-2.0): `specification/v0_9_1/json/
//! server_to_client.json` defines exactly four messages, each an object with
//! a required `version` (`"v0.9"` / `"v0.9.1"`) and exactly one payload key:
//! `createSurface`, `updateComponents`, `updateDataModel`, `deleteSurface`.
//! Components arrive as a **flat list with id references** (an adjacency
//! list), in any order — one of them must have `id: "root"`.
//!
//! Nothing here validates the component bodies: that is the renderer's job
//! ([`crate::scene`]), which renders what it can and shows a chip for the
//! rest. A renderer must not die on a document (VISION §5: never silent).
//!
//! Documents arrive in three shapes in the wild, all accepted:
//! a single message object, a JSON array of messages (the spec gallery's
//! `{"name": …, "messages": […]}` wrapper included), or JSONL — one message
//! per line, which is how a streaming agent sends them.

use serde_json::Value;

/// One server→client A2UI message.
#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    CreateSurface {
        surface_id: String,
        catalog_id: String,
        theme: Option<Value>,
        send_data_model: bool,
    },
    UpdateComponents {
        surface_id: String,
        components: Vec<Value>,
    },
    UpdateDataModel {
        surface_id: String,
        path: Option<String>,
        value: Option<Value>,
    },
    DeleteSurface {
        surface_id: String,
    },
}

/// A message that could not be read. `index` is the position in the document
/// (array index or JSONL line number, 0-based) when one is known.
#[derive(Clone, Debug, PartialEq)]
pub struct ParseError {
    pub index: Option<usize>,
    pub message: String,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.index {
            Some(i) => write!(f, "message {i}: {}", self.message),
            None => write!(f, "{}", self.message),
        }
    }
}

/// What a whole document yielded: the messages we could read, plus every
/// message we could not (counted and named, never dropped silently).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Doc {
    pub messages: Vec<Message>,
    pub errors: Vec<ParseError>,
}

impl Doc {
    /// Merge another document's messages/errors (streaming appends).
    pub fn extend(&mut self, other: Doc) {
        self.messages.extend(other.messages);
        self.errors.extend(other.errors);
    }
}

/// Parse a whole document: a message object, an array of messages, or the
/// gallery wrapper `{"name": …, "messages": […]}`.
pub fn parse_document(text: &str) -> Doc {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Doc { errors: vec![note("empty document")], ..Default::default() };
    }
    let value: Value = match serde_json::from_str(trimmed) {
        Ok(v) => v,
        Err(e) => return Doc { errors: vec![note(format!("invalid JSON: {e}"))], ..Default::default() },
    };
    let list: Vec<Value> = match value {
        Value::Array(items) => items,
        // The spec's example files wrap messages in a named document.
        Value::Object(map) => match map.get("messages") {
            Some(Value::Array(items)) => items.clone(),
            Some(_) => return Doc { errors: vec![note("`messages` must be an array")], ..Default::default() },
            None => vec![Value::Object(map)],
        },
        other => {
            return Doc { messages: Vec::new(), errors: vec![note(format!("expected a message object or array, got {other}"))] }
        }
    };
    collect(list)
}

/// Parse a JSONL stream: one message per non-blank line. Partial or corrupt
/// lines are counted, the rest still parse (a journal is written live).
pub fn parse_jsonl(text: &str) -> Doc {
    let mut list = Vec::new();
    let mut errors = Vec::new();
    for (i, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str(line) {
            Ok(v) => list.push(v),
            Err(e) => errors.push(ParseError { index: Some(i), message: format!("invalid JSON: {e}") }),
        }
    }
    let mut doc = collect(list);
    doc.errors.extend(errors);
    doc
}

fn collect(list: Vec<Value>) -> Doc {
    let mut doc = Doc::default();
    for (i, item) in list.iter().enumerate() {
        match parse_message(item) {
            Ok(m) => doc.messages.push(m),
            Err(message) => doc.errors.push(ParseError { index: Some(i), message }),
        }
    }
    doc
}

fn note(message: impl Into<String>) -> ParseError {
    ParseError { index: None, message: message.into() }
}

/// Read one message value. Errors are precise enough to act on: a generated
/// document is usually wrong in exactly one field.
pub fn parse_message(value: &Value) -> Result<Message, String> {
    let obj = value.as_object().ok_or("message must be a JSON object")?;

    match obj.get("version") {
        None => return Err("message is missing the required `version`".into()),
        Some(Value::String(v)) if v == "v0.9" || v == "v0.9.1" => {}
        Some(other) => {
            return Err(format!("unsupported A2UI version {other} (this renderer speaks v0.9)"))
        }
    }

    let payload_keys: Vec<&String> = obj
        .keys()
        .filter(|k| k.as_str() != "version")
        .collect();
    if payload_keys.len() != 1 {
        return Err(format!("expected exactly one payload key, got {}", payload_keys.len()));
    }
    let kind = payload_keys[0].as_str();

    let payload = obj.get(kind).and_then(Value::as_object).ok_or_else(|| {
        format!("`{kind}` must be an object")
    })?;
    let surface_id = payload
        .get("surfaceId")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("`{kind}` requires a string `surfaceId`"))?
        .to_string();

    match kind {
        "createSurface" => {
            let catalog_id = payload
                .get("catalogId")
                .and_then(Value::as_str)
                .ok_or("`createSurface` requires a string `catalogId`")?
                .to_string();
            Ok(Message::CreateSurface {
                surface_id,
                catalog_id,
                theme: payload.get("theme").cloned(),
                send_data_model: payload.get("sendDataModel").and_then(Value::as_bool).unwrap_or(false),
            })
        }
        "updateComponents" => {
            let components = payload
                .get("components")
                .and_then(Value::as_array)
                .filter(|a| !a.is_empty())
                .ok_or("`updateComponents` requires a non-empty `components` array")?
                .clone();
            for (i, c) in components.iter().enumerate() {
                if c.get("id").and_then(Value::as_str).is_none() {
                    return Err(format!("components[{i}] needs a string `id`"));
                }
                if c.get("component").and_then(Value::as_str).is_none() {
                    return Err(format!("components[{i}] needs a string `component`"));
                }
            }
            Ok(Message::UpdateComponents { surface_id, components })
        }
        "updateDataModel" => {
            let path = match payload.get("path") {
                None | Some(Value::Null) => None,
                Some(Value::String(p)) => Some(p.clone()),
                Some(other) => return Err(format!("`path` must be a string, got {other}")),
            };
            Ok(Message::UpdateDataModel { surface_id, path, value: payload.get("value").cloned() })
        }
        "deleteSurface" => Ok(Message::DeleteSurface { surface_id }),
        other => Err(format!("unknown message type `{other}`")),
    }
}
