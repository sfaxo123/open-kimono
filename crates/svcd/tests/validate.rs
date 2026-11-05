//! Load/validate: the D1 worked examples are accepted; malformed IRs are
//! rejected with the exact JSON Pointer and error code.

mod common;

use common::*;
use serde_json::{json, Value};
use svcd::{validate::load_value, Code, IrErrors};

fn reject(v: Value) -> IrErrors {
    match load_value(v) {
        Ok(ir) => panic!("{} should have been rejected", ir.id),
        Err(e) => e,
    }
}

#[track_caller]
fn assert_rejects(v: Value, path: &str, code: Code) {
    let errs = reject(v);
    assert!(errs.has(path, code), "expected {code:?} at {path}, got:\n{errs}");
}

#[test]
fn timer_and_battery_examples_are_accepted() {
    let timer = svcd::load(TIMER).expect("timer example");
    assert_eq!(timer.id, "svc.time.countdown_timer");
    let battery = svcd::load(BATTERY).expect("battery example");
    assert_eq!(battery.triggers.len(), 2);
}

#[test]
fn fixtures_are_verbatim_copies_of_the_design_doc() {
    let examples = doc_examples();
    for fixture in [TIMER, BATTERY, NAV] {
        assert!(examples.contains(&json(fixture)), "fixture drifted from SERVICE-IR.md");
    }
}

#[test]
fn nav_example_is_rejected_for_two_real_d1_defects() {
    let errs = reject(json(NAV));
    assert!(errs.has("/actions/act_announce_offline_miss/idempotency_key_template", Code::MissingField), "{errs}");
    assert!(errs.has("/actions/act_geocode_and_route/idempotency_key_template", Code::UnknownReference), "{errs}");
    assert_eq!(errs.0.len(), 3, "only the known defects:\n{errs}");
}

#[test]
fn not_json_is_a_syntax_error_at_the_root() {
    let errs = svcd::load("{\"ir_version\": ").unwrap_err();
    assert_eq!(errs.0.len(), 1);
    assert_eq!((errs.0[0].path.as_str(), errs.0[0].code), ("", Code::Syntax));
}

#[test]
fn unknown_top_level_field() {
    assert_rejects(mutate(TIMER, "/colour", Some(json!("orange"))), "/colour", Code::UnknownField);
}

#[test]
fn unknown_nested_field_names_its_path() {
    let v = mutate(TIMER, "/actions/act_arm_timer/timeout/deadline_ms", Some(json!(5)));
    assert_rejects(v, "/actions/act_arm_timer/timeout/deadline_ms", Code::UnknownField);
}

#[test]
fn missing_required_field() {
    assert_rejects(mutate(TIMER, "/power_budget", None), "/power_budget", Code::MissingField);
}

#[test]
fn bad_enum_value() {
    assert_rejects(mutate(TIMER, "/triggers/1/kind", Some(json!("cron"))), "/triggers/1/kind", Code::Enum);
}

#[test]
fn wrong_type() {
    let v = mutate(TIMER, "/actions/act_arm_timer/timeout/idle_ms", Some(json!("fast")));
    assert_rejects(v, "/actions/act_arm_timer/timeout/idle_ms", Code::Type);
}

#[test]
fn id_pattern() {
    assert_rejects(mutate(TIMER, "/id", Some(json!("Svc.Timer"))), "/id", Code::Pattern);
}

#[test]
fn timeout_out_of_range() {
    let v = mutate(TIMER, "/actions/act_arm_timer/timeout/total_ms", Some(json!(999_999)));
    assert_rejects(v, "/actions/act_arm_timer/timeout/total_ms", Code::Range);
}

#[test]
fn unknown_entry_action() {
    let v = mutate(TIMER, "/triggers/0/entry_action", Some(json!("act_missing")));
    assert_rejects(v, "/triggers/0/entry_action", Code::UnknownReference);
}

#[test]
fn cycle_in_action_graph() {
    let v = mutate(TIMER, "/actions/act_cancel_timer/next", Some(json!([{ "target": "act_fire_alarm" }])));
    let errs = reject(v);
    let e = errs.0.iter().find(|e| e.code == Code::Cycle).unwrap_or_else(|| panic!("{errs}"));
    assert!(e.message.contains("act_cancel_timer -> act_fire_alarm -> act_cancel_timer"), "{}", e.message);
}

#[test]
fn irreversible_action_without_idempotency_key() {
    let v = mutate(TIMER, "/actions/act_fire_alarm/idempotency_key_template", None);
    assert_rejects(v, "/actions/act_fire_alarm/idempotency_key_template", Code::MissingField);
}

#[test]
fn malformed_cel_guard() {
    let v = mutate(TIMER, "/triggers/1/condition_cel", Some(json!("state.armed = true")));
    assert_rejects(v, "/triggers/1/condition_cel", Code::Expression);
}

#[test]
fn guard_reading_unknown_state_property() {
    let v = mutate(TIMER, "/triggers/1/condition_cel", Some(json!("state.is_armed == true")));
    assert_rejects(v, "/triggers/1/condition_cel", Code::UnknownReference);
}

#[test]
fn template_reading_undeclared_parameter() {
    let v = mutate(TIMER, "/actions/act_confirm_timer_voice/dialog/speech", Some(json!("{{params.label}} set")));
    assert_rejects(v, "/actions/act_confirm_timer_voice/dialog/speech", Code::UnknownReference);
}

#[test]
fn patch_path_outside_state_schema() {
    let v = mutate(TIMER, "/actions/act_cancel_timer/state_patch/0/path", Some(json!("/enabled")));
    assert_rejects(v, "/actions/act_cancel_timer/state_patch/0/path", Code::StateSchema);
}

#[test]
fn initial_state_violates_state_schema() {
    let v = mutate(TIMER, "/state_schema/initial_state/status", Some(json!("paused")));
    assert_rejects(v, "/state_schema/initial_state/status", Code::StateSchema);
}

#[test]
fn unsupported_state_schema_keyword_is_refused() {
    let v = mutate(TIMER, "/state_schema/schema/oneOf", Some(json!([])));
    assert_rejects(v, "/state_schema/schema/oneOf", Code::StateSchema);
}

#[test]
fn timer_trigger_without_timer_capability() {
    assert_rejects(mutate(TIMER, "/capabilities/0", None), "/triggers/1", Code::Capability);
}

#[test]
fn dialog_without_audio_dependency() {
    assert_rejects(mutate(TIMER, "/deps/schemes/1", None), "/actions/act_confirm_timer_voice", Code::Dependency);
}

#[test]
fn ui_patch_outside_granted_surface() {
    let v = mutate(TIMER, "/actions/act_update_timer_surface/ui_patch/surface_id", Some(json!("someone-elses-surface")));
    assert_rejects(v, "/actions/act_update_timer_surface/ui_patch/surface_id", Code::Capability);
}

#[test]
fn read_only_state_mutation_is_a_contradiction() {
    let v = mutate(TIMER, "/actions/act_cancel_timer/read_only", Some(json!(true)));
    assert_rejects(v, "/actions/act_cancel_timer/read_only", Code::Contradiction);
}

#[test]
fn bus_emit_that_targets_svcd_is_refused() {
    let v = mutate(
        TIMER,
        "/actions/act_cancel_timer",
        Some(json!({
            "type": "bus_emit", "description": "Try to shut the executor down.", "effect_class": "reversible_staged",
            "read_only": false, "timeout": { "idle_ms": 10, "total_ms": 10 },
            "bus_emit": { "derivation": "root", "msg_type": "svcd.shutdown", "data": {} }
        })),
    );
    assert_rejects(v, "/actions/act_cancel_timer/bus_emit/msg_type", Code::LifecycleGuard);
}

#[test]
fn unsupported_timer_mode_is_explicit() {
    let v = mutate(TIMER, "/triggers/1/timer_schedule/mode", Some(json!("cron_wall")));
    assert_rejects(v, "/triggers/1/timer_schedule/mode", Code::Unsupported);
}

#[test]
fn sentence_with_undeclared_slot() {
    let v = mutate(TIMER, "/triggers/0/voice_intent/sentences/0", Some(json!("timer for {mins} minutes")));
    assert_rejects(v, "/triggers/0/voice_intent/sentences/0", Code::UnknownReference);
}

#[test]
fn unbalanced_sentence_template() {
    let v = mutate(TIMER, "/triggers/0/voice_intent/sentences/0", Some(json!("timer for {minutes} minute[s")));
    assert_rejects(v, "/triggers/0/voice_intent/sentences/0", Code::Expression);
}

#[test]
fn simulator_events_must_not_go_back_in_time() {
    let v = mutate(BATTERY, "/tests/0/events/1/at_monotonic_ms", Some(json!(5)));
    assert_rejects(v, "/tests/0/events/1/at_monotonic_ms", Code::Test);
}

#[test]
fn every_problem_is_reported_not_just_the_first() {
    let v = mutate(TIMER, "/id", Some(json!("bad")));
    let v = mutate(&v.to_string(), "/title", Some(json!("x")));
    let v = mutate(&v.to_string(), "/power_budget/memory_limit_mb", Some(json!(0)));
    let errs = reject(v);
    assert!(errs.has("/id", Code::Pattern) && errs.has("/title", Code::Length));
    assert!(errs.has("/power_budget/memory_limit_mb", Code::Range), "{errs}");
}
