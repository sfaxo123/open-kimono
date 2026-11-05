mod common;

use common::{kinds, load};
use sandbox::{Capability, EventKind, LogLevel};

const HELLO: &str = r#"
(module
  (import "op6" "trace_emit" (func $trace (param i32 i32 i32 i32) (result i32)))
  (import "op6" "log" (func $log (param i32 i32 i32)))
  (memory (export "memory") 1)
  (data (i32.const 0) "service.hello")
  (data (i32.const 16) "{\"greeting\":\"hello\"}")
  (data (i32.const 48) "hello from wasm")
  (func (export "run")
    (call $log (i32.const 2) (i32.const 48) (i32.const 15))
    (if (i32.ne (call $trace (i32.const 0) (i32.const 13) (i32.const 16) (i32.const 20)) (i32.const 0))
      (then unreachable))))
"#;

#[test]
fn hello_service_emits_a_trace_event() {
    let mut svc = load(HELLO, &[Capability::TraceEmit, Capability::Log]);
    assert_eq!(svc.requested_capabilities(), &[Capability::Log, Capability::TraceEmit]);

    let report = svc.invoke("run").expect("hello runs");
    assert!(report.fuel_used > 0, "fuel use is reported");

    let events = kinds(&svc);
    assert_eq!(
        events[1..],
        [
            EventKind::Log { level: LogLevel::Info, message: "hello from wasm".into() },
            EventKind::Trace { name: "service.hello".into(), payload: r#"{"greeting":"hello"}"#.into() },
        ]
    );
    assert!(svc.events().iter().all(|e| e.service == "test.svc"));
}

#[test]
fn loaded_event_lists_requested_capabilities() {
    let svc = load(HELLO, &[Capability::TraceEmit, Capability::Log]);
    assert_eq!(
        kinds(&svc)[0],
        EventKind::Loaded { requested: vec![Capability::Log, Capability::TraceEmit] }
    );
}

#[test]
fn service_runs_again_after_success_with_a_fresh_budget() {
    let mut svc = load(HELLO, &[Capability::TraceEmit, Capability::Log]);
    let a = svc.invoke("run").unwrap();
    let b = svc.invoke("run").unwrap();
    assert_eq!(a.fuel_used, b.fuel_used);
    assert_eq!(svc.drain_events().len(), 5);
    assert!(svc.events().is_empty());
}

#[test]
fn bad_trace_name_is_refused() {
    let bad = HELLO.replace("service.hello", "Service Hello");
    let mut svc = load(&bad, &[Capability::TraceEmit, Capability::Log]);
    let err = svc.invoke("run").unwrap_err();
    assert!(matches!(err, sandbox::SandboxError::BadHostCall(ref m) if m.contains("trace name")), "{err}");
}

#[test]
fn missing_entry_point_is_reported_without_stopping() {
    let mut svc = load(HELLO, &[Capability::TraceEmit, Capability::Log]);
    assert_eq!(svc.invoke("nope"), Err(sandbox::SandboxError::MissingExport("nope".into())));
    assert!(svc.invoke("run").is_ok());
}
