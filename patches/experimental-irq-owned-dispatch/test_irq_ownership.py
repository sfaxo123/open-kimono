#!/usr/bin/env python3
"""Experimental UNWIRED Stage B seam: compile the ACTUAL kernel src/dtb/irq_dispatch.rs and
src/sync/irq_lock.rs with mock core/controllers. Host only; proves no target
asm, runtime, SMP or hardware behaviour. Each case has its own target dir."""
import json, os, subprocess, sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
MANIFEST = '''[package]
name = "irq_ownership_probe"
version = "0.0.0"
edition = "2024"
publish = false
[lib]
path = "src/lib.rs"
[dependencies]
spin = { version = "=0.9.9", default-features = false, features = ["mutex", "spin_mutex"] }
[workspace]
'''
CARGO = [str(Path.home() / ".cargo/bin/cargo"), "+nightly-2026-05-24"]


def crate(folder, lock_src, dispatch_src, body="", tests=True):
    (folder / "src").mkdir(parents=True)
    (folder / "Cargo.toml").write_text(MANIFEST)
    (folder / "src/lib.rs").write_text(
        "#![allow(dead_code)]\nextern crate alloc;\n"
        f"pub(crate) mod sync {{ #[path = {json.dumps(str(lock_src))}] pub(crate) mod irq_lock; }}\n"
        f"#[path = {json.dumps(str(dispatch_src))}] pub(crate) mod irq_dispatch;\n"
        + (f"#[cfg(test)] #[path = {json.dumps(str(HERE / 'ownership_tests.rs'))}] mod ownership_tests;\n" if tests else "")
        + body)


def run(folder, action, extra, fails, required):
    cmd = CARGO + [action, "--offline", "--manifest-path", str(folder / "Cargo.toml"),
                   "--target-dir", str(folder / "target"), "--jobs", "2"] + extra
    try:
        r = subprocess.run(cmd, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                           timeout=180, env={**os.environ, "CARGO_NET_OFFLINE": "true"})
        out, code = r.stdout, r.returncode
    except subprocess.TimeoutExpired as e:
        out, code = (e.stdout or b"").decode() if isinstance(e.stdout, bytes) else (e.stdout or ""), "timeout"
    (folder / "result.log").write_text(out + f"\nexit={code}\n")
    ok = (code != 0 if fails else code == 0) and code != "timeout" and all(s in out for s in required)
    print(f"{'PASS' if ok else 'FAIL'} {folder.name}: exit={code}", flush=True)
    return ok


MUTANTS = [  # (name, old, new, test filter, required failure text)
    ("callback-under-guard",
     "                handler.handle(virq, token);\n                let eoi",
     "                unsafe { self.with_ready(|_| handler.handle(virq, token)) }?;\n                let eoi",
     "callback_runs_outside_core_guard", "callback invoked under core guard"),
    ("omit-handler-eoi", "                if eoi {", "                if false && eoi {",
     "opt_in_eoi_completes_copied_target_once", "opt_in_eoi_completes_copied_target_once"),
    ("reverse-tlmm-clear",
     "    for pin in from..enabled.len() {\n        if enabled[pin] && pending(pin) {\n            clear(pin);\n",
     "    if from > 0 { clear(from - 1); }\n    for pin in from..enabled.len() {\n        if enabled[pin] && pending(pin) {\n",
     "tlmm_ascending_clear_before_each_event", "tlmm_ascending_clear_before_each_event"),
    ("bcm-normal-fallback",
     "                    let handler = unsafe { self.with_ready(|core| core.handler(virq)) }?;\n"
     "                    if let Some(handler) = handler {\n"
     "                        handler.handle(virq as u32, token);\n                    }\n",
     "                    unsafe { self.dispatch(virq as u32, token, sink) }?;\n",
     "bcm_registered_only_then_self_eoi", "bcm_registered_only_then_self_eoi"),
    ("plic-extra-eoi",
     "                ControllerStep::Dispatch { virq } => return unsafe { self.dispatch(virq, token, sink) },",
     "                ControllerStep::Dispatch { virq } => { unsafe { self.dispatch(virq, token, sink) }?; 0 }",
     "plic_translated_nested_without_extra_eoi", "plic_translated_nested_without_extra_eoi"),
    ("omit-reservation-check",
     "            if core.is_reserved(virq as usize) {",
     "            if false && core.is_reserved(virq as usize) {",
     "duplicate_reservation_is_atomic", "duplicate_reservation_is_atomic"),
]
HDR = ("use crate::irq_dispatch::*;\nuse crate::sync::irq_lock::{MaskPolicy, Saved};\n"
       "struct F; unsafe impl MaskPolicy for F { fn save_and_disable() -> Saved { Saved { prior_enabled: false } }"
       " unsafe fn restore(_: Saved) {} }\n")
CORE = ("struct C<T>(T); impl<T> DispatchCore<()> for C<T> { type Handler = dyn IrqHandler<()>;"
        " fn route(&mut self, _: u32) -> Route<Self::Handler> { Route::Scheme }"
        " fn handler(&mut self, _: usize) -> Option<alloc::sync::Arc<Self::Handler>> { None }"
        " fn slot(&mut self, _: usize) -> &mut Option<alloc::sync::Arc<Self::Handler>> { unimplemented!() }"
        " fn begin(&mut self, _: usize, _: u32) -> ControllerStep { ControllerStep::Done }"
        " fn resume(&mut self, _: usize, _: u32) -> ControllerStep { ControllerStep::Done }"
        " fn complete(&mut self, _: Completion) {} fn is_reserved(&mut self, _: usize) -> bool { false }"
        " fn set_reserved(&mut self, _: usize, _: bool) {} }\n")
FIXTURES = [  # (name, body, fails, required)
    ("compile-ok-send-core", HDR + CORE + "static R: IrqRegistry<C<u32>, F> = IrqRegistry::new();\n", False, ()),
    ("compile-fail-not-send-core", HDR + CORE +
     "static R: IrqRegistry<C<*mut u8>, F> = IrqRegistry::new();\n", True, ("cannot be sent between threads",)),
    ("compile-fail-not-sync-handler", HDR +
     "struct H(core::cell::Cell<u32>); impl IrqHandler<()> for H { fn handle(&self, _: u32, _: &mut ()) {} }\n",
     True, ("cannot be shared between threads",)),
]
# Appended inside a copy of the module so the private HRTB facade is reachable.
ESCAPE = ("\nimpl<C: DispatchCore<()>, P: MaskPolicy> IrqRegistry<C, P> {\n"
          "    fn leak(&self) -> &mut Option<Arc<C::Handler>> {\n"
          "        unsafe { self.with_ready(|c| c.slot(0)) }.unwrap()\n    }\n}\n")


def main():
    kernel, out = Path(sys.argv[1]).resolve(), Path(sys.argv[2]).resolve()
    out.mkdir(parents=True, exist_ok=False)
    lock, src = kernel / "src/sync/irq_lock.rs", kernel / "src/dtb/irq_dispatch.rs"
    ok = run_case(out / "prototype", lock, src, "test", [], False, ("18 passed; 0 failed",))
    text = src.read_text()
    for name, old, new, filt, req in MUTANTS:
        assert text.count(old) == 1, f"mutant anchor drifted: {name}"
        mutant = out / name / "irq_dispatch.rs"
        mutant.parent.mkdir(parents=True)
        mutant.write_text(text.replace(old, new))
        ok &= run(build(out / name / "crate", lock, mutant), "test", ["--", "--exact",
                  f"ownership_tests::{filt}"], True, (req, "test result: FAILED. 0 passed; 1 failed", f"ownership_tests::{filt} ... FAILED"))
    for name, body, fails, req in FIXTURES:
        ok &= run_case(out / name, lock, src, "check", [], fails, req, body=body, tests=False)
    escape = out / "compile-fail-guard-borrow-escape" / "irq_dispatch.rs"
    escape.parent.mkdir(parents=True)
    escape.write_text(text + ESCAPE)
    ok &= run(build(escape.parent / "crate", lock, escape, tests=False), "check", [], True,
              ("lifetime may not live long enough",))
    print("PASS all" if ok else "FAIL some", "(host seam only; no target/runtime/SMP proof)")
    sys.exit(0 if ok else 1)


def build(folder, lock, src, body="", tests=True):
    crate(folder, lock, src, body, tests)
    return folder


def run_case(folder, lock, src, action, extra, fails, req, body="", tests=True):
    return run(build(folder, lock, src, body, tests), action, extra, fails, req)


if __name__ == "__main__":
    main()
