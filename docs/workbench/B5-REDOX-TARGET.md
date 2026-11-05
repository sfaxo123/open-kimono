# B5 — sandbox crate: real `aarch64-unknown-redox` build

Worker: B5 build worker (lane 5090). Branch: `b5/redox-target-build` (based on clean main `9f7247a`).
Date: 2026-10-24 (7:40 PM PT / 2026-10-25 02:40 UTC).

## Result

**GREEN.** `cargo build --locked --manifest-path crates/sandbox/Cargo.toml --target aarch64-unknown-redox`
compiles the sandbox crate and all wasmi dependencies for the real Redox aarch64 target, using the
project's own Redox cookbook toolchain. Host tests remain green: **31 passed, 0 failed**.
**No source, Cargo.toml, or feature changes were required** — the earlier blocker was purely
toolchain selection.

## Step 1 — default toolchain fails (raw tail)

Command (from the worktree root, cargo at `$HOME/.cargo/bin`, stable 1.99.0):

```
cargo build --locked --manifest-path crates/sandbox/Cargo.toml --target aarch64-unknown-redox
```

```
error[E0463]: can't find crate for `core`
  |
  = note: the `aarch64-unknown-redox` target may not be installed
  = help: consider building the standard library from source with `cargo build -Zbuild-std`

error[E0463]: can't find crate for `std`
  |
  = note: the `aarch64-unknown-redox` target may not be installed
  = help: consider building the standard library from source with `cargo build -Zbuild-std`

error: could not compile `bitflags` (lib) due to 1 previous error
warning: build failed, waiting for other jobs to finish...
error: could not compile `wasmi_collections` (lib) due to 1 previous error
error: could not compile `spin` (lib) due to 1 previous error
EXIT: 101
```

## Step 2 — investigation: why, and where the real target std lives

- `rustc --print target-list` (stable 1.99.0) **does** list `aarch64-unknown-redox` — the target
  *spec* exists.
- `rustup target list` offers **only `x86_64-unknown-redox`** as an installable precompiled std.
  There is no rustup component for `aarch64-unknown-redox` std, so `rustup target install` cannot
  fix this — the std for that target is not distributed by rustup.
- The project already built the missing std during W0/E1: the Redox cookbook at
  `$HOME/Developer/forks/redox/redox` (read-only upstream fork, untouched) contains a complete
  standalone Rust toolchain at
  `prefix/aarch64-unknown-redox/rust-install/` with component
  `manifest-rust-std-aarch64-unknown-redox`, i.e. prebuilt
  `libcore-9f692afe645604ed.rlib`, `liballoc`, `libstd` (.rlib **and** .so), `libcompiler_builtins`,
  `libstd_detect` under `lib/rustlib/aarch64-unknown-redox/lib/`.
  This is the same toolchain family `scripts/integrate.sh` uses
  (`COOKBOOK_TOOLCHAIN=/mnt/redox/prefix/aarch64-unknown-redox/sysroot`, `TARGET=aarch64-unknown-redox`).
- One environment fix: the cookbook `rustc` links against the sysroot's LLVM —
  `LD_LIBRARY_PATH=<fork>/redox/prefix/aarch64-unknown-redox/sysroot/lib` is required
  (`libLLVM.so.21.1`), and the toolchain `bin/` must be on `PATH` so cargo finds its own rustc.

## Step 3 — the green build (exact reproduction)

```
R=$HOME/Developer/forks/redox/redox
RI=$R/prefix/aarch64-unknown-redox/rust-install
export PATH=$RI/bin:$R/prefix/aarch64-unknown-redox/sysroot/bin:$PATH
export LD_LIBRARY_PATH=$R/prefix/aarch64-unknown-redox/sysroot/lib
cargo build --locked --manifest-path crates/sandbox/Cargo.toml --target aarch64-unknown-redox
```

Raw output (first full compile, tail):

```
   Compiling libm v0.2.16
   Compiling wasmparser v0.228.0
   Compiling wasmi_ir v2.0.0
   Compiling bitflags v2.13.2
   Compiling wasmi v2.0.0
   Compiling spin v0.9.9
   Compiling wasmi_collections v2.0.0
   Compiling wasmi_core v2.0.0
   Compiling sandbox v0.1.0 ($HOME/Developer/experiments/redox-oneplus6-b5/crates/sandbox)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 20.68s
EXIT: 0
```

Idempotent re-run: `Finished dev profile ... in 0.09s`, exit 0.

Toolchain versions used for the target build:

```
rustc 1.98.0-dev (cookbook custom build, commit-hash unknown)
cargo 1.98.0-dev (8b241a6b6 2026-06-01)
LLVM version: 21.1.2
host: x86_64-unknown-linux-gnu
```

Host toolchain (unchanged, used for host tests): `rustc 1.99.0 (b940084d7 2026-09-28)`,
`cargo 1.99.0 (5f94df478 2026-08-27)`, stable-x86_64-unknown-linux-gnu.

## Artifact proof — real aarch64 objects

```
crates/sandbox/target/aarch64-unknown-redox/debug/libsandbox.rlib
crates/sandbox/target/aarch64-unknown-redox/debug/deps/libsandbox-a2f46ebf1ec7442b.rlib
```

Extracting an object member from the rlib:

```
sandbox.o: ELF 64-bit LSB relocatable, ARM aarch64, version 1 (SYSV), with debug_info, not stripped
readelf -h: Type: REL (Relocatable file)   Machine: AArch64
```

## Host tests still green (31 passed)

```
cd crates/sandbox && cargo test --offline
```

```
unittests src/lib.rs      test result: ok. 0 passed
tests/grants.rs           test result: ok. 9 passed
tests/hello.rs            test result: ok. 5 passed
tests/limits.rs           test result: ok. 11 passed
tests/rust_guest.rs       test result: ok. 2 passed
tests/timers.rs           test result: ok. 4 passed
Doc-tests sandbox         test result: ok. 0 passed
```

Total: **31 passed, 0 failed** (stable 1.99.0, `--offline`).

## What changed and why

- **No code changes.** The sandbox crate (wasmi `=2.0.0`, `default-features = false` + `std`
  feature, `auto-dispatch`, `prefer-btree-collections`) compiles for `aarch64-unknown-redox`
  as-is once a toolchain with the target std is selected.
- `docs/GOTCHAS.md`: the K7 gotcha "aarch64-unknown-redox build stops at missing core/std" now
  records the resolution (cookbook `rust-install` toolchain) instead of only the workaround.
- This file is the evidence record.

## Notes / boundaries

- The upstream fork `$HOME/Developer/forks/redox` was used **read-only**; nothing was written
  to it and nothing was pushed anywhere.
- `crates/sandbox` is a standalone workspace (no root `Cargo.toml`), so target artifacts land in
  `crates/sandbox/target/` (git-ignored).
- Linking a *binary* for the target would additionally need the sysroot's `cc`/linker flow
  (`scripts/integrate.sh` uses `cookbook_redoxer` with `COOKBOOK_SYSROOT`); the sandbox crate is a
  library, so `cargo build` produces the rlib without invoking the linker. A daemon binary on top
  of it should go through the redoxer flow.
