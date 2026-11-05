//! Cross-check against the D1 JSON Schema itself (SERVICE-IR.md §2.1, run
//! with the `jsonschema` crate as a test-only oracle). For schema-level
//! mistakes svcd and the schema must agree; the semantic cases show what svcd
//! adds on top (the schema accepts them, svcd must not).

mod common;

use common::*;
use serde_json::{json, Value};

fn schema_accepts(v: &Value) -> bool {
    let schema = json(SCHEMA);
    let validator = jsonschema::draft202012::new(&schema).expect("D1 schema compiles");
    validator.is_valid(v)
}

fn svcd_accepts(v: &Value) -> bool {
    svcd::validate::load_value(v.clone()).is_ok()
}

#[test]
fn embedded_schema_is_the_one_in_the_design_doc() {
    let schema = json(SCHEMA);
    assert!(DESIGN_DOC.contains("\"$id\": \"https://redox-op6.local/schemas/service-ir/v1.json\""));
    assert_eq!(schema["$id"], "https://redox-op6.local/schemas/service-ir/v1.json");
}

#[test]
fn schema_and_svcd_accept_the_worked_examples() {
    for (name, text) in [("timer", TIMER), ("battery", BATTERY)] {
        let v = json(text);
        assert!(schema_accepts(&v), "D1 schema rejects its own {name} example");
        assert!(svcd_accepts(&v), "svcd rejects the {name} example");
    }
}

#[test]
fn schema_level_mistakes_are_rejected_by_both() {
    let cases: Vec<(&str, Value)> = vec![
        ("unknown field", mutate(TIMER, "/colour", Some(json!(1)))),
        ("missing power_budget", mutate(TIMER, "/power_budget", None)),
        ("bad trigger kind", mutate(TIMER, "/triggers/1/kind", Some(json!("cron")))),
        ("wrong type", mutate(TIMER, "/actions/act_arm_timer/timeout/idle_ms", Some(json!("fast")))),
        ("id pattern", mutate(TIMER, "/id", Some(json!("Svc.Timer")))),
        ("version pattern", mutate(TIMER, "/version", Some(json!("1.0")))),
        ("ir_version const", mutate(TIMER, "/ir_version", Some(json!("2.0.0")))),
        ("title too short", mutate(TIMER, "/title", Some(json!("x")))),
        ("timeout range", mutate(TIMER, "/actions/act_arm_timer/timeout/total_ms", Some(json!(999_999)))),
        ("memory range", mutate(TIMER, "/power_budget/memory_limit_mb", Some(json!(0)))),
        ("fuel range", mutate(TIMER, "/power_budget/wasm_fuel_per_invocation", Some(json!(10)))),
        ("unknown capability", mutate(TIMER, "/capabilities/0/capability", Some(json!("root.shell")))),
        ("reserved intent", mutate(TIMER, "/triggers/0/voice_intent/intent_name", Some(json!("stop")))),
        ("trigger id pattern", mutate(TIMER, "/triggers/0/id", Some(json!("start")))),
        ("patch path pattern", mutate(TIMER, "/actions/act_cancel_timer/state_patch/0/path", Some(json!("armed")))),
        ("state namespace", mutate(TIMER, "/state_schema/namespace", Some(json!("svc.time")))),
        ("no triggers", mutate(TIMER, "/triggers", Some(json!([])))),
        ("no tests", mutate(TIMER, "/tests", Some(json!([])))),
        ("sensor scheme", mutate(BATTERY, "/triggers/0/sensor_threshold/scheme_path", Some(json!("disk:battery")))),
        ("wakeups range", mutate(BATTERY, "/power_budget/max_wakeups_per_minute", Some(json!(601)))),
        ("battery_pct range", mutate(BATTERY, "/tests/0/initial_env/battery_pct", Some(json!(140)))),
    ];
    for (name, v) in cases {
        assert!(!schema_accepts(&v), "{name}: D1 schema accepted it");
        assert!(!svcd_accepts(&v), "{name}: svcd accepted it");
    }
}

#[test]
fn semantic_mistakes_pass_the_schema_but_not_svcd() {
    let cases: Vec<(&str, Value)> = vec![
        ("unknown entry_action", mutate(TIMER, "/triggers/0/entry_action", Some(json!("act_missing")))),
        ("cycle", mutate(TIMER, "/actions/act_cancel_timer/next", Some(json!([{ "target": "act_fire_alarm" }])))),
        ("irreversible without key", mutate(TIMER, "/actions/act_fire_alarm/idempotency_key_template", None)),
        ("bad CEL", mutate(TIMER, "/triggers/1/condition_cel", Some(json!("state.armed = true")))),
        ("unknown state field", mutate(TIMER, "/triggers/1/condition_cel", Some(json!("state.on == true")))),
        ("initial_state vs schema", mutate(TIMER, "/state_schema/initial_state/status", Some(json!("paused")))),
        ("missing capability", mutate(TIMER, "/capabilities/0", None)),
        ("read_only mutation", mutate(TIMER, "/actions/act_cancel_timer/read_only", Some(json!(true)))),
    ];
    for (name, v) in cases {
        assert!(schema_accepts(&v), "{name}: expected the D1 schema to accept (svcd-only rule)");
        assert!(!svcd_accepts(&v), "{name}: svcd accepted it");
    }
}
