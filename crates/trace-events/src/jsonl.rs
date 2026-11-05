//! JSONL transport: one JSON object per journal line.
//!
//! Same line shape as the `mmio-replay` traces (one flat object per line), so
//! the same tailing tools work on both. This is what the `trace:` scheme
//! serves and what `docs/workbench/` evidence files store.

use alloc::string::String;
use alloc::vec::Vec;

use crate::envelope::Event;

/// Serialize one event to a single JSON line (no trailing newline).
pub fn to_json_line(event: &Event) -> Result<String, serde_json::Error> {
    serde_json::to_string(event)
}

/// Parse one JSON line back into an event.
pub fn from_json_line(line: &str) -> Result<Event, serde_json::Error> {
    serde_json::from_str(line)
}

/// Serialize events to JSONL: each line newline-terminated.
pub fn to_jsonl(events: &[Event]) -> Result<String, serde_json::Error> {
    let mut out = String::new();
    for ev in events {
        out.push_str(&to_json_line(ev)?);
        out.push('\n');
    }
    Ok(out)
}

/// Parse JSONL text (blank lines skipped) back into events.
pub fn from_jsonl(text: &str) -> Result<Vec<Event>, serde_json::Error> {
    let mut out = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        out.push(from_json_line(line)?);
    }
    Ok(out)
}
