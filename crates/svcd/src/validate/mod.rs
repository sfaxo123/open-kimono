//! Load + validate a ServiceIR. [`load`] is the only way in: JSON syntax,
//! then typed decoding (unknown/missing fields, wrong types, bad enums — with
//! the JSON Pointer of the offending field), then schema rules serde cannot
//! express ([`fields`]), then meaning ([`graph`]), state and test checks.

mod fields;
mod action;
mod graph;
pub mod state_schema;

pub use fields::{is_semver, is_service_id, is_state_pointer};

use serde_json::Value;

use crate::error::{ptr, Code, IrError, IrErrors};
use crate::ir::{ServiceIr, SimEventType};
use crate::time::parse_iso_utc_us;

#[derive(Default)]
pub struct Report(pub Vec<IrError>);

impl Report {
    pub fn err(&mut self, path: impl Into<String>, code: Code, msg: impl Into<String>) {
        self.0.push(IrError::new(path, code, msg));
    }
}

/// Parse and fully validate an IR document.
pub fn load(json: &str) -> Result<ServiceIr, IrErrors> {
    let value: Value = serde_json::from_str(json)
        .map_err(|e| IrErrors(vec![IrError::new("", Code::Syntax, format!("not JSON: {e}"))]))?;
    load_value(value)
}

pub fn load_value(value: Value) -> Result<ServiceIr, IrErrors> {
    let ir: ServiceIr = serde_path_to_error::deserialize(value).map_err(|e| IrErrors(vec![serde_error(&e)]))?;
    validate(&ir)?;
    Ok(ir)
}

/// Every rule beyond typed decoding. Returns all errors, not just the first.
pub fn validate(ir: &ServiceIr) -> Result<(), IrErrors> {
    let mut r = Report::default();
    fields::check(ir, &mut r);
    graph::check(ir, &mut r);
    state(ir, &mut r);
    tests(ir, &mut r);
    if r.0.is_empty() {
        Ok(())
    } else {
        Err(IrErrors(r.0))
    }
}

fn serde_error(e: &serde_path_to_error::Error<serde_json::Error>) -> IrError {
    use serde_path_to_error::Segment;
    let mut path = String::new();
    for seg in e.path().iter() {
        match seg {
            Segment::Seq { index } => path = ptr(&path, index),
            Segment::Map { key } => path = ptr(&path, key),
            Segment::Enum { variant } => path = ptr(&path, variant),
            Segment::Unknown => {}
        }
    }
    let msg = e.inner().to_string();
    let code = if msg.starts_with("unknown field") {
        Code::UnknownField
    } else if msg.starts_with("missing field") {
        Code::MissingField
    } else if msg.starts_with("unknown variant") {
        Code::Enum
    } else {
        Code::Type
    };
    // serde reports a missing field at its parent object; point at the field.
    if code == Code::MissingField {
        if let Some(name) = msg.split('`').nth(1) {
            path = ptr(&path, name);
        }
    }
    let msg = msg.split(" at line ").next().unwrap_or(&msg).to_string();
    IrError::new(path, code, msg)
}

fn state(ir: &ServiceIr, r: &mut Report) {
    let schema = Value::Object(ir.state_schema.schema.clone());
    if let Err(e) = state_schema::check_schema(&schema) {
        r.err(format!("/state_schema/schema{}", e.path), Code::StateSchema, e.message);
        return;
    }
    let initial = Value::Object(ir.state_schema.initial_state.clone());
    for e in state_schema::validate(&schema, &initial) {
        r.err(format!("/state_schema/initial_state{}", e.path), Code::StateSchema, e.message);
    }
}

fn tests(ir: &ServiceIr, r: &mut Report) {
    for (i, t) in ir.tests.iter().enumerate() {
        let base = ptr("/tests", i);
        if parse_iso_utc_us(&t.initial_env.wall_iso).is_none() {
            let p = ptr(&ptr(&base, "initial_env"), "wall_iso");
            r.err(p, Code::Test, format!("`{}` is not an RFC 3339 UTC timestamp (YYYY-MM-DDThh:mm:ssZ)", t.initial_env.wall_iso));
        }
        let mut last = t.initial_env.monotonic_ms;
        for (j, ev) in t.events.iter().enumerate() {
            let eb = ptr(&ptr(&base, "events"), j);
            if ev.at_monotonic_ms < last {
                r.err(ptr(&eb, "at_monotonic_ms"), Code::Test, format!("{} goes back in time (previous {last})", ev.at_monotonic_ms));
            }
            last = last.max(ev.at_monotonic_ms);
            let need = |r: &mut Report, key: &str, ok: fn(&Value) -> bool| {
                if !ev.payload.get(key).is_some_and(ok) {
                    r.err(ptr(&ptr(&eb, "payload"), key), Code::Test, format!("`{}` event needs payload.{key}", ev.event_type.as_str()));
                }
            };
            match ev.event_type {
                SimEventType::Utterance => need(r, "text", Value::is_string),
                SimEventType::SensorSample => need(r, "scheme", Value::is_string),
                SimEventType::BusMessage => need(r, "msg_type", Value::is_string),
                SimEventType::UiAction => {
                    // A2UI wire shape (`surfaceId`, `name`) or snake_case.
                    for (snake, camel) in [("surface_id", "surfaceId"), ("action_name", "name")] {
                        if !ev.payload.get(camel).is_some_and(Value::is_string) {
                            need(r, snake, Value::is_string);
                        }
                    }
                }
                SimEventType::AdvanceClock => {
                    if let Some(m) = ev.payload.get("monotonic_ms") {
                        if m.as_u64() != Some(ev.at_monotonic_ms) {
                            r.err(ptr(&ptr(&eb, "payload"), "monotonic_ms"), Code::Test, "advance_clock target must equal at_monotonic_ms");
                        }
                    }
                }
                SimEventType::StopSignal => {}
            }
        }
    }
}
