//! Builds `guests/hello-rs` with the real Rust toolchain for
//! `wasm32-unknown-unknown` and runs it in the sandbox.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;

use sandbox::{Capability, DenyReason, EventKind, Grants, MemKv, Sandbox, SandboxError, ServiceSpec};

fn build_guest() -> Vec<u8> {
    let guest = Path::new(env!("CARGO_MANIFEST_DIR")).join("guests/hello-rs");
    let target_dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("hello-rs");
    let status = Command::new(env!("CARGO"))
        .current_dir(&guest)
        .args(["build", "--release", "--offline", "--target-dir"])
        .arg(&target_dir)
        .env_remove("RUSTFLAGS")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .status()
        .expect("cargo runs");
    assert!(status.success(), "guest build failed (is the wasm32-unknown-unknown target installed?)");
    std::fs::read(target_dir.join("wasm32-unknown-unknown/release/hello_rs.wasm")).expect("guest wasm")
}

fn traces(svc: &sandbox::Service) -> Vec<(String, String)> {
    svc.events()
        .iter()
        .filter_map(|e| match &e.kind {
            EventKind::Trace { name, payload } => Some((name.clone(), payload.clone())),
            _ => None,
        })
        .collect()
}

#[test]
fn rust_built_service_runs_with_its_grants() {
    let wasm = build_guest();
    let kv = Rc::new(RefCell::new(MemKv::new()));
    let all = Grants::of(&Capability::ALL);
    let mut svc = Sandbox::new().load(&wasm, ServiceSpec::new("user.hello", all), kv.clone()).unwrap();
    assert_eq!(svc.requested_capabilities(), &Capability::ALL);

    svc.invoke("run").unwrap();
    svc.invoke("run").unwrap();
    svc.invoke("schedule").unwrap();
    svc.fire_due_timers(250).unwrap();

    assert_eq!(
        traces(&svc),
        [
            ("service.hello".to_string(), r#"{"count":1}"#.to_string()),
            ("service.hello".to_string(), r#"{"count":2}"#.to_string()),
            ("timer.fired".to_string(), "{}".to_string()),
        ]
    );
    assert_eq!(kv.borrow().keys("user.hello"), vec!["count".to_string()]);
    assert!(svc.memory_bytes() <= sandbox::Limits::default().max_memory_bytes);
}

#[test]
fn rust_built_service_without_kv_set_is_refused() {
    let wasm = build_guest();
    let grants = Grants::of(&[Capability::Log, Capability::KvGet, Capability::TraceEmit]);
    let mut svc = Sandbox::new().load(&wasm, ServiceSpec::new("user.hello", grants), MemKv::new()).unwrap();
    assert_eq!(
        svc.invoke("run"),
        Err(SandboxError::Denied { capability: Capability::KvSet, reason: DenyReason::NotGranted })
    );
    assert!(traces(&svc).is_empty(), "stopped before emitting");
}
