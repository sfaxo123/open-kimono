mod common;

use common::{kinds, load, load_with};
use sandbox::{
    Capability, DenyReason, EventKind, Grant, Grants, Limits, SandboxError, StopReason, ERR_QUOTA,
    OK,
};

/// `run` asks for a callback after 500 ms with token 7; `on_timer` emits
/// `timer.fired` carrying nothing.
const TIMED: &str = r#"
(module
  (import "op6" "timer_after" (func $after (param i32 i32) (result i32)))
  (import "op6" "trace_emit" (func $trace (param i32 i32 i32 i32) (result i32)))
  (memory (export "memory") 1)
  (global $rc (export "rc") (mut i32) (i32.const 99))
  (global $last (export "last") (mut i32) (i32.const 0))
  (data (i32.const 0) "timer.fired")
  (func (export "run") (global.set $rc (call $after (i32.const 500) (i32.const 7))))
  (func (export "on_timer") (param $token i32)
    (global.set $last (local.get $token))
    (drop (call $trace (i32.const 0) (i32.const 11) (i32.const 0) (i32.const 0)))))
"#;

#[test]
fn timer_fires_on_timer_with_its_token_when_due() {
    let mut svc = load(TIMED, &[Capability::TimerAfter, Capability::TraceEmit]);
    svc.set_now(1_000);
    svc.invoke("run").unwrap();
    assert_eq!(svc.global_i32("rc"), Some(OK));
    assert_eq!(svc.pending_timers(), vec![(1_500, 7)]);

    assert!(svc.fire_due_timers(1_499).unwrap().is_empty(), "not yet due");
    let reports = svc.fire_due_timers(1_500).unwrap();
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].export, "on_timer");
    assert_eq!(svc.global_i32("last"), Some(7));
    assert!(svc.pending_timers().is_empty());

    let events = kinds(&svc);
    assert!(events.contains(&EventKind::TimerScheduled { token: 7, due_ms: 1_500 }));
    assert!(events.contains(&EventKind::TimerFired { token: 7 }));
    assert!(events.contains(&EventKind::Trace { name: "timer.fired".into(), payload: String::new() }));
}

#[test]
fn pending_timer_cap_is_a_return_code() {
    let limits = Limits { max_pending_timers: 1, ..Limits::default() };
    let mut svc = load_with(TIMED, Grants::of(&[Capability::TimerAfter]), limits);
    svc.invoke("run").unwrap();
    svc.invoke("run").unwrap();
    assert_eq!(svc.global_i32("rc"), Some(ERR_QUOTA));
    assert_eq!(svc.pending_timers().len(), 1);
}

#[test]
fn timer_without_grant_is_refused() {
    let mut svc = load(TIMED, &[Capability::TraceEmit]);
    assert_eq!(
        svc.invoke("run"),
        Err(SandboxError::Denied { capability: Capability::TimerAfter, reason: DenyReason::NotGranted })
    );
    assert!(svc.pending_timers().is_empty());
}

#[test]
fn clock_never_goes_backwards() {
    let mut svc = load(TIMED, &[Capability::TimerAfter]);
    svc.set_now(10);
    svc.set_now(5);
    assert_eq!(svc.now_ms(), 10);
}

/// S-1: `on_timer` re-enters the guest and can emit through still-live grants,
/// so firing a due timer with a dead `timer.after` grant must refuse the
/// callback on the same terms as a grantless host call: refusal event, no
/// side effect (the guest is not re-entered), and the service is stopped.

#[test]
fn timer_firing_after_revoke_is_refused_and_stops_the_service() {
    let mut svc = load(TIMED, &[Capability::TimerAfter, Capability::TraceEmit]);
    svc.set_now(1_000);
    svc.invoke("run").unwrap(); // schedules (500 ms, token 7), due at 1_500
    assert!(svc.revoke(Capability::TimerAfter));

    let err = svc.fire_due_timers(1_500).unwrap_err();
    let denied =
        SandboxError::Denied { capability: Capability::TimerAfter, reason: DenyReason::Revoked };
    assert_eq!(err, denied, "a revoked grant does not deliver its pending callbacks");
    assert_eq!(svc.global_i32("last"), Some(0), "on_timer was not entered: the guest is not re-run");
    let events = kinds(&svc);
    assert!(
        events.contains(&EventKind::Denied {
            capability: Capability::TimerAfter,
            reason: DenyReason::Revoked
        }),
        "the refusal is itself a journal event: {events:?}"
    );
    assert!(
        !events.iter().any(|k| matches!(k, EventKind::Trace { .. })),
        "a refused firing has no side effect: {events:?}"
    );
    assert_eq!(
        svc.stop_reason(),
        Some(&StopReason::Denied {
            capability: Capability::TimerAfter,
            reason: DenyReason::Revoked
        })
    );
    assert_eq!(
        svc.fire_due_timers(1_500),
        Err(SandboxError::Stopped),
        "a stopped service is never re-entered"
    );
}

#[test]
fn timer_firing_after_expiry_is_refused_and_stops_the_service() {
    let grants = Grants::none()
        .with(Grant::until(Capability::TimerAfter, 1_400))
        .with(Grant::new(Capability::TraceEmit));
    let mut svc = load_with(TIMED, grants, Limits::default());
    svc.set_now(1_000);
    svc.invoke("run").unwrap(); // grant dies at 1_400, the timer is due at 1_500

    let err = svc.fire_due_timers(1_500).unwrap_err();
    let denied =
        SandboxError::Denied { capability: Capability::TimerAfter, reason: DenyReason::Expired };
    assert_eq!(err, denied, "an expired grant does not deliver its pending callbacks");
    assert_eq!(svc.global_i32("last"), Some(0), "on_timer was not entered: the guest is not re-run");
    let events = kinds(&svc);
    assert!(
        events.contains(&EventKind::Denied {
            capability: Capability::TimerAfter,
            reason: DenyReason::Expired
        }),
        "the refusal is itself a journal event: {events:?}"
    );
    assert_eq!(
        svc.fire_due_timers(1_500),
        Err(SandboxError::Stopped),
        "a stopped service is never re-entered"
    );
}

/// REVIEW-1 #2: ticking the clock of a service that never asked for timers (no
/// `timer.after` grant, nothing pending) must be a no-op — not a refusal that
/// stops it. The grant is only checked when a due timer would re-enter the guest.
#[test]
fn ticking_a_service_without_timers_or_grant_is_a_no_op() {
    let mut svc = load(TIMED, &[Capability::TraceEmit]);
    assert_eq!(svc.fire_due_timers(10), Ok(Vec::new()));
    assert!(svc.stop_reason().is_none());
    assert!(!kinds(&svc).iter().any(|k| matches!(k, EventKind::Denied { .. } | EventKind::Stopped(_))));
}

// ---- LUNA merge-review finding 3 (P2): a scheduler poll that has nothing to
// deliver must be side-effect-free w.r.t. capability enforcement (idle +
// future-dated-only). Red against fire_due_timers before aa16f2f (a poll with
// a dead grant errored and stopped the service even with nothing due); green
// guards against the merged any_due guard. A DUE timer under a dead grant
// stays refused before re-entry — pinned by the two timer_firing_after_*
// tests above, and by the refusal chain inside future_only_timers_poll.

#[test]
fn idle_poll_with_revoked_timer_grant_is_side_effect_free() {
    let mut svc = load(TIMED, &[Capability::TimerAfter, Capability::TraceEmit]);
    // no entry point executed: the service holds zero pending timers
    assert!(svc.revoke(Capability::TimerAfter));
    let reports = svc.fire_due_timers(1_500).expect("an idle poll must not error");
    assert!(reports.is_empty(), "nothing to deliver");
    assert!(svc.stop_reason().is_none(), "idle poll must not stop the service");
    assert!(svc.fire_due_timers(2_000).is_ok(), "and the next poll is a poll again, not a stop");
    let events = kinds(&svc);
    assert!(
        !events.iter().any(|k| matches!(k, EventKind::Denied { .. } | EventKind::Stopped(_))),
        "a no-due poll neither checks the grant nor leaks journal: {events:?}"
    );
}

#[test]
fn idle_poll_with_expired_timer_grant_is_side_effect_free() {
    let grants = Grants::none()
        .with(Grant::until(Capability::TimerAfter, 1_400))
        .with(Grant::new(Capability::TraceEmit));
    let mut svc = load_with(TIMED, grants, Limits::default());
    svc.set_now(1_500); // grant already dead; a timer was never scheduled
    let reports = svc.fire_due_timers(1_500).expect("an idle poll must not error");
    assert!(reports.is_empty(), "nothing to deliver");
    assert!(svc.stop_reason().is_none(), "idle poll must not stop the service");
    let events = kinds(&svc);
    assert!(
        !events.iter().any(|k| matches!(k, EventKind::Denied { .. } | EventKind::Stopped(_))),
        "a no-due poll neither checks the grant nor leaks journal: {events:?}"
    );
}

#[test]
fn future_only_timers_poll_is_side_effect_free() {
    let mut svc = load(TIMED, &[Capability::TimerAfter, Capability::TraceEmit]);
    svc.set_now(1_000);
    svc.invoke("run").unwrap(); // schedules (500 ms, token 7), due at 1_500
    assert!(svc.revoke(Capability::TimerAfter));
    let reports = svc.fire_due_timers(1_200).expect("nothing is due at 1_200: a scheduler poll is not a violation");
    assert!(reports.is_empty());
    assert!(svc.stop_reason().is_none(), "future-only queue must not stop the service");
    assert_eq!(svc.pending_timers(), vec![(1_500, 7)], "the future timer stays pending");
    // once it becomes due under a dead grant, S-1 refuses before re-entry:
    assert!(
        matches!(svc.fire_due_timers(1_500), Err(SandboxError::Denied { .. })),
        "a DUE timer under a dead grant is still refused"
    );
    assert_eq!(svc.global_i32("last"), Some(0), "the guest was not re-entered");
}
