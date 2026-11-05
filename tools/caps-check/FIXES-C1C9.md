# caps-check — C-1…C-9 fix evidence (Cursor audit 2026-10-25)

Source of findings: `$HOME/artifacts/handoff/redox-perplexity/CURSOR-AUDIT-20261025.md` §2
(lines 174-290 carry the traces). Branch `fix/caps-check-c1c9`, one commit per finding, each
message carrying its own red-run excerpt. Scope: `tools/caps-check/**` only — `crates/sandbox`,
`crates/wyoming`, `scripts/`, `docs/HANDOFF.md` untouched.

## Gate, as corrected

`tools/caps-check` has exactly one integration target, `tests/model.rs`. The originally stated
`--test grants` / `--test timers` targets **do not exist in this crate** (they belong to the
separate `crates/sandbox` owner), so the gate actually run for every step was:

```
cargo test --offline --locked --manifest-path tools/caps-check/Cargo.toml
cargo test --offline --locked --release --manifest-path tools/caps-check/Cargo.toml
```

Every finding followed the same order: write the regression, **observe it red**, fix, re-run both
profiles green, commit. Test counts moved 21 → 30 (one test per finding; the doc test and the
unit-test target stayed at 1 and 0).

| step | debug | release |
| --- | --- | --- |
| baseline (clean tree, before any change) | 21 passed | 21 passed |
| after C-1 | 22 | 22 |
| after C-2 | 23 | 23 |
| after C-3 | 24 | 24 |
| after C-4 | 25 | 25 |
| after C-5 | 26 | 26 |
| after C-6 | 27 | 27 |
| after C-7 | 28 | 28 |
| after C-8 | 29 | 29 |
| after C-9 (final) | **30** | **30** |

---

## C-1 — zero-byte connect rode a spent `Bytes` budget — `d38dfcd`

`cost_of` metered `Want::Connect { bytes: 0 }` at zero, so a spent `Bytes` budget let an unlimited
number of zero-byte connects through — each of which still brings the modem rail up.

Test: `zero_byte_connect_cannot_ride_a_spent_budget`

```
thread 'zero_byte_connect_cannot_ride_a_spent_budget' panicked at tests/model.rs:1159:
assertion `left == right` failed
  left: Allowed { grant: 1, op: 2 }
 right: Denied(BudgetSpent)
```

Fix: `cost_of` requires `bytes > 0` for a connect, mirroring the existing `dur > 0` rule for a
stream — a cost has to be positive to exist.

## C-2 — duplicate `uses` entries multiplied the budget — `6f279bc`

`mint` mints one grant per `uses` entry and `check` falls through to the next affordable grant when
the first is spent, so two entries with the same `(cap, scope)` are a doubled budget written in
plain manifest syntax — while `Manifest::uses()` shows only the first, so the reviewer sees half
the authority.

Test: `duplicate_uses_entries_are_refused_not_a_double_budget`

```
thread 'duplicate_uses_entries_are_refused_not_a_double_budget' panicked at tests/model.rs:1179:
C-2: duplicate (cap,scope) must be refused — one authority, one budget
```

Fix: `validate` counts declarations and refuses a repeated `(cap, scope)`. The test pins the
non-regression: the same cap for two *different* exact destinations still validates clean.

## C-3 — exclusive device handed to two holders — `c46bbaf`

`check` set `contended = true` but only reported it when *nothing* was affordable. Two services
that each held an exclusive `audio.capture` grant for the same mic were both Allowed.

Test: `two_holders_cannot_both_run_on_one_exclusive_device`

```
thread '...' panicked at tests/model.rs:1248:
assertion `left == right` failed: C-3: one microphone, one holder ...
  left: Allowed { grant: 2, op: 2 }
 right: Denied(Contended)
```

Fix: `check` tracks the earliest-minted live exclusive grant of a *different* service for that
kind+scope (grant numbers are never reused, so the number is arrival order) and refuses any later
holder with `Denied(Contended)`. Enforcement sits at the decision point: minting the second
declaration stays legal so the audit trail keeps showing it, but its *use* is refused with the
reason that names the conflict. The pre-existing single-holder test and the first holder's own use
both stay green.

## C-4 — `mode=` typo failed open to read-write — `39896c3`

`read_only` was `matches!(get("mode"), Some("ro"))`, so `mode="read-only"`, `readonly`, `RO`, `r`
all landed on read-**write**: the typo bought more authority than the author asked for, and R3's
ceiling then let that grant cover write requests.

Test: `an_unknown_mode_is_refused_not_read_write`

```
thread '...' panicked at tests/model.rs:1263:
assertion `left == right` failed: C-4: `mode="read-only"` is not a mode the host can check, so it must grant nothing
  left: Area { area: "nav", read_only: false }
 right: Empty
```

Fix (R9 — an unknown shape is an error, never a shrug): an unrecognised `mode=` yields
`Scope::Empty`, which manifest validation refuses; `validate` additionally names the offending
`mode=` value so the author sees the typo rather than a mysterious empty scope. `mode=ro`,
`mode=rw` and no `mode=` keep their existing meaning.

## C-5 — negative spend refunded an on-time budget — `bab29e7`

`WakeLedger::spend` never checked the sign of `dur`: `on_time + dur > max_on` is trivially
satisfied by a negative cost, and the meter then *subtracted* it.

Test: `a_negative_spend_cannot_refund_a_spent_on_time_budget` (the audit's own trace)

```
thread '...' panicked at tests/model.rs:1315:
C-5: a negative spend must be refused, not credited back to the meter
```

Fix: a spend is refused unless it is a positive amount of on-time, mirroring `cost_of`'s `dur > 0`;
a refused spend drops the block to `Lp` like every other refusal. Zero-length spends are refused too.

## C-6 — hold re-entry reset the on-time meter — `e3b7c02`

`hold` wrote `on_time: 0` unconditionally, so re-taking the hold before each wake window handed the
budget back: `wake = "on", max = "5m"` behaved as "on forever".

Test: `re_taking_a_hold_cannot_reset_the_on_time_meter`

```
thread '...' panicked at tests/model.rs:1337:
C-6: re-taking the hold for the same grant cannot mint a fresh budget
```

Fix: the meter belongs to the grant. Re-entering a live hold for the same grant carries its
`on_time` (and may extend its own deadline); a grant whose carried meter already meets `max_on` is
refused and dropped to `Lp` rather than re-armed. The test also pins the carry-over itself
(`spent_by_grant` still shows 3 s after the re-entry) and that a *different* grant is a new
authority that may hold afresh.

## C-7 — `Hold.deadline` was never enforced — `ef657a3`

The deadline was stored, documented as absolute monotonic, and compared to nothing: `sleep(dt)`
only checked on-time, so a hold with a 10 s deadline and 60 s of budget stayed `Fm` through an hour
of sleep — the exact "deep sleep freezes the timer, the block stays powered" failure R7 forbids.

Test: `a_wake_window_dies_at_its_deadline_not_only_at_its_budget`

```
thread '...' panicked at tests/model.rs:1388:
  left: Fm
 right: Lp
```

Fix: the ledger carries its own monotonic clock, advanced by `sleep`, and the deadline is enforced
at all three points — `sleep` expires holds whose deadline passed, `hold` refuses a deadline that
is already behind, `spend` charges nothing after it. `now()` exposes the clock, because a deadline
that cannot be read cannot be audited.

**Deviation from the audit's literal expectation:** it asserted `!is_on("apps")` after the deadline
passes. `is_on` counts `Lp` as on, because `Lp` is the designed idle state with the wake input
still armed (R12). The test asserts the real enforcement instead: the block drops out of `Fm`,
`wake_source` is `None`, and the closed window charges nothing with 59 s of budget unspent.

## C-8 — unchecked adds wrapped in `--release` — `bbcbaa6`

Every addition in the wake ledger was unchecked. In debug the suite panicked; in `--release`
`on_time + dur` wrapped negative, so an unaddable overspend read as *affordable* and the on-time
budget stopped being a ceiling. The per-grant bill could wrap negative too — "this grant cost
nothing" about the most expensive grant on the phone.

Test: `power_arithmetic_cannot_wrap_past_its_ceiling` (red captured by restoring the pre-fix
`power.rs` from HEAD)

```
debug:   panicked at src/power.rs:196:74: attempt to add with overflow
release: panicked at tests/model.rs:1422:
         C-8: `on_time + dur` must not wrap past the ceiling — in release this *accepted* the overspend
```

Fix: the ceiling tests use `checked_add` and treat overflow as overspend; `spent_by_grant`
saturates. The release red run stops at the first sub-case, so the `sleep` and per-grant-bill
sub-cases are pinned in the same test as the same class of wrap rather than observed individually.

## C-9 — a deadline was accepted as a budget — `5146762`

`budget_covers` demanded a power-bounding budget only when the grant had *no* deadline, so a
hand-built `model.infer` grant with `Budget::Unlimited` and any deadline was unlimited inference
until the clock ran out. `mint` refuses to write one, but the suite's own rule is that the runtime
check fails closed on a bypassed mint — and that coverage existed only for `audio.capture`.

Test: `a_deadline_is_not_a_budget_for_a_power_spending_kind`

```
thread '...' panicked at tests/model.rs:1467:
(assertion failed: !g.budget_covers(&Want::Use, 0))
```

Fix: a power-spending kind is refused `Budget::Unlimited` deadline or no deadline — a deadline
bounds a lifetime, R4's "never unlimited" bounds the budget. The test also drives the same grant
through `check` (refused as `BudgetSpent`, R5) and pins that `Budget::Uses(3)` still covers, so the
fix refuses unbounded power rather than inference.

**Deviation, caught by the suite:** the first attempt dropped the `deadline.is_none()` condition
entirely and required `bounds_power_for` in all cases, which broke
`budgets_are_ceilings_not_suggestions` — a 500 ms/s duty cycle inside a 2 s deadline is a
legitimate power budget. The unconditional refusal is therefore scoped to `Unlimited`, and the
existing no-deadline guard is kept.

---

## Final verification

```
$ cargo test --offline --locked --manifest-path tools/caps-check/Cargo.toml
test result: ok. 30 passed; 0 failed; 0 ignored  (tests/model.rs)
test result: ok. 1 passed; 0 failed              (doc-test target)

$ cargo test --offline --locked --release --manifest-path tools/caps-check/Cargo.toml
test result: ok. 30 passed; 0 failed; 0 ignored  (tests/model.rs)
test result: ok. 1 passed; 0 failed              (doc-test target)
```

Working tree clean on `fix/caps-check-c1c9`; nine commits, one per finding. Not touched:
`crates/sandbox`, `crates/wyoming`, `scripts/`, `docs/HANDOFF.md`.

## Not fixed here (out of scope for C-1…C-9)

- **C-10** ("finite" has no ceiling: `window: "2562047h"` passes `validate`) needs a platform
  ceiling defined first — the audit itself says the regression should exist "once a platform ceiling
  is defined". Left for the design owner rather than invented here.
- **C-11 / C-12** live outside `tools/caps-check` and were not touched.
