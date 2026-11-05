#![allow(dead_code)]

use serde_json::Value;

pub const TIMER: &str = include_str!("../fixtures/svc.time.countdown_timer.json");
pub const BATTERY: &str = include_str!("../fixtures/svc.power.battery_low_watch.json");
pub const NAV: &str = include_str!("../fixtures/svc.nav.offline_route.json");
pub const SCHEMA: &str = include_str!("../fixtures/service-ir-v1.schema.json");
pub const DESIGN_DOC: &str = include_str!("../../../../docs/design/SERVICE-IR.md");

pub fn json(text: &str) -> Value {
    serde_json::from_str(text).expect("fixture is JSON")
}

/// Replace (or with `None`, remove) the value at a JSON Pointer.
pub fn mutate(text: &str, pointer: &str, new: Option<Value>) -> Value {
    let mut v = json(text);
    let (parent, key) = pointer.rsplit_once('/').expect("pointer has a parent");
    let target = v.pointer_mut(parent).unwrap_or_else(|| panic!("no parent {parent}"));
    match (target, new) {
        (Value::Object(m), Some(n)) => {
            m.insert(key.to_string(), n);
        }
        (Value::Object(m), None) => {
            m.remove(key).unwrap_or_else(|| panic!("nothing at {pointer}"));
        }
        (Value::Array(a), Some(n)) => a[key.parse::<usize>().unwrap()] = n,
        (Value::Array(a), None) => {
            a.remove(key.parse::<usize>().unwrap());
        }
        _ => panic!("{parent} is not a container"),
    }
    v
}

/// Every ```json block in the design doc that is a ServiceIR.
pub fn doc_examples() -> Vec<Value> {
    DESIGN_DOC
        .split("```json\n")
        .skip(1)
        .filter_map(|b| b.split("\n```").next())
        .filter_map(|b| serde_json::from_str::<Value>(b).ok())
        .filter(|v| v.get("ir_version").is_some())
        .collect()
}
