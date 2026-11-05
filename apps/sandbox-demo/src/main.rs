//! E1 QEMU gate: prove the Wasm service sandbox *executes* on real AArch64
//! Redox, not just on the host.
//!
//! Two deterministic invocations of the same guest bytes
//! (`crates/sandbox/guests/hello-rs`, built for `wasm32-unknown-unknown`):
//!
//! 1. full grants -> the service runs to completion and its
//!    `service.hello {"count":1}` trace event is observed
//!    -> serial marker `SANDBOX_HELLO_OK`
//! 2. `trace.emit` withheld -> the same guest is refused at `trace_emit`
//!    with `Denied { TraceEmit, NotGranted }`, emits no trace event, is
//!    stopped, and is never re-entered
//!    -> serial marker `SANDBOX_REFUSED_TRACEEMIT` plus the stop reason
//!
//! Output goes through `logd` to the kernel debug console, so it lands in the
//! QEMU serial capture. Any mismatch prints `sandbox-demo: FAIL ...` and exits
//! non-zero; the markers are never printed on a path that did not happen. Each
//! marker is printed only after the assertion that justifies it, so on a
//! successful run `sandbox-demo: done` is the last line — that terminal line,
//! not the two markers, is what scripts/qemu-demo/validate.sh accepts.

mod generator;

use std::process::exit;

use sandbox::{
    Capability, DenyReason, EventKind, Grants, MemKv, Sandbox, SandboxError, ServiceSpec, StopReason,
};

/// The user-built service, embedded so the guest needs no filesystem.
const HELLO_RS_WASM: &[u8] = include_bytes!("../assets/hello_rs.wasm");

/// Pinned identity of the embedded guest: `sha256sum assets/hello_rs.wasm`.
const HELLO_RS_SHA256: &str = "ade98b90d6f85193673516d8765b47840454d5eecd4af612d5012ea29f37a012";

fn main() {
    eprintln!(
        "sandbox-demo: begin guest=hello_rs.wasm bytes={} sha256={}",
        HELLO_RS_WASM.len(),
        HELLO_RS_SHA256
    );
    match demo() {
        Ok(()) => eprintln!("sandbox-demo: done"),
        Err(reason) => {
            eprintln!("sandbox-demo: FAIL {reason}");
            exit(1);
        }
    }
}

fn demo() -> Result<(), String> {
    granted_run()?;
    refusal_run()?;
    Ok(())
}

/// Run 1 — every capability the guest uses is granted, so `run` completes and
/// its trace event reaches the host journal.
fn granted_run() -> Result<(), String> {
    let mut svc = Sandbox::new()
        .load(
            HELLO_RS_WASM,
            ServiceSpec::new("user.hello", Grants::of(&Capability::ALL)),
            MemKv::new(),
        )
        .map_err(|e| format!("load with full grants: {e}"))?;
    let report = svc
        .invoke("run")
        .map_err(|e| format!("invoke(run) with full grants: {e}"))?;

    let events = svc.drain_events();
    for event in &events {
        eprintln!("sandbox-demo: granted event {:?}", event.kind);
    }
    let trace = events.iter().find_map(|event| match &event.kind {
        EventKind::Trace { name, payload } => Some((name.clone(), payload.clone())),
        _ => None,
    });
    let (trace_name, trace_payload) =
        trace.ok_or_else(|| format!("no trace event was emitted: {events:?}"))?;
    if trace_name != "service.hello" || trace_payload != r#"{"count":1}"# {
        return Err(format!("expected service.hello with count 1, got {trace_name} {trace_payload}"));
    }
    eprintln!(
        "sandbox-demo: SANDBOX_HELLO_OK trace={trace_name} payload={trace_payload} fuel_used={} memory_bytes={}",
        report.fuel_used,
        svc.memory_bytes(),
    );
    Ok(())
}

/// Run 2 — `Log`, `KvGet` and `KvSet` are granted so the refusal lands exactly
/// on `trace_emit`, and nothing before it is skipped.
fn refusal_run() -> Result<(), String> {
    let grants = Grants::of(&[Capability::Log, Capability::KvGet, Capability::KvSet]);
    let mut svc = Sandbox::new()
        .load(HELLO_RS_WASM, ServiceSpec::new("user.hello.untrusted", grants), MemKv::new())
        .map_err(|e| format!("load with trace.emit withheld: {e}"))?;

    let expected = SandboxError::Denied { capability: Capability::TraceEmit, reason: DenyReason::NotGranted };
    match svc.invoke("run") {
        Err(refused) if refused == expected => {}
        other => return Err(format!("expected {expected:?}, got {other:?}")),
    }

    let events = svc.drain_events();
    for event in &events {
        eprintln!("sandbox-demo: withheld event {:?}", event.kind);
    }
    let refusal = EventKind::Denied { capability: Capability::TraceEmit, reason: DenyReason::NotGranted };
    if !events.iter().any(|event| event.kind == refusal) {
        return Err(format!("the refusal was not recorded as a journal event: {events:?}"));
    }
    if events.iter().any(|event| matches!(event.kind, EventKind::Trace { .. })) {
        return Err("a refused host call must have no side effect: a trace event was emitted".to_string());
    }

    let stop = svc.stop_reason().ok_or("no stop reason was recorded")?;
    let expected = StopReason::Denied { capability: Capability::TraceEmit, reason: DenyReason::NotGranted };
    if *stop != expected {
        return Err(format!("expected stop reason {expected:?}, got {stop:?}"));
    }
    // The borrow of `svc` has to end before the re-entry call below, so the stop
    // reason is rendered to an owned string here instead of being held as a
    // reference across a `&mut svc` call. The printed text is unchanged.
    let stop = format!("{stop:?}");
    // The re-entry check is the last assertion in the demo, so the success marker
    // is printed only after it. Emitting the marker first would leave both markers
    // on the serial for a run that then failed, and a marker grep would accept a
    // capture that ends in `sandbox-demo: FAIL`.
    match svc.invoke("run") {
        Err(SandboxError::Stopped) => {}
        other => return Err(format!("a stopped service must not re-enter, got {other:?}")),
    }
    eprintln!("sandbox-demo: stopped service is never re-entered");
    eprintln!(
        "sandbox-demo: SANDBOX_REFUSED_TRACEEMIT capability=trace.emit reason=NotGranted stop={stop}"
    );
    Ok(())
}
