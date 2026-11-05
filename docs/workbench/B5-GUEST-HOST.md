# B5 — hello-rs guest → wasm32 + sandbox host runner + TraceEmit refusal test

Worker: WAVE2-G (lane: spark seat). Branch: `b5/guest-host-runner` (based on clean main `33f288d`).
Date: 2026-10-25 ~02:56–03:20 UTC (Oct 24, 20:56–21:20 PT).

## Result

**GREEN on all three acceptance gates** (host toolchain stable 1.99.0, `$HOME/.cargo/bin`, `--offline` throughout):

1. **Guest artifact exists**: `cargo build --offline --release --target wasm32-unknown-unknown` in
   `crates/sandbox/guests/hello-rs` → `hello_rs.wasm`, **1433 bytes** (matches the README's "1.4 KB,
   one memory page" claim), sha256 `ade98b90d6f85193673516d8765b47840454d5eecd4af612d5012ea29f37a012`.
2. **Host tests green**: `cargo test --offline` in `crates/sandbox` → **33 passed, 0 failed, exit 0**
   (previous 31 + 2 new in `tests/guest_host.rs`). Baseline re-run at main state (before the new
   files were added) was also green: 31 passed, exit 0.
3. **aarch64-unknown-redox lib build green**: cookbook-toolchain `cargo build --locked --offline
   --target aarch64-unknown-redox` → exit 0, `Finished dev profile in 15.72s`. No lib/`src/`
   changes were made or needed; the new test file + example do not participate in that build
   (`cargo build` compiles lib + bins only).

New files: `crates/sandbox/examples/run_hello.rs` (minimal host runner) and
`crates/sandbox/tests/guest_host.rs` (the same flow wired as a host integration test — the
"wire as a host integration test if cleaner" option, done so `cargo test --offline` exercises it).

## Step 1 — guest build (wasm32)

```
cd crates/sandbox/guests/hello-rs
cargo build --offline --release --target wasm32-unknown-unknown
```

```
   Compiling hello-rs v0.1.0 (…/crates/sandbox/guests/hello-rs)
    Finished `release` profile [optimized] target(s) in 0.40s
```

Artifact: `crates/sandbox/guests/hello-rs/target/wasm32-unknown-unknown/release/hello_rs.wasm` —
1433 bytes. `target/` is git-ignored (`.gitignore:10`), so the wasm is a recorded artifact, not a
committed one; path + size + sha256 above are the proof.

## Step 2 — host runner + refusal test design

The runner (example `run_hello` and `tests/guest_host.rs` alike) drives two flows:

- **Running**: grants `[Log, KvGet, KvSet, TraceEmit]` → `invoke("run")` completes, journal shows
  `Log{Info,"hello from rust"}`, `KvSet{count,4}` and `Trace{service.hello,{"count":N}}`.
- **Refusal**: `TraceEmit` withheld (grants `[Log, KvGet, KvSet]`) → the guest's `run` traps at
  `trace_emit`; the sandbox records `EventKind::Denied { TraceEmit, NotGranted }`, stops the
  service (no further re-entry — `SandboxError::Stopped`), and the refusal test asserts exactly
  that, plus "no side effect" (no `Trace` event was recorded).

**Gotcha found and fixed during development (honest record):** the first draft granted only
`[TraceEmit, Log]` (per the task brief) and withheld everything else. That refuses on
`kv_get`/`kv_set` *before* `trace_emit` is reached — the refusal event was `Denied{KvGet,…}` and
`run` never emitted a trace. The final grant sets grant every capability the guest's `run` uses
in the running variant, and withhold **only TraceEmit** in the refusal variant, so the refusal
under test is unambiguously `Denied { TraceEmit, NotGranted }`.

Host run of the example (final state, exit 0, exit 0 in the later clean re-run):

```
run_hello: loading guest …/guests/hello-rs/target/wasm32-unknown-unknown/release/hello_rs.wasm
running: fuel_used=727
  Loaded { requested: [Log, TraceEmit, TimerAfter, KvGet, KvSet] }
  Log { level: Info, message: "hello from rust" }
  KvSet { key: "count", value_bytes: 4 }
  Trace { name: "service.hello", payload: "{\"count\":1}" }
refusal: TraceEmit withheld -> denied, no trace emitted, service stopped
ok
```

## Step 3 — host tests green

```
cd crates/sandbox
cargo test --offline          # exit 0
```

Before the new files (baseline at main state): **31 passed** (grants 9, hello 5, limits 11,
rust_guest 2, timers 4), exit 0. After (`guest_host` builds `hello_rs.wasm` itself into its
`CARGO_TARGET_TMPDIR` target dir, same pattern as `rust_guest.rs`):

```
unittests src/lib.rs    test result: ok. 0 passed
tests/grants.rs         test result: ok. 9 passed
tests/guest_host.rs     test result: ok. 2 passed   ← new
tests/hello.rs          test result: ok. 5 passed
tests/limits.rs         test result: ok. 11 passed
tests/rust_guest.rs     test result: ok. 2 passed
tests/timers.rs         test result: ok. 4 passed
Doc-tests sandbox       test result: ok. 0 passed
```

Total: **33 passed, 0 failed** (was 31).

## Step 4 — aarch64-unknown-redox via the cookbook toolchain

Environment (per `B5-REDOX-TARGET.md`, forks untouched — read-only use):

```
R=$HOME/Developer/forks/redox/redox
RI=$R/prefix/aarch64-unknown-redox/rust-install
export PATH=$RI/bin:$R/prefix/aarch64-unknown-redox/sysroot/bin:$PATH
export LD_LIBRARY_PATH=$R/prefix/aarch64-unknown-redox/sysroot/lib
```

**Lib target (first, per brief)** — from `crates/sandbox`:

```
cargo build --locked --offline --target aarch64-unknown-redox
→ Finished `dev` profile [unoptimized + debuginfo] target(s) in 15.72s   (exit 0)
```

Still green at this commit, same as main. `target/aarch64-unknown-redox/debug/libsandbox.rlib` —
18,458,104 bytes (debug info included).

**Example check for the target (recorded, not hacked):**
`cargo build --locked --offline --target aarch64-unknown-redox --examples` → **exit 101**.
`run_hello` *compiles* for the target (its codegen units are real
`ELF 64-bit LSB relocatable, ARM aarch64` objects under
`target/aarch64-unknown-redox/debug/examples/`) but fails at the final link: `/usr/bin/ld` picks
the AArch64 objects up as "Relocations in generic ELF (EM: 183) … file in wrong format". This is
the host binary-linking flow, exactly the gap `B5-REDOX-TARGET.md` already documents ("Linking a
*binary* for the target would additionally need the sysroot's cc/linker flow… a daemon binary on
top of it should go through the redoxer flow"). The host runner is by design a **host-side**
tool (it shells out to `cargo` in tests and reads the host filesystem), so it is not expected to
link for the target; recorded as required, no workaround attempted, `src/` untouched.

## Step 5 — S-1 (Cursor finding): timer re-entry refused after `timer.after` revoke/expiry

**Finding:** `Service::fire_due_timers` delivered `on_timer` callbacks for due timers even when
the `timer.after` grant had been revoked or had expired. The guest's `on_timer` re-enters guest
code and can emit through still-live grants (e.g. `trace_emit`), so a dead capability still
bought a live callback — contrary to the crate rule "the grant check runs before any side
effect".

**Reproduction first (failing regression, recorded raw):** two regression tests added to
`tests/timers.rs` (`timer_firing_after_revoke_is_refused_and_stops_the_service`,
`timer_firing_after_expiry_is_refused_and_stops_the_service`); with the pre-fix
`src/service.rs` they failed at the `fire_due_timers(1_500).unwrap_err()` assertion — the
firing returned `Ok` and the guest was re-entered (globals `last`/`rc` were being written,
`Trace` events emitted) with a dead grant.

**Fix:** `fire_due_timers` now checks `Grants::check(TimerAfter, now)` before computing or
firing due timers, mirroring the per-host-call refusal path: record
`EventKind::Denied { TimerAfter, Revoked|Expired }` + `EventKind::Stopped(…)`, stop the service,
return `Err(SandboxError::Denied { … })`. No guest code runs when the grant is dead; a stopped
service stays non-re-entrant (`Err(Stopped)` on later fires).

**Green after fix (both profiles, raw):**

```
cargo test --offline            → 35 passed, 0 failed, exit 0 (incl. timers.rs 6 passed)
cargo test --offline --release  → 35 passed, 0 failed, exit 0
cargo build --locked --offline --target aarch64-unknown-redox   → Finished in 1.19s, exit 0 (lib still green after the src change)
cargo run --offline --example run_hello                         → ok, exit 0
```

## Boundaries

- Touched only `crates/sandbox/**` (`examples/run_hello.rs`, `tests/guest_host.rs`, this file's
  §Tests line in `README.md`) and `docs/workbench/B5-GUEST-HOST.md`. No `scripts/` edits, no
  forks writes, no phone, no QEMU, no push, no new worktrees.
- `crates/sandbox/Cargo.toml` unchanged (example + test use only `std` + the crate's own
  public API; dev-deps untouched).
- Evidence commands above are the exact ones run from this worktree; tails are raw from tool logs.
