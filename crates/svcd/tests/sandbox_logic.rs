//! `logic` actions executed as real Wasm in `crates/sandbox` through the
//! `ActionHost` seam (`cargo test --features sandbox`).

use serde_json::{json, Value};
use svcd::sandbox_host::{Component, SandboxHost};
use svcd::{ActionKind, Config, ServiceStatus, SimHost, Svcd};

const CHIME: &str = include_str!("fixtures/svc.test.chime.json");

/// `reserve` echoes its args as a trace event and returns `{"slot":7}`.
const CHIME_CORE: &str = r#"
(module
  (import "op6" "kv_get" (func $kv_get (param i32 i32 i32 i32) (result i32)))
  (import "op6" "kv_set" (func $kv_set (param i32 i32 i32 i32) (result i32)))
  (import "op6" "trace_emit" (func $trace (param i32 i32 i32 i32) (result i32)))
  (memory (export "memory") 1)
  (data (i32.const 0) "args")
  (data (i32.const 8) "output")
  (data (i32.const 16) "{\"slot\":7}")
  (data (i32.const 32) "chime.reserved")
  (func (export "reserve")
    (local $n i32)
    (local.set $n (call $kv_get (i32.const 0) (i32.const 4) (i32.const 256) (i32.const 512)))
    (drop (call $trace (i32.const 32) (i32.const 14) (i32.const 256) (local.get $n)))
    (drop (call $kv_set (i32.const 8) (i32.const 6) (i32.const 16) (i32.const 10))))
  (func (export "release"))
  (func (export "report"))
  (func (export "spin") (loop $l (br $l))))
"#;

fn host() -> SandboxHost<SimHost> {
    use sandbox::{Capability, Grants, Limits};
    let grants = Grants::of(&[Capability::KvGet, Capability::KvSet, Capability::TraceEmit]);
    let c = Component { wasm: wat::parse_str(CHIME_CORE).unwrap(), grants, limits: Limits::default() };
    SandboxHost::new(SimHost::new()).component("chime_core", c)
}

fn install(ir: Value) -> Svcd<SandboxHost<SimHost>> {
    let mut s = Svcd::new(host(), Config::default());
    s.install(svcd::validate::load_value(ir).unwrap()).unwrap();
    s
}

#[test]
fn logic_action_runs_wasm_and_its_output_lands_in_state() {
    let mut s = install(serde_json::from_str(CHIME).unwrap());
    s.utterance("ring the brass chime", Some("u1")).expect("matched");
    let state = s.state("svc.test.chime").unwrap();
    assert_eq!(state["slot"]["result"], json!({ "slot": 7 }), "guest output via KV `output`");
    let trace = &state["slot"]["trace"][0];
    assert_eq!(trace["name"], json!("chime.reserved"));
    assert_eq!(trace["payload"], json!(r#"{"label":"brass"}"#), "rendered args_mapping reached the guest");
    assert_eq!(state["count"], json!(1));
    assert_eq!(s.host().inner.spoken(), vec!["Ding (brass)."], "dialogs still go to the inner host");
    assert_eq!(s.service("svc.test.chime").unwrap().status, ServiceStatus::Running);
}

#[test]
fn runaway_guest_is_stopped_by_fuel_and_svcd_stops_the_service() {
    let mut ir: Value = serde_json::from_str(CHIME).unwrap();
    ir["actions"]["act_reserve"]["wasm_call"]["function"] = json!("spin");
    let mut s = install(ir);
    s.utterance("ring the chime", Some("u1"));
    match &s.service("svc.test.chime").unwrap().status {
        ServiceStatus::Stopped { violation: Some(v), .. } => {
            assert_eq!(v.budget, "wasm_fuel_per_invocation");
            assert!(v.observed.contains("10001"), "{}", v.observed);
        }
        other => panic!("expected fuel stop, got {other:?}"),
    }
    assert!(s.host().inner.spoken().is_empty());
    let comp = s.host().inner.calls.iter().find(|c| matches!(c.kind, ActionKind::Compensate { .. }));
    assert_eq!(comp.map(|c| c.action_id.as_str()), Some("act_release"), "the half-done reserve is compensated");
    assert_eq!(s.state("svc.test.chime").unwrap(), &json!({ "count": 0 }));
}

#[test]
fn missing_component_bytes_fail_the_run_with_a_reason() {
    let mut s = Svcd::new(SandboxHost::new(SimHost::new()), Config::default());
    s.install(svcd::load(CHIME).unwrap()).unwrap();
    s.utterance("ring the chime", None);
    let err = s.bus().iter().find(|m| m.msg_type == "ovos.intent.handler.error").unwrap();
    assert!(err.data["error"].as_str().unwrap().contains("no Wasm registered for component `chime_core`"));
}
