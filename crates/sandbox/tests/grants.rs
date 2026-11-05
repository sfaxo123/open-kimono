mod common;

use std::cell::RefCell;
use std::rc::Rc;

use common::{kinds, load, load_with, wasm, KV_SETTER};
use sandbox::{
    Capability, DenyReason, EventKind, Grant, Grants, KvBackend, Limits, MemKv, Sandbox, SandboxError,
    ServiceSpec, StopReason, ERR_QUOTA, OK,
};

#[test]
fn kv_set_without_grant_is_refused() {
    let kv = Rc::new(RefCell::new(MemKv::new()));
    let spec = ServiceSpec::new("no.kv", Grants::of(&[Capability::KvGet, Capability::Log]));
    let mut svc = Sandbox::new().load(&wasm(KV_SETTER), spec, kv.clone()).unwrap();

    let err = svc.invoke("run").unwrap_err();
    let denied = SandboxError::Denied { capability: Capability::KvSet, reason: DenyReason::NotGranted };
    assert_eq!(err, denied);

    assert!(kv.borrow().keys("no.kv").is_empty(), "refused call has no side effect");
    let events = kinds(&svc);
    assert!(events.contains(&EventKind::Denied { capability: Capability::KvSet, reason: DenyReason::NotGranted }));
    assert_eq!(
        svc.stop_reason(),
        Some(&StopReason::Denied { capability: Capability::KvSet, reason: DenyReason::NotGranted })
    );
    assert_eq!(svc.invoke("run"), Err(SandboxError::Stopped), "a stopped service is not reused");
}

#[test]
fn kv_set_with_grant_writes_only_the_service_namespace() {
    let kv = Rc::new(RefCell::new(MemKv::new()));
    let spec = ServiceSpec::new("svc.a", Grants::of(&[Capability::KvSet]));
    let mut svc = Sandbox::new().load(&wasm(KV_SETTER), spec, kv.clone()).unwrap();
    svc.invoke("run").unwrap();
    assert_eq!(svc.global_i32("rc"), Some(OK));
    assert_eq!(kv.borrow().keys("svc.a"), vec!["k".to_string()]);
    assert_eq!(kv.borrow().keys("svc.b"), Vec::<String>::new());
    assert!(kinds(&svc).contains(&EventKind::KvSet { key: "k".into(), value_bytes: 1 }));
}

const KV_GETTER: &str = r#"
(module
  (import "op6" "kv_get" (func $kv_get (param i32 i32 i32 i32) (result i32)))
  (import "op6" "trace_emit" (func $trace (param i32 i32 i32 i32) (result i32)))
  (memory (export "memory") 1)
  (data (i32.const 0) "k")
  (data (i32.const 16) "kv.got")
  (func (export "run") (local $n i32)
    (local.set $n (call $kv_get (i32.const 0) (i32.const 1) (i32.const 64) (i32.const 32)))
    (if (i32.lt_s (local.get $n) (i32.const 0)) (then (local.set $n (i32.const 0))))
    (drop (call $trace (i32.const 16) (i32.const 6) (i32.const 64) (local.get $n)))))
"#;

#[test]
fn kv_is_scoped_per_service_on_a_shared_backend() {
    let kv = Rc::new(RefCell::new(MemKv::new()));
    kv.borrow_mut().set("svc.b", "k", b"secret-of-b");
    kv.borrow_mut().set("svc.a", "k", b"mine");
    let sandbox = Sandbox::new();
    let caps = Grants::of(&[Capability::KvGet, Capability::TraceEmit]);
    let mut a = sandbox.load(&wasm(KV_GETTER), ServiceSpec::new("svc.a", caps.clone()), kv.clone()).unwrap();
    let mut c = sandbox.load(&wasm(KV_GETTER), ServiceSpec::new("svc.c", caps), kv.clone()).unwrap();
    a.invoke("run").unwrap();
    c.invoke("run").unwrap();
    let got = |s: &sandbox::Service| match s.events().last().unwrap().kind.clone() {
        EventKind::Trace { payload, .. } => payload,
        other => panic!("{other:?}"),
    };
    assert_eq!(got(&a), "mine");
    assert_eq!(got(&c), "", "svc.c sees nothing, never svc.b's value");
}

#[test]
fn expired_grant_is_refused() {
    let grants = Grants::none().with(Grant::until(Capability::KvSet, 1_000));
    let mut svc = load_with(KV_SETTER, grants, Limits::default());
    svc.set_now(1_000);
    assert_eq!(
        svc.invoke("run"),
        Err(SandboxError::Denied { capability: Capability::KvSet, reason: DenyReason::Expired })
    );
}

#[test]
fn grant_valid_before_expiry() {
    let grants = Grants::none().with(Grant::until(Capability::KvSet, 1_000));
    let mut svc = load_with(KV_SETTER, grants, Limits::default());
    svc.set_now(999);
    assert!(svc.invoke("run").is_ok());
}

#[test]
fn revoked_grant_is_refused_while_running() {
    let mut svc = load(KV_SETTER, &[Capability::KvSet]);
    svc.invoke("run").unwrap();
    assert!(svc.revoke(Capability::KvSet));
    assert_eq!(
        svc.invoke("run"),
        Err(SandboxError::Denied { capability: Capability::KvSet, reason: DenyReason::Revoked })
    );
}

#[test]
fn log_without_grant_is_refused() {
    let src = r#"(module (import "op6" "log" (func $log (param i32 i32 i32)))
      (memory (export "memory") 1)
      (func (export "run") (call $log (i32.const 2) (i32.const 0) (i32.const 1))))"#;
    let mut svc = load(src, &[Capability::TraceEmit]);
    assert_eq!(
        svc.invoke("run"),
        Err(SandboxError::Denied { capability: Capability::Log, reason: DenyReason::NotGranted })
    );
}

#[test]
fn unknown_import_is_refused_at_load() {
    let src = r#"(module (import "wasi_snapshot_preview1" "fd_write" (func (param i32 i32 i32 i32) (result i32))))"#;
    let err = Sandbox::new()
        .load(&wasm(src), ServiceSpec::new("x", Grants::none()), MemKv::new())
        .err()
        .unwrap();
    assert_eq!(
        err,
        SandboxError::UnknownImport { module: "wasi_snapshot_preview1".into(), name: "fd_write".into() }
    );
}

#[test]
fn kv_quota_is_a_return_code() {
    let limits = Limits { kv_quota_bytes: 1, ..Limits::default() };
    let mut svc = load_with(KV_SETTER, Grants::of(&[Capability::KvSet]), limits);
    svc.invoke("run").unwrap();
    assert_eq!(svc.global_i32("rc"), Some(ERR_QUOTA));
    assert!(!kinds(&svc).iter().any(|k| matches!(k, EventKind::KvSet { .. })));
}
