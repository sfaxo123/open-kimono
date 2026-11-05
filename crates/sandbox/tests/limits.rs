mod common;

use std::time::{Duration, Instant};

use common::{kinds, load, load_with, wasm};
use sandbox::{
    Capability, EventKind, Grants, Limits, MemKv, ResourceDenial, Sandbox, SandboxError, ServiceSpec,
    StopReason, HOST_CALL_FUEL, WASM_PAGE_BYTES,
};

const RUNAWAY: &str = r#"
(module
  (func (export "run") (loop $spin (br $spin))))
"#;

#[test]
fn runaway_loop_is_stopped_by_fuel() {
    let limits = Limits { fuel_per_call: 50_000, ..Limits::default() };
    let mut svc = load_with(RUNAWAY, Grants::none(), limits);
    let started = Instant::now();
    assert_eq!(svc.invoke("run"), Err(SandboxError::OutOfFuel { fuel_per_call: 50_000 }));
    assert!(started.elapsed() < Duration::from_secs(5), "stopped promptly");
    assert_eq!(svc.stop_reason(), Some(&StopReason::OutOfFuel { fuel_per_call: 50_000 }));
    assert!(kinds(&svc).contains(&EventKind::Stopped(StopReason::OutOfFuel { fuel_per_call: 50_000 })));
    assert_eq!(svc.invoke("run"), Err(SandboxError::Stopped));
}

#[test]
fn runaway_loop_is_stopped_with_default_budget() {
    let mut svc = load(RUNAWAY, &[]);
    let fuel = Limits::default().fuel_per_call;
    assert_eq!(svc.invoke("run"), Err(SandboxError::OutOfFuel { fuel_per_call: fuel }));
}

#[test]
fn host_calls_cost_fuel_so_log_spam_stops() {
    let src = r#"(module (import "op6" "log" (func $log (param i32 i32 i32)))
      (memory (export "memory") 1)
      (func (export "run") (loop $spam (call $log (i32.const 2) (i32.const 0) (i32.const 8)) (br $spam))))"#;
    let limits = Limits { fuel_per_call: 10_000, ..Limits::default() };
    let mut svc = load_with(src, Grants::of(&[Capability::Log]), limits);
    assert_eq!(svc.invoke("run"), Err(SandboxError::OutOfFuel { fuel_per_call: 10_000 }));
    let logs = kinds(&svc).iter().filter(|k| matches!(k, EventKind::Log { .. })).count() as u64;
    assert!(logs > 0 && logs <= 10_000 / HOST_CALL_FUEL, "{logs} log lines");
}

#[test]
fn unbounded_recursion_traps_without_harming_the_host() {
    let src = r#"(module (func $f (export "run") (call $f)))"#;
    let mut svc = load(src, &[]);
    let err = svc.invoke("run").unwrap_err();
    assert!(matches!(err, SandboxError::Trap(_) | SandboxError::OutOfFuel { .. }), "{err}");
    assert!(svc.stop_reason().is_some());
}

#[test]
fn initial_memory_over_cap_is_refused_at_load() {
    let limits = Limits { max_memory_bytes: 2 * WASM_PAGE_BYTES, ..Limits::default() };
    let src = r#"(module (memory (export "memory") 3))"#;
    let err = Sandbox::new()
        .load(&wasm(src), ServiceSpec::new("big", Grants::none()).with_limits(limits), MemKv::new())
        .err()
        .unwrap();
    assert_eq!(
        err,
        SandboxError::ResourceCap(ResourceDenial::Memory {
            requested_bytes: 3 * WASM_PAGE_BYTES,
            cap_bytes: 2 * WASM_PAGE_BYTES,
        })
    );
}

const GROWER: &str = r#"
(module
  (memory (export "memory") 1)
  (global $rc (export "rc") (mut i32) (i32.const 0))
  (func (export "grow_one") (global.set $rc (memory.grow (i32.const 1))))
  (func (export "grow_four") (global.set $rc (memory.grow (i32.const 4)))))
"#;

#[test]
fn memory_growth_past_cap_fails_and_is_recorded() {
    let limits = Limits { max_memory_bytes: 2 * WASM_PAGE_BYTES, ..Limits::default() };
    let mut svc = load_with(GROWER, Grants::none(), limits);

    svc.invoke("grow_four").expect("a failed grow is not a trap");
    assert_eq!(svc.global_i32("rc"), Some(-1));
    assert_eq!(svc.memory_bytes(), WASM_PAGE_BYTES);
    assert!(kinds(&svc).contains(&EventKind::ResourceDenied(ResourceDenial::Memory {
        requested_bytes: 5 * WASM_PAGE_BYTES,
        cap_bytes: 2 * WASM_PAGE_BYTES,
    })));

    svc.invoke("grow_one").unwrap();
    assert_eq!(svc.global_i32("rc"), Some(1), "growth within the cap still works");
    assert_eq!(svc.memory_bytes(), 2 * WASM_PAGE_BYTES);

    svc.invoke("grow_one").unwrap();
    assert_eq!(svc.global_i32("rc"), Some(-1), "the cap is a hard ceiling");
    assert_eq!(svc.memory_bytes(), 2 * WASM_PAGE_BYTES);
}

#[test]
fn table_over_cap_is_refused_at_load() {
    let limits = Limits { max_table_elements: 16, ..Limits::default() };
    let src = r#"(module (table 17 funcref))"#;
    let err = Sandbox::new()
        .load(&wasm(src), ServiceSpec::new("t", Grants::none()).with_limits(limits), MemKv::new())
        .err()
        .unwrap();
    assert_eq!(
        err,
        SandboxError::ResourceCap(ResourceDenial::Table { requested_elements: 17, cap_elements: 16 })
    );
}

fn log_call(ptr: u32, len: u32) -> String {
    format!(
        r#"(module (import "op6" "log" (func $log (param i32 i32 i32)))
          (memory (export "memory") 1)
          (func (export "run") (call $log (i32.const 2) (i32.const {ptr}) (i32.const {len}))))"#
    )
}

#[test]
fn oversize_message_is_refused_before_allocation() {
    let mut svc = load(&log_call(0, u32::MAX), &[Capability::Log]);
    let err = svc.invoke("run").unwrap_err();
    assert!(matches!(err, SandboxError::BadHostCall(ref m) if m.contains("limit")), "{err}");
}

#[test]
fn out_of_bounds_pointer_is_refused() {
    let mut svc = load(&log_call(65_530, 16), &[Capability::Log]);
    let err = svc.invoke("run").unwrap_err();
    assert!(matches!(err, SandboxError::BadHostCall(ref m) if m.contains("out of bounds")), "{err}");
}

#[test]
fn start_function_is_refused() {
    let src = r#"(module (func $init) (start $init))"#;
    let err = Sandbox::new()
        .load(&wasm(src), ServiceSpec::new("s", Grants::none()), MemKv::new())
        .err()
        .unwrap();
    assert_eq!(err, SandboxError::StartFunction);
}

#[test]
fn invalid_bytes_are_refused() {
    let err = Sandbox::new()
        .load(b"\0asm\x01\0\0\0garbage", ServiceSpec::new("g", Grants::none()), MemKv::new())
        .err()
        .unwrap();
    assert!(matches!(err, SandboxError::Compile(_)), "{err}");
}
