# FIX-TIMER-IDLE-POLL — finding F-3 (P2): idle timer polls must be side-effect-free

Worker: G/sandbox seat (DSH, spark) · 2026-10-25 · branch `fix/timer-idle-poll`, rebased onto main `193ca54` —
this branch **sits on top of the external fix `aa16f2f`**, it does not duplicate it.

## Finding (input)

`fire_due_timers` checked the `TimerAfter` grant **before** collecting due timers and, with the grant dead
(revoked or expired), recorded `Denied` + `Stopped` and killed the service on **every** poll — including
polls where nothing was due (no pending timers, or all future-dated). A routine scheduler tick therefore
killed an otherwise idle service although no callback ever re-entered guest code
(LUNA-MERGE-REVIEW-20261024-2102.md, finding 3, P2 = Claude REVIEW-1 finding #2).

## Timeline (honest)

1. Task dispatched test-first against main `4442937` (pre-`aa16f2f`).
2. **The finding was already fixed externally on main by `aa16f2f`** — "sandbox: only check timer.after
   when a due timer would re-enter the guest" (Claude REVIEW-1 #2; Co-Authored-By session link kept in
   that commit). `fix/timer-idle-poll` was rebased onto `193ca54` (which contains `aa16f2f`) mid-task;
   I re-implemented **nothing** — the `any_due` guard is `aa16f2f`'s, credited there.
3. **Correction:** my first run, logged as "red reproduction (2 failing)", is mislabelled — after the
   rebase the guard was already present and my regression tests **passed**; they are green *guards*, not
   red reproducers. No red was manufactured on the rebased base (and fabricating one was barred).
4. `fecfd0c` carried my first test block (4 tests, one redundant); this pass trims the redundant
   due-timer refusal+stop guard (main's `timer_firing_after_revoke/expiry_is_refused_and_stops_the_service`
   already pin that behavior, more thoroughly) and corrects the stale "red against merged code" comment.

## Audit of `aa16f2f` (steer question 1) — PASS on both cases

- **Future-dated-only poll:** all `due_ms > now` ⇒ `any_due == false` ⇒ the grant check is skipped
  entirely; collect/retain/deliver handle nothing ⇒ `Ok(empty)`, no journal event, service stays alive,
  and the future timers **stay pending** (not silently dropped). Covered.
- **Due timer without a live grant:** `any_due == true` ⇒ grant checked ⇒ `Denied` + `Stopped` recorded
  and the service stopped **before** `on_timer` re-entry (S-1 preserved; polls of a stopped service keep
  returning `Err(Stopped)`, exactly as main's S-1 tests pin). Preserved.
- No real gap found: `any_due` is computed at the advanced `now` (monotonic clock), and the
  stopped-service interaction is behaviourally unchanged from the pre-fix code.

## Added on this branch (test + docs only — no production-code change)

- `crates/sandbox/tests/timers.rs`: `idle_poll_with_revoked_timer_grant_is_side_effect_free`,
  `idle_poll_with_expired_timer_grant_is_side_effect_free`, `future_only_timers_poll_is_side_effect_free`
  (its tail also chains the due-under-dead-grant refusal). These are **green against the merged guard**
  (they guard it — per the steer, honestly: no red state exists on the rebased base) and would be red
  against pre-`aa16f2f` code, where idle and future-only polls error and stop the service.
- `crates/sandbox/README.md`: §Refusal timer bullet now describes both halves (due+dead-grant → refused
  and stopped before re-entry; nothing-due → not a capability use); test-count line corrected 35 → 39
  (main's count was already stale at 36 after the external fix merged).

## Gates (run against the final state, `PATH=$HOME/.cargo/bin` + rustup stable)

- debug `cargo test --offline` (gate form `--manifest-path crates/sandbox/Cargo.toml`): **all suites ok —
  39 passed, 0 failed, exit 0** (lib 0, grants 9, guest_host 2, hello 5, limits 11, rust_guest 2, timers 10)
- release `cargo test --offline --release`: **39/39, exit 0**
- workspace build: no root workspace manifest exists (none added); host `cargo build --offline` for
  crates/sandbox: Finished, exit 0

## Boundaries + coordination

Scope kept: `crates/sandbox/**` + this file only. `tools/caps-check/**` untouched (CAPS worker owns it);
no scripts/forks/apps/qemu-INTx edits; no phone. Coordination path: this file and the audit answer go
through Perplexity (orchestrator steer) — `aa16f2f` credited as the external fix with its own session
link; my delta is guard tests + doc corrections. Push: branch only, fast-forward from the remote copy
at `fecfd0c`.
