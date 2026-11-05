# caps-check — Luna merge review F-1 / F-2

Source review: `$HOME/artifacts/handoff/redox-perplexity/LUNA-MERGE-REVIEW-20261024-2102.md`
(review of `f12d668..5a5f83f`, static reading only — both findings were re-verified by execution here).
Branch `fix/caps-luna-f1f2` off `origin/main` `4442937`. Scope: `tools/caps-check/**` only.
Agent `dsh-5090`, model `qwen3.8-flash-next-iq3_s`, local provider.

**Bottom line: both findings are real, both reproduced red against `4442937` before any code was
touched, both fixed, one commit each, and the whole gate is green in debug and release.**
32 integration tests + 1 unit test, 0 failed, both profiles.

| Finding | Reproduced? | Commit |
| --- | --- | --- |
| F-1 (P1) same-grant hold resets the exhausted budget | **Yes** — second re-hold accepted | `a978772` |
| F-2 (P1) exclusive arbitration misses same-service distinct-instance holders | **Yes** — `Allowed { grant: 2, op: 2 }` where `Denied(Contended)` is required | `fd94845` |

F-3 in the same review sits in `crates/sandbox/src/service.rs:232-252` — **out of scope here, not
touched.** Nothing outside `tools/caps-check/**` changed.

---

## F-1 — a refused hold could be retried into a fresh budget (`power.rs`)

**The hole.** C-6 put the on-time meter inside the domain's live `Hold` and carried it across a
re-hold. But *both* refusal paths clear that hold — the re-hold that finds the budget spent, and
the spend that would overspend — so the meter became unreachable the moment it mattered. The next
call carrying the same grant id fell into the fresh-hold branch with `on_time: 0`. Retrying a
refused call was sufficient to defeat `wake = "on", max = "5m"`.

**Red, before the fix** (debug and release, same panic; the review's own
`caps-repeat-hold-repro.rs` reaches the same place, exit 101):

```
thread 'a_refused_hold_cannot_be_retried_into_a_fresh_budget' panicked at tests/model.rs:1518:5:
F-1: and the *second* re-hold is refused too — the meter has to outlive the hold it closed
```

**The fix.** The meter belongs to the grant, not to the call, and now outlives the hold it closed.
A private `Meter { domain, grant, on_time, closed }` per (domain, grant):

- `hold` carries the **larger** of the live hold's `on_time` and the durable record — never a reset;
- a record marked `closed` (already refused on budget) refuses every later hold under that grant;
- a spend refused **on budget** marks the grant's record `closed`, so a later hold or spend under
  the same grant is refused too rather than starting from zero;
- `sleep` ends a *window*, not a budget — the record survives deliberately;
- `release(domain)` — what the caller does when the grant behind the hold dies (R7: expired,
  revoked, spent, or the service stopped) — drops the domain's records, and is the only reset.

A malformed `dur <= 0` spend still closes the window (existing behaviour) but does not permanently
close the grant; only a budget exhaustion does. A different grant id is a different authority, and
the C-6 case where a live grant with budget left extends its own deadline still passes.

**Green:**

```
cargo test --offline --locked --manifest-path tools/caps-check/Cargo.toml          -> 31 passed, 0 failed
cargo test --offline --locked --release --manifest-path tools/caps-check/Cargo.toml -> 31 passed, 0 failed
```

(31 at that commit: the 30 prior integration tests plus this regression; the F-2 test landed in
the next commit.)

---

## F-2 — two generations of one service both held one exclusive device (`grant.rs`)

**Honest reproduction status: REPRODUCED.** The review asked for the violation to be reproduced
first and stopped if it did not appear. It appeared on the first run against `4442937`, with the
test's own minted-grant ids:

```
thread 'one_service_two_generations_are_still_one_holder' panicked at tests/model.rs:1603:5:
assertion `left == right` failed: F-2: one microphone, one holder — a *different generation* of the
same service is a different holder, so its own live exclusive grant does not put it on the mic as well
  left: Allowed { grant: 2, op: 2 }
 right: Denied(Contended)
```

**The hole.** C-3's one-holder rule recorded `other_holder` only when `grant.service !=
req.service`, so exclusivity was arbitrated by *service name*. The candidate filter
(`grant.kind != req.kind || grant.service != req.service || grant.instance != req.instance`)
already sends a same-service / different-instance grant into the "someone else holds this" branch
and sets `contended`, but `contended` alone only explains a refusal that was already going to
happen — with the requester's own grant affordable, the pick went through and the request was
allowed. Two live exclusive grants of one microphone, one per generation.

**The fix.** The holder identity is `(service, instance)`, exactly as the grant model documents:
`instance` is the generation, and B9's "a rollback revokes the old generation's grants" only works
because the generation is part of the identity. Since the branch is only reachable when the
identity differs (the kind is matched inside the branch), every live exclusive grant of the
resource found there belongs to a different holder — another service *or* another generation — and
now counts as `other_holder` either way. First-come arbitration is unchanged: the earliest-minted
live exclusive grant wins, and the new generation is refused until the old generation's grant is
revoked or dies.

The test pins the handover path too, so the fix does not strand the resource: after
`table.revoke(1)`, generation 1's request is allowed.

**Green:** the different-service contention tests (`two_holders_cannot_share_one_exclusive_device`,
`two_holders_cannot_both_run_on_one_exclusive_device`) stay green.

---

## Gate

```
export PATH="$HOME/.cargo/bin:$PATH"
cargo test --offline --locked          --manifest-path tools/caps-check/Cargo.toml
cargo test --offline --locked --release --manifest-path tools/caps-check/Cargo.toml
```

Both, at `fd94845`:

```
test result: ok. 1 passed;  0 failed  (unit: audit::tests::journal_keeps_the_tail_and_counts_what_it_lost)
test result: ok. 32 passed; 0 failed  (tests/model.rs — 30 prior + F-1 + F-2)
test result: ok. 0 passed;  0 failed  (doc tests)
```

No test is red at any commit. `git -c core.excludesFile=/dev/null` throughout; working tree clean
after the last commit.
