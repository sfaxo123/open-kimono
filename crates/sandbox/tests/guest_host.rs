//! Host integration test for the B5 host runner: builds `guests/hello-rs`
//! for `wasm32-unknown-unknown` (same pattern as `tests/rust_guest.rs`) and
//! drives the host runner flow — a full-grant run whose trace event must
//! reach the journal, and a refusal run with `TraceEmit` withheld whose
//! refusal must be recorded as a `Denied` event and stop the service.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use common::kinds;
use sandbox::{
    Capability, DenyReason, EventKind, Grants, LogLevel, MemKv, Sandbox, SandboxError, ServiceSpec,
    StopReason,
};

fn build_guest() -> Vec<u8> {
    let guest = Path::new(env!("CARGO_MANIFEST_DIR")).join("guests/hello-rs");
    let target_dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("guest-host");
    let status = Command::new(env!("CARGO"))
        .current_dir(&guest)
        .args(["build", "--release", "--offline", "--target", "wasm32-unknown-unknown", "--target-dir"])
        .arg(&target_dir)
        .env_remove("RUSTFLAGS")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .status()
        .expect("cargo runs");
    assert!(status.success(), "guest build failed (is the wasm32-unknown-unknown target installed?)");
    std::fs::read(target_dir.join("wasm32-unknown-unknown/release/hello_rs.wasm")).expect("guest wasm")
}

#[test]
fn host_runner_runs_built_guest_with_a_full_grant_set() {
    let wasm = build_guest();
    let grants = Grants::of(&[Capability::Log, Capability::KvGet, Capability::KvSet, Capability::TraceEmit]);
    let mut svc = Sandbox::new()
        .load(&wasm, ServiceSpec::new("user.hello", grants), MemKv::new())
        .expect("service loads");

    let report = svc.invoke("run").expect("run completes when every used capability is granted");
    assert!(report.fuel_used > 0, "fuel use is reported");

    // Loaded first, then the journal entries `run` produced: log, kv_set, trace_emit.
    let events = kinds(&svc);
    assert_eq!(
        events[1..],
        [
            EventKind::Log { level: LogLevel::Info, message: "hello from rust".into() },
            EventKind::KvSet { key: "count".into(), value_bytes: 4 },
            EventKind::Trace { name: "service.hello".into(), payload: r#"{"count":1}"#.into() },
        ],
        "the granted TraceEmit call emits its event"
    );
    assert_eq!(svc.drain_events().len(), 4, "the runner drains Loaded + Log + KvSet + Trace");
    assert!(svc.events().is_empty(), "drain empties the journal");
}

#[test]
fn trace_emit_without_grant_is_refused_and_stops_the_service() {
    let wasm = build_guest();
    // TraceEmit withheld; Log, KvGet and KvSet granted, so the refusal lands
    // on trace_emit itself, after the successful log and kv calls.
    let grants = Grants::of(&[Capability::Log, Capability::KvGet, Capability::KvSet]);
    let mut svc = Sandbox::new()
        .load(&wasm, ServiceSpec::new("user.hello", grants), MemKv::new())
        .expect("service loads");

    let denied = SandboxError::Denied { capability: Capability::TraceEmit, reason: DenyReason::NotGranted };
    assert_eq!(svc.invoke("run"), Err(denied.clone()), "the refusal is a trap, not a return code");

    let events = kinds(&svc);
    assert!(
        events.contains(&EventKind::Denied { capability: Capability::TraceEmit, reason: DenyReason::NotGranted }),
        "the refusal is itself a journal event: {events:?}"
    );
    assert!(
        !events.iter().any(|k| matches!(k, EventKind::Trace { .. })),
        "a refused host call has no side effect: {events:?}"
    );
    assert_eq!(
        svc.stop_reason(),
        Some(&StopReason::Denied { capability: Capability::TraceEmit, reason: DenyReason::NotGranted }),
        "the service stops at the refusal"
    );
    assert_eq!(svc.invoke("run"), Err(SandboxError::Stopped), "a stopped service is never re-entered");
}
