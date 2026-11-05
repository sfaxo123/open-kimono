#!/usr/bin/env python3
"""Exercise the actual preparatory IRQ guard, never a VM or physical device.

Requires cached nightly-2026-05-24/spin 0.9.9 (the kernel lockfile version).
Host fixtures model a thread-local CPU; cross builds prove codegen, not hardware.
Each mutation has its own target directory to prevent stale-binary false passes.
"""
import argparse
import difflib
import json
import os
from pathlib import Path
import shutil
import subprocess


MANIFEST = '''[package]
name = "irq_guard_probe"
version = "0.0.0"
edition = "2021"
publish = false
[lib]
path = "src/lib.rs"
[dependencies]
spin = { version = "=0.9.9", default-features = false, features = ["mutex", "spin_mutex"] }
[workspace]
'''
FAKE = '''use irq_lock::{IrqSaveMutex, MaskPolicy, Saved};
struct Fake;
unsafe impl MaskPolicy for Fake {
    fn save_and_disable() -> Saved { Saved { prior_enabled: false } }
    unsafe fn restore(_: Saved) {}
}
'''


def crate(folder, source, body="", no_std=False):
    (folder / "src").mkdir(parents=True)
    (folder / "Cargo.toml").write_text(MANIFEST)
    (folder / "src/lib.rs").write_text(
        ("#![no_std]\n" if no_std else "")
        + f'#[path = {json.dumps(str(source))}] mod irq_lock;\n' + body)


def run(folder, command, required=(), fails=False):
    result = subprocess.run(command, text=True, stdout=subprocess.PIPE,
                            stderr=subprocess.STDOUT, timeout=180,
                            env={**os.environ, "CARGO_NET_OFFLINE": "true"})
    (folder / "result.log").write_text(result.stdout + f"\nexit={result.returncode}\n")
    if (result.returncode != 0) != fails or any(s not in result.stdout for s in required):
        raise RuntimeError(f"unexpected result: {folder}/result.log")
    print(f"PASS {folder.name}: exit={result.returncode}", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("kernel", type=Path)
    parser.add_argument("evidence", type=Path, help="new directory; refuses reuse")
    parser.add_argument("--cross", action="store_true", help="also emit A64/RV64 asm and objects")
    args = parser.parse_args()
    kernel, out = args.kernel.resolve(), args.evidence.resolve()
    out.mkdir(parents=True, exist_ok=False)
    source = kernel / "src/sync/irq_lock.rs"
    original = source.read_text()
    cargo = [str(Path.home() / ".cargo/bin/cargo"), "+nightly-2026-05-24"]

    def cmd(folder, action="test"):
        return cargo + [action, "--offline", "--manifest-path", str(folder / "Cargo.toml"),
                        "--target-dir", str(folder / "target"), "--jobs", "2"]

    host = out / "production"
    crate(host, source)
    run(host, cmd(host), ["6 passed; 0 failed"])

    mutations = [
        ("restore-before-unlock", '''        let result = {
            let mut guard = self.inner.lock();
            f(&mut *guard)
        };
        drop(token);''', '''        let mut guard = self.inner.lock();
        let result = f(&mut *guard);
        drop(token);
        drop(guard);''', "restore_happens_after_inner_unlock"),
        ("force-enable", "P::restore(self.saved)",
         "P::restore(Saved { prior_enabled: true })", "initially_masked_stays_masked_after"),
    ]
    for name, old, new, expected in mutations:
        if original.count(old) != 1:
            raise RuntimeError(f"mutation anchor changed: {name}")
        folder = out / name
        local = folder / "source/irq_lock.rs"
        local.parent.mkdir(parents=True)
        changed = original.replace(old, new)
        local.write_text(changed)
        shutil.copy2(source.with_name("irq_lock_tests.rs"), local.with_name("irq_lock_tests.rs"))
        (folder / "control.diff").write_text("".join(difflib.unified_diff(
            original.splitlines(True), changed.splitlines(True), fromfile="production", tofile=name)))
        crate(folder, local)
        run(folder, cmd(folder), [f"{expected} ... FAILED", "test result: FAILED"], fails=True)

    cases = [
        ("normal", '''pub fn probe() -> u32 {
 let a = std::sync::Arc::new(IrqSaveMutex::<u32, Fake>::new(1));
 let b = a.clone();
 // SAFETY: inert fake policy; synchronous closure never changes its CPU state.
 std::thread::spawn(move || unsafe { b.with(|v| *v += 1) }).join().unwrap();
 unsafe { a.with(|v| *v) }
}''', (), False),
        ("escape", '''pub fn probe() {
 let a = IrqSaveMutex::<u32, Fake>::new(1);
 let _: &mut u32 = unsafe { a.with(|v| v) };
}''', ("lifetime may not live long enough",), True),
        ("not-send", '''pub fn probe() {
 let a = std::sync::Arc::new(IrqSaveMutex::<std::rc::Rc<u32>, Fake>::new(std::rc::Rc::new(1)));
 std::thread::spawn(move || unsafe { a.with(|v| **v) }).join().unwrap();
}''', ("cannot be sent between threads safely", "Rc<u32>"), True),
        ("unsafe-required", '''pub fn probe() {
 let a = IrqSaveMutex::<u32, Fake>::new(1);
 a.with(|v| *v += 1);
}''', ("E0133", "requires unsafe function or block"), True),
    ]
    for name, body, expected, fails in cases:
        folder = out / name
        crate(folder, source, FAKE + body)
        run(folder, cmd(folder, "build"), expected, fails)

    if args.cross:
        for arch in ("aarch64", "riscv64"):
            folder = out / arch
            crate(folder, source, '''use irq_lock::{ArchMask, IrqSaveMutex};
static PROBE: IrqSaveMutex<usize, ArchMask> = IrqSaveMutex::new(0);
// Compile-only. Any real caller must prove IRQ/CPU/scheduling obligations.
#[no_mangle]
pub unsafe extern "C" fn probe(x: usize) -> usize {
 unsafe { PROBE.with(|v| { *v += x; *v }) }
}
''', no_std=True)
            run(folder, cmd(folder, "rustc") + ["--lib", "--release", "--target",
                str(kernel / f"targets/{arch}-unknown-kernel.json"),
                "-Z", "build-std=core,alloc", "-Z", "build-std-features=compiler-builtins-mem",
                "-Z", "json-target-spec", "--", "--emit=asm,obj"])
    print("PASS preparatory guard only; no IRQ_CHIP wiring, runtime or hardware proof")


if __name__ == "__main__":
    main()
