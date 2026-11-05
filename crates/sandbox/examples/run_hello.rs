//! Minimal host runner for the built `hello-rs` service (B5).
//!
//! Loads the built `.wasm`, runs `run` with a full grant set and drains the
//! events, then runs a refusal variant with `TraceEmit` withheld and asserts
//! the refusal event: the guest stops at `trace_emit` with
//! `Denied { TraceEmit, NotGranted }` and emits no trace event.
//!
//! From `crates/sandbox`: `cargo run --example run_hello --offline`
//! (or `cargo test --offline` via `tests/guest_host.rs`).

use std::path::{Path, PathBuf};
use std::process::Command;

use sandbox::{
    Capability, DenyReason, EventKind, Grants, MemKv, Sandbox, SandboxError, ServiceSpec,
};

/// Locate the guest binary without assuming it was rebuilt here:
/// `$HELLO_RS_WASM`, then the two plain-build locations, then build it fresh
/// (same recipe as `tests/rust_guest.rs`, into a scratch target dir).
fn hello_rs_wasm() -> Vec<u8> {
    if let Some(p) = std::env::var_os("HELLO_RS_WASM") {
        let p = PathBuf::from(p);
        if p.is_file() {
            eprintln!("run_hello: loading guest {}", p.display());
            return std::fs::read(&p).expect("HELLO_RS_WASM is unreadable");
        }
    }
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    for candidate in [
        crate_dir.join("guests/hello-rs/target/wasm32-unknown-unknown/release/hello_rs.wasm"),
        crate_dir.join("target/wasm32-unknown-unknown/release/hello_rs.wasm"),
    ] {
        if candidate.is_file() {
            eprintln!("run_hello: loading guest {}", candidate.display());
            return std::fs::read(&candidate).expect("guest wasm is unreadable");
        }
    }
    let guest = crate_dir.join("guests/hello-rs");
    let target_dir = std::env::temp_dir().join("hello-rs-run-hello");
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

fn main() {
    let wasm = hello_rs_wasm();

    // Running variant: every capability `run` uses is granted, so the service
    // completes and its log + kv + trace events reach the journal.
    let running = Grants::of(&[Capability::Log, Capability::KvGet, Capability::KvSet, Capability::TraceEmit]);
    let mut svc = Sandbox::new()
        .load(&wasm, ServiceSpec::new("user.hello", running), MemKv::new())
        .expect("the guest loads under the op6 ABI");
    let report = svc.invoke("run").expect("run completes with all used caps granted");
    println!("running: fuel_used={}", report.fuel_used);
    for e in svc.drain_events() {
        println!("  {:?}", e.kind);
    }

    // Refusal variant: TraceEmit withheld (Log, KvGet and KvSet granted, so
    // the refusal falls on trace_emit itself).
    let withheld = Grants::of(&[Capability::Log, Capability::KvGet, Capability::KvSet]);
    let mut svc = Sandbox::new()
        .load(&wasm, ServiceSpec::new("user.hello", withheld), MemKv::new())
        .expect("the guest loads under the op6 ABI");
    let denied = SandboxError::Denied { capability: Capability::TraceEmit, reason: DenyReason::NotGranted };
    assert_eq!(svc.invoke("run"), Err(denied.clone()), "TraceEmit without a grant is refused");
    let kinds: Vec<EventKind> = svc.drain_events().into_iter().map(|e| e.kind).collect();
    let refusal = EventKind::Denied { capability: Capability::TraceEmit, reason: DenyReason::NotGranted };
    assert!(kinds.contains(&refusal), "the refusal is itself a journal event: {kinds:?}");
    assert!(
        !kinds.iter().any(|k| matches!(k, EventKind::Trace { .. })),
        "a refused call has no side effect: {kinds:?}"
    );
    assert_eq!(svc.invoke("run"), Err(SandboxError::Stopped), "a stopped service is never re-entered");
    println!("refusal: TraceEmit withheld -> denied, no trace emitted, service stopped");
    println!("ok");
}
