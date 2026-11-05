# WY harness lifetimes — Cursor review W-1/W-3/W-4/W-5 (+W-6/W-7/W-8)

Owner: WAVE2-WY (client5090) · Branch `codex/wy-harness-lifetimes` off main
`193ca54` (retains Claude fixes `0dd4111`/`aa16f2f`/`56ddd03`) · 2026-10-25
Scope: `crates/wyoming/**` + this doc only. Source review:
`$HOME/artifacts/handoff/redox-perplexity/CURSOR-WY-REVIEW-20261025.md`.

## Bottom line

The pyinterop gate is now lifetime-safe: the python child is killed+reaped on
every early exit (including panics), every readiness/teardown read is bounded
by an explicit deadline, child stderr is drained and surfaced in failures, the
imported reference is *verified* (path + recorded git commit), and an explicit
`WYOMING_PYINTEROP=required` mode makes a silent skip impossible for the
strict gate. All four review weaknesses were reproduced red on a scratch copy
*before* the fix, and each now has a committed fault-injection regression
test. Full wyoming suite green debug **and** release (83 tests, was 78), no
leftover child processes after any run.

## 1. Reproductions (pre-fix, scratch copy `/tmp/wylife/exp`, repo untouched)

Method: copy of `crates/wyoming` with the fault injected in the *scratch*,
`cargo test --no-run`, then the test binary under `timeout -k 3 N`.

| # | Injection | Result (raw) |
|---|-----------|--------------|
| W-1 | removed `let _ = self.framed.shutdown();` from `Client::close()` | `timeout -k 3 45` → **exit=124 elapsed=45s** — the untimed `SERVED` `read_line` hangs forever (`/tmp/wylife/w1.log`) |
| W-3 | scratch test connects to `port + 1` → panic before `server.wait()` | exit=101, then **orphan `1986673 python3 /tmp/wylife/exp/tests/pyinterop_server.py`** blocked in `accept()` (`/tmp/wylife/w3.log`) |
| W-5 | `WYOMING_PYREF=/tmp/wylife/stallref` (reference `wyoming/__init__.py` sleeps 300s) | `timeout -k 3 30` → **exit=124 elapsed=30s** — untimed `PORT` `read_line` hangs (`/tmp/wylife/w5.log`) |
| W-4 | scratch `pyref_meta` stub removed → child dies with `PackageNotFoundError` | test failed with `"python server died before printing PORT line"` but **0 occurrences of `PackageNotFoundError`** in the output — traceback swallowed by the never-drained stderr pipe (`/tmp/wylife/w4.log`) |

## 2. The fix (`crates/wyoming/tests/pyinterop.rs`, rewritten harness)

* **RAII child guard** (`ChildGuard`): `Drop` → `kill()` + `wait()`, so any
  early exit — including panic unwind — kills and reaps the python child
  (W-3). Belt-and-braces on the python side: the reference server now sets
  `settimeout(120)` on `accept()`/connection.
* **Bounded reads** (`LineReader`): stdout is moved to a helper thread and
  every `PORT`/`REF`/`SERVED` wait uses `recv_timeout(deadline)`
  (PORT/REF 15 s, SERVED 15 s, exit wait 5 s via `try_wait` polling). A
  close() regression now fails red in ≤15 s instead of hanging the suite
  (W-1, W-5).
* **Stderr drain** (`StderrLog`): child stderr is collected on a helper
  thread and appended to every harness failure message — tracebacks are
  never swallowed and the 64 KiB pipe can never deadlock the child (W-4).
* **Committed fault-injection tests** (`tests/pyinterop_fault_server.py`,
  byte-level framing, no reference import needed):
  `fault_stalled_port_fails_bounded` (W-5), `fault_stuck_served_fails_bounded`
  (W-1 — simulates a close() that never delivers EOF),
  `fault_crash_stderr_preserved` (W-4 — asserts the crash marker appears in
  the panic text), `guard_kills_and_reaps_child_on_drop` (W-3). Each asserts
  the failure arrives within `deadline + 5 s` and that
  `pgrep -f <unique tag>` finds **no surviving process**.

## 3. Gate modes (0dd4111 skip behavior preserved; `required` added)

`WYOMING_PYINTEROP=required` is the strict-gate mode for the orchestrator;
unset keeps the portable default exactly as 0dd4111 shipped it. Demonstrated
against the built test binary:

```
A) WYOMING_PYINTEROP=required WYOMING_PYREF=/nonexistent/ref  -> FAILED (exit 101)
   "python reference checkout missing at /nonexistent/ref (WYOMING_PYREF is an explicit pin)"
B) WYOMING_PYINTEROP=required HOME=/tmp/nohome               -> FAILED (exit 101)
   "WYOMING_PYINTEROP=required but no python reference at ... — the 59-event
    gate must not be skipped (set WYOMING_PYREF)"
C) default (unset) HOME=/tmp/nohome                          -> ok, visible:
   "pyinterop: SKIPPED — no reference at ... (set WYOMING_PYREF)"
D) WYOMING_PYINTEROP=required + real reference               -> actually runs:
   "pyinterop: reference commit 08a0c258df83fea6e1643bcfe67e86243f01cf31"
   "pyinterop: exchanged 59 sent / 59 received; python EOF + exit 0"
```

A missing `python3` in `required` mode is likewise a hard failure
(`require_python3`).

## 3. Reference provenance (W-6 — recorded, not assumed)

The gate no longer trusts the stub metadata version. The server prints
`REF <wyoming.__file__> <version>` before `PORT`; the Rust side asserts the
imported package directory **is** `<WYOMING_PYREF>/wyoming` (canonicalized)
and records the checkout's actual git commit. Ground truth on this box:

* path: `$HOME/Developer/forks/op6-refs/wyoming` (via the 0dd4111
  default; `WYOMING_PYREF` overrides)
* **commit: `08a0c258df83fea6e1643bcfe67e86243f01cf31`** ("Cleanup", fork of
  rhasspy/wyoming; tree clean per the review probe)
* `metadata version 1.11.0` is **stub-supplied** by
  `tests/pyref_meta/wyoming-1.11.0.dist-info/METADATA` solely to satisfy
  `importlib.metadata.version("wyoming")`; it is reported, never asserted.

## 4. Library-side fixes in scope

* **W-8** (`src/transport.rs`): `Framed::shutdown()` now attempts the sink
  shutdown even when the flush fails (first error still returned), so the
  peer always observes EOF. Regression test
  `transport::tests::shutdown_runs_even_when_flush_fails` (mock sink whose
  flush errors; asserts `shutdown` ran and the flush error surfaced).
* **W-7** (documented, not changed): the `Sink::shutdown` default no-op is
  now explicitly flagged as a footgun — real transports must override it.

## 5. Verification

* Real gate against the actual reference: **59 sent / 59 received, python
  EOF + exit 0**; identity verified (see §3).
* Induced failures: all four fault tests assert bounded failure
  (`deadline + 5 s` cap) **and** `pgrep -f <unique tag>` finds no survivor.
* `pgrep -af 'pyinterop[_]'` after every run: empty (exit 1).
* Full wyoming suite: **debug 83/83, release 83/83** (unit 16, framing 18,
  golden 2, loopback 3, pyinterop 5, typed 39, doc 0). `cargo test` and
  `cargo test --release` exit 0.

## 6. Handoff notes

* Integration + strict-gate wiring (setting `WYOMING_PYINTEROP=required` in
  the CI/orchestrator gate) belongs to the Perplexity orchestrator — this
  branch only provides the mode. No merge/push performed here.
* X-1 (`integrate.sh` pointer) is Luna's; untouched. No scripts/, sandbox,
  caps, fork, or phone writes.
* W-10 (pyinterop single-threaded exchange) intentionally untouched: the
  golden-fixture sequence is the contract.

---

# Phase 2 — gate-mode parsing hardened (branch `codex/wy-harness-modes`)

Follow-up (same WY owner, from merged main `ac1f535`): the strict gate
previously treated **any** non-`required` value — typos, wrong case,
garbage — as the optional default, so a misconfigured strict gate silently
degraded into the missing-ref skip.

## 2.1 Accepted values (documented from actual callers)

| `WYOMING_PYINTEROP` | Mode | Caller |
|---|---|---|
| unset / empty | Optional — visible skip when reference absent (0dd4111 preserved) | developer machines, CI without a reference checkout |
| `required` | Strict — 59-event exchange must actually run | Perplexity orchestrator strict gate |
| anything else | **Rejected**: hard fail naming the value + accepted set | (misconfiguration) |

## 2.2 Change (`crates/wyoming/tests/pyinterop.rs` only)

* Pure `parse_gate_mode(Option<&str>) -> Result<GateMode, String>`;
  `gate_mode()` reads the env once and panics on unknown values (incl.
  non-UTF-8). `ref_dir()` parses the mode **first**, so an invalid value
  fails even when the reference happens to exist.
* Pure regression `gate_mode_parsing_rejects_unknown_values`: unset/empty →
  Optional, `required` → Required, and 8 bad values (`Required`, `REQUIRED`,
  `reqired`, `strict`, `skip`, `true`, `1`, `"required "`) each rejected with
  the offending value named. No process-env mutation — safe alongside the
  env-sensitive tests.

## 2.3 Demonstrations (built binary)

```
WYOMING_PYINTEROP=reqird   (reference EXISTS) -> FAILED exit 101:
  'WYOMING_PYINTEROP="reqird" is not a recognized gate mode (accepted: unset
   for the optional default, "required" for the strict gate)'
WYOMING_PYINTEROP=Required                    -> FAILED exit 101 (same shape)
WYOMING_PYINTEROP=required + real reference   -> ok:
  'pyinterop: exchanged 59 sent / 59 received; python EOF + exit 0'
                                          (reference commit 08a0c258df83…)
```

## 2.4 Flake hardening found along the way

Two intermittent failures were observed while verifying (one release run,
one debug run; never reproduced in 8 subsequent full/binary runs):

* lower-bound timing asserts (`elapsed >= deadline`) — `recv_timeout` can
  wake a hair early; asserts now carry 100 ms slack;
* one real-gate readiness wait near the 15 s mark under load — the real
  gate's REF/PORT waits now get a 30 s `READY_TIMEOUT` (python cold start
  under fleet load); fault-injection tests deliberately keep the tight 15 s
  deadline since they *are* the bounded-failure proof.

## 2.5 Verification

* pyinterop binary: 6/6 (new pure parse test included).
* Full wyoming suite: **84/84 green, debug and release** — 5 consecutive
  full runs after hardening (3 debug + 2 release), plus standalone binary
  runs; `pgrep -af 'pyinterop[_]'` empty after every run.
