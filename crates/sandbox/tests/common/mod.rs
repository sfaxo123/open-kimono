#![allow(dead_code)]

use sandbox::{Capability, EventKind, Grants, Limits, MemKv, Sandbox, Service, ServiceSpec};

pub fn wasm(wat_src: &str) -> Vec<u8> {
    wat::parse_str(wat_src).expect("fixture WAT parses")
}

pub fn load(wat_src: &str, caps: &[Capability]) -> Service {
    load_with(wat_src, Grants::of(caps), Limits::default())
}

pub fn load_with(wat_src: &str, grants: Grants, limits: Limits) -> Service {
    Sandbox::new()
        .load(&wasm(wat_src), ServiceSpec::new("test.svc", grants).with_limits(limits), MemKv::new())
        .expect("service loads")
}

pub fn kinds(service: &Service) -> Vec<EventKind> {
    service.events().iter().map(|e| e.kind.clone()).collect()
}

/// A service that calls `kv_set("k", "v")` from `run`.
pub const KV_SETTER: &str = r#"
(module
  (import "op6" "kv_set" (func $kv_set (param i32 i32 i32 i32) (result i32)))
  (memory (export "memory") 1)
  (global $rc (export "rc") (mut i32) (i32.const 99))
  (data (i32.const 0) "k")
  (data (i32.const 8) "v")
  (func (export "run")
    (global.set $rc (call $kv_set (i32.const 0) (i32.const 1) (i32.const 8) (i32.const 1)))))
"#;
