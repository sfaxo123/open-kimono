//! Core Wyoming event type and stream framing.
//!
//! Wire format (from the upstream README, adapted in this port):
//!
//! ```text
//! { "type": "...", "version": "...", "data_length": ..., "payload_length": ... }\n
//! <data_length bytes (optional)>
//! <payload_length bytes (optional)>
//! ```
//!
//! 1. A JSON object header as a single `\n`-terminated line (UTF-8).
//!    * `type` — event type (string, required)
//!    * `data` — event data (object, optional, inline in the header)
//!    * `data_length` — byte length of an additional data block (int, optional)
//!    * `payload_length` — byte length of a binary payload block (int, optional)
//! 2. Optional additional data: exactly `data_length` bytes of a JSON object
//!    that is merged on top of the inline `data` object (shallow merge, the
//!    additional block wins on key collisions).
//! 3. Optional payload: exactly `payload_length` bytes of binary data
//!    (typically PCM audio).
//!
//! This mirrors `wyoming/event.py` (MIT, commit 08a0c25) of the Python
//! reference implementation. The encoder is byte-identical to the reference:
//! JSON is written the way Python's `json.dumps(..., ensure_ascii=False)`
//! writes it (default `", "` / `": "` separators, non-ASCII passed through).

use serde_json::{Map, Value};

use crate::error::Error;

/// Version written into the `version` field of every header line.
///
/// Matches the upstream Python package version of the reference commit this
/// port was made against. The field is informational; readers must not
/// reject frames based on it.
pub const WYOMING_VERSION: &str = "1.11.0";

/// A Wyoming event: a type string, a JSON data object, and an optional
/// binary payload.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Event {
    /// Event type, e.g. `"audio-chunk"`.
    pub type_: String,
    /// Event data as a JSON object.
    pub data: Map<String, Value>,
    /// Optional binary payload (typically raw PCM audio).
    pub payload: Option<Vec<u8>>,
}

impl Event {
    /// Create an event with the given type and empty data/payload.
    pub fn new(type_: impl Into<String>) -> Self {
        Event {
            type_: type_.into(),
            data: Map::new(),
            payload: None,
        }
    }

    /// Builder: attach a data object.
    pub fn with_data(mut self, data: Map<String, Value>) -> Self {
        self.data = data;
        self
    }

    /// Builder: attach a binary payload.
    pub fn with_payload(mut self, payload: impl Into<Vec<u8>>) -> Self {
        self.payload = Some(payload.into());
        self
    }

    /// Get a field from the event's data object.
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.data.get(key)
    }
}

/// Serialize an event into one complete Wyoming frame.
///
/// Like the Python reference, an empty `data` object and an empty payload are
/// omitted (no `data_length`/`payload_length` keys, no blocks).
pub fn encode(event: &Event) -> Vec<u8> {
    let mut out = Vec::new();
    encode_into(event, &mut out).expect("writing to Vec never fails");
    out
}

/// Serialize an event into any [`std::io::Write`] sink.
pub fn encode_into<W: std::io::Write>(event: &Event, writer: &mut W) -> Result<(), Error> {
    // Header keys are inserted in the same order as the Python reference
    // (serde_json's preserve_order feature keeps insertion order).
    let mut header = Map::new();
    header.insert("type".to_string(), Value::String(event.type_.clone()));
    header.insert("version".to_string(), Value::String(WYOMING_VERSION.to_string()));

    // Additional data block: only when data is non-empty.
    let data_bytes: Option<Vec<u8>> = if event.data.is_empty() {
        None
    } else {
        let mut buf = String::new();
        dumps_python(&Value::Object(event.data.clone()), &mut buf);
        Some(buf.into_bytes())
    };
    if let Some(bytes) = &data_bytes {
        header.insert(
            "data_length".to_string(),
            Value::from(bytes.len() as u64),
        );
    }

    // Payload block: only when payload is non-empty.
    let payload: Option<&[u8]> = match &event.payload {
        Some(p) if !p.is_empty() => Some(p.as_slice()),
        _ => None,
    };
    if let Some(payload) = payload {
        header.insert(
            "payload_length".to_string(),
            Value::from(payload.len() as u64),
        );
    }

    let mut header_buf = String::new();
    dumps_python(&Value::Object(header), &mut header_buf);
    let header_line = header_buf.into_bytes();
    writer.write_all(&header_line)?;
    writer.write_all(b"\n")?;
    if let Some(bytes) = &data_bytes {
        writer.write_all(bytes)?;
    }
    if let Some(payload) = payload {
        writer.write_all(payload)?;
    }
    writer.flush()?;
    Ok(())
}

/// Serialize a JSON value exactly like Python's
/// `json.dumps(value, ensure_ascii=False)` with the default separators:
/// `", "` between items, `": "` between key and value, non-ASCII characters
/// written raw UTF-8, and the same string escapes (`\"`, `\\`, `\b`, `\f`,
/// `\n`, `\r`, `\t`, `\u00xx` for other control characters).
///
/// The reference `write_event` uses `json.dumps` defaults, so matching this
/// makes the Rust encoder byte-identical to the Python one.
fn dumps_python(value: &Value, out: &mut String) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => out.push_str(&n.to_string()),
        Value::String(s) => escape_python(s, out),
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                dumps_python(item, out);
            }
            out.push(']');
        }
        Value::Object(map) => {
            out.push('{');
            for (i, (key, val)) in map.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                escape_python(key, out);
                out.push_str(": ");
                dumps_python(val, out);
            }
            out.push('}');
        }
    }
}

/// Escape a string the way Python's `json.dumps(..., ensure_ascii=False)`
/// does: only `"`, `\`, and control characters are escaped; everything else
/// (including DEL and non-ASCII) is written raw.
fn escape_python(s: &str, out: &mut String) {
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Read one Wyoming frame from a buffered stream.
///
/// Returns `Ok(None)` on a clean end of stream (no bytes before EOF, exactly
/// like the Python reference returning `None`). A partial frame, malformed
/// JSON, or a header without a string `"type"` is an error.
pub fn decode<R: std::io::BufRead>(reader: &mut R) -> Result<Option<Event>, Error> {
    let mut line = Vec::new();
    let read = reader.read_until(b'\n', &mut line)?;
    if read == 0 {
        return Ok(None);
    }
    if line.last() != Some(&b'\n') {
        // Stream ended mid-line.
        return Err(Error::UnexpectedEof);
    }
    line.pop(); // trailing \n

    let header: Value = serde_json::from_slice(&line)
        .map_err(|e| Error::InvalidHeader(e.to_string()))?;
    let header = match header {
        Value::Object(map) => map,
        _ => return Err(Error::InvalidHeader("header line is not a JSON object".to_string())),
    };

    let type_ = match header.get("type") {
        Some(Value::String(s)) => s.clone(),
        _ => return Err(Error::MissingType),
    };

    // Inline data object (may be overwritten by the additional data block).
    let mut data = match header.get("data") {
        Some(Value::Object(map)) => map.clone(),
        Some(_) => return Err(Error::InvalidData("inline \"data\" is not an object".to_string())),
        None => Map::new(),
    };

    // Additional data block, merged on top of the inline data.
    let data_length = length_field(&header, "data_length")?;
    if data_length > 0 {
        let mut bytes = vec![0u8; data_length];
        read_exact(reader, &mut bytes)?;
        let extra: Value = serde_json::from_slice(&bytes)
            .map_err(|e| Error::InvalidData(e.to_string()))?;
        match extra {
            Value::Object(map) => {
                for (key, value) in map {
                    data.insert(key, value);
                }
            }
            _ => return Err(Error::InvalidData("additional data is not an object".to_string())),
        }
    }

    // Binary payload block.
    let payload_length = length_field(&header, "payload_length")?;
    let payload = if payload_length > 0 {
        let mut bytes = vec![0u8; payload_length];
        read_exact(reader, &mut bytes)?;
        Some(bytes)
    } else {
        None
    };

    Ok(Some(Event { type_, data, payload }))
}

/// Read exactly `buf.len()` bytes or fail with [`Error::UnexpectedEof`].
fn read_exact<R: std::io::BufRead>(reader: &mut R, buf: &mut [u8]) -> Result<(), Error> {
    let mut offset = 0;
    while offset < buf.len() {
        let filled = {
            let available = reader.fill_buf()?;
            if available.is_empty() {
                return Err(Error::UnexpectedEof);
            }
            let take = std::cmp::min(available.len(), buf.len() - offset);
            buf[offset..offset + take].copy_from_slice(&available[..take]);
            take
        };
        reader.consume(filled);
        offset += filled;
    }
    Ok(())
}

/// Read a non-negative integer length field from the header.
fn length_field(header: &Map<String, Value>, key: &str) -> Result<usize, Error> {
    match header.get(key) {
        None => Ok(0),
        Some(Value::Number(n)) => n
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .ok_or_else(|| Error::InvalidHeader(format!("\"{key}\" is not a non-negative integer"))),
        Some(_) => Err(Error::InvalidHeader(format!(
            "\"{key}\" is not a non-negative integer"
        ))),
    }
}
