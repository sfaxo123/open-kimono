# caps-check — Luna review follow-up: R8 throttle vs R7 retirement, then the R12 bill

Source review: Codex/Luna review of `560621c`, relayed through the Perplexity parent (see
`$HOME/artifacts/handoff/redox-perplexity/HEARTBEAT-CURRENT.md`, "CAPS lifecycle 560621c" —
"root independently 35 tests debug+release", "Full diff and tests required before merge").
Branch `caps/release-lifecycle` (this capslife worktree, no `main` integration).
Scope: `tools/caps-check/**` + its own docs/evidence only. Agent `dsh-spark` (same session that
wrote `560621c`, per the "do NOT create another CAPS owner" instruction), model
`qwen3.8-flash-next` on the Spark lane, local provider. **No served-model or lane-weight change
was made or needed: nothing in this work dispatches inference, and the `pplx-5090/base4007`
app-guard model was left exactly as found.**

**Bottom line: both review items are closed. 39 tests green in debug *and* release (was 35 on
`560621c`, which is the count the reviewer independently reproduced: 34 in `tests/model.rs` + 1
unit in `src/audit.rs`; the bill fix added 4). Nothing outside `tools/caps-check/**` and the two
docs lines above changed. Nothing pushed, nothing merged to `main`, awaiting independent review.**

| Item | Reproduced? | Commit | Evidence |
| --- | --- | --- | --- |
| R8 — `release` doubled as the safety floors' temporary throttle, and rewound the live grant's on-time meter | **Yes** (repro'd before the previous commit; closed there) | `560621c` | `throttle` keeps the meter: bill `[(1, 4000000000)]` + `spend(2 s)` refused. `release` on a *live* grant still reset it: `[(1, 0)]` + `spend(2 s)` accepted — that was the hole, not the intended retirement semantics. |
| R12 — `spent_by_grant` read only the live `Hold`, so a grant throttled mid-budget vanished from the bill while still live | **Yes** — all four new tests failed `left: []` against a meter reading 4 s / 5 s | `313cb6a` | Bill now folds `self.meters`; `[(grant, 4 s)]` is asserted **between** throttle and re-arm (no live hold exists there), and again after the re-arm so any `hold + meter` double-count would fail. |

## The R12 hole, stated precisely

`spent_by_grant()` scanned `domains[].hold`. Three lifecycle events *close* a hold while leaving
the grant **live** — `throttle` (the floor's lever, R8), `sleep` (C-7), and a budget refusal — so
after any of them the bill came back **empty** for a grant that had genuinely spent on-time. The
pair with `wake_source()` is what made it invisible: `wake_source` answers `None` correctly (there
is no live hold), so "who keeps it powered?" and "what did it cost?" contradicted each other, and
the R12 UI line ("what was spent?") understated the most expensive grant on the phone to zero.

## The invariant to preserve if this is reworked

**The bill reads the durable meter; the live hold is *not* added to it.** `charge()` mirrors the
live hold into the meter on both the `hold` and the `spend` path (`m.on_time = max(old, new)`), so
there is exactly one authoritative number per `(domain, grant)` — the same value the ceiling test
compares against. Reading holds *and* meters, or summing the pair, bills the same seconds twice:
that is the failure mode the "BETWEEN and AFTER" assertions exist to catch, and the reason the
aggregation test pins **one line, summed** (`[(7, 5s)]` for one grant on `adsp`+`codec`, 2 s + 3 s)
rather than one line per rail.

Kept deliberately: `wake_source()` / `is_on()` still read the live hold — those answer "right now".
Kept: `release` clearing the meter (retirement is a reset, otherwise the bill is an unbounded
leak and a new grant cannot get a budget). Kept: **live-only billing** — `release` drops the line.
A permanent per-grant history belongs in the R5 journal (`docs/workbench/B1-TRACEJOURNAL.md`,
`EventKind::PowerOff`/`GrantReleased`), not in a phone-RAM ledger table with no eviction policy.

## Verification (run 2026-10-24 ~23:58 PT / 2026-10-25 06:58 UTC, on the final tree)

```
cargo test --quiet                                                          -> 1 + 38 passed, 0 failed
cargo test --release --quiet                                                -> 1 + 38 passed, 0 failed
cargo clippy --quiet  (tools/caps-check/src/power.rs, tests/model.rs)       -> 0 warnings
```

Release is not decoration here: the ceiling tests depend on `checked_add`/`saturating_add`, and
release disables overflow checks. Verified by probe that an unaddable overspend is still refused
and the per-grant bill still saturates at `i64::MAX` in **both** profiles — no wrap either way.

## Next probes, for whoever integrates

1. Read `tools/caps-check/src/power.rs` `throttle` / `release` / `spent_by_grant`, then run the two
   `cargo test` lines above from the repo root with `--manifest-path tools/caps-check/Cargo.toml`,
   debug and release, and expect `1 + 38`.
2. Independent check of the R12 fix, one command: run `cargo test --test model` and confirm these
   pass — `a_throttled_rail_cannot_be_re_armed_with_a_fresh_budget` (the R8 guard from `560621c`,
   still green, kept so the throttle path stays non-resetting), plus the four R12 tests
   `the_power_bill_survives_the_throttle_that_closed_its_window`,
   `one_grant_on_two_rails_is_one_bill_line_even_while_throttled`,
   `throttling_between_windows_neither_refills_the_budget_nor_blanks_the_bill`,
   `retiring_a_grant_clears_its_bill_line_and_a_new_grant_starts_clean`. The last **four** went
   red on the tree this review was written against (each on `left: []` against a meter reading
   4 s / 5 s); the first was already passing and must not be "simplified" back.
3. `docs/design/CAPS-SANDBOX.md` §8.2 still records the **2026-10-24 historical** count (17 tests)
   for the original delivery; the live count is the 39 above. Do not "correct" §8.2's dated claim —
   it is a record of a past run, and the live number is stated here and in `docs/GOTCHAS.md`.
4. Out of scope and untouched here: `crates/sandbox/**` (F-3 in the earlier Luna round),
   `crates/trace-events`, the QEMU/INTx lanes, and every phone change. No `main` merge: this branch
   waits for the independent review the parent asked for.
