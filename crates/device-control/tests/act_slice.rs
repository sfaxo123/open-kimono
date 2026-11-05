//! DC-CLI-02 act-slice tests: five focused checks of the bounded act
//! executor over the scripted fake transport and the session policy.
//!
//! The frozen `Script`/`FakeTransport` serve only their fixed frames and
//! cannot be re-scripted, so the act slice ships its own scenario
//! transport (the ACK/RESULT convention documented in `src/act.rs`):
//! tests build already-encoded act frames with `script_ack`/`script_result`
//! and serve them through `ScenarioScript`/`ScenarioTransport`. The
//! frozen modules are untouched.

use device_control::act::{
    script_ack, script_result, ActError, ActExecutor, ScenarioScript, ScenarioTransport,
};
use device_control::command_record::result_contract::{ResultStatus, MAX_STREAM_BODY};
use device_control::command_record::CommandRequest;
use device_control::session::{PeerId, SessionPolicy, State};

const BOOT: [u8; 16] = [3u8; 16];
const BUILD: [u8; 16] = [7u8; 16];
const SESSION_ID: u64 = 42;

/// Build a connected policy (Ready) with the standard ids.
fn connected_policy() -> SessionPolicy {
    let mut p = SessionPolicy::new(BUILD, BOOT, PeerId::new(9).unwrap()).unwrap();
    p.accept_trusted_peer(PeerId::new(9).unwrap()).unwrap();
    p
}

fn request() -> CommandRequest<'static> {
    static ARGS: [&'static [u8]; 1] = [b"ls".as_slice()];
    CommandRequest::new(5000, &ARGS).unwrap()
}

/// Test 1 — success path: ACK then RESULT completes the action; the
/// returned ActionResult carries the scripted streams and correlation,
/// and the policy returns to idle (ResultKnown).
#[test]
fn success_path_completes_action_and_policy_is_idle_after() {
    let ack = script_ack(1, &BOOT);
    let res = script_result(1, ResultStatus::Success, b"hello", b"", &BOOT);
    let frames = [(&ack.0[..], ack.1), (&res.0[..], res.1)];
    let script = ScenarioScript::new(&frames);
    let mut transport = ScenarioTransport::new(&script);

    let mut policy = connected_policy();
    let mut exec = ActExecutor::new(&mut transport, &mut policy, BOOT, SESSION_ID);
    let result = exec.run(&request(), 8).expect("success path");
    let sent = exec.requests_sent();
    drop(exec);

    assert_eq!(result.status, ResultStatus::Success);
    assert_eq!(&result.stdout.bytes[..result.stdout.len], b"hello");
    assert_eq!(result.stderr.len, 0);
    assert!(!result.stdout.truncated);
    assert_eq!(result.correlation.session_id, SESSION_ID);
    assert_eq!(result.correlation.request_seq, 1);
    assert_eq!(sent, 1);
    // Policy idle after: the action slot is closed.
    assert_eq!(policy.state(), State::ResultKnown);
    assert_eq!(policy.snapshot().inflight_id, None);
}

/// Test 2 — timeout: no result within budget yields UnknownOutcome with
/// empty bodies and correlation set; the policy STAYS in-flight
/// (Acknowledged, not auto-reset) and the request was sent EXACTLY once.
#[test]
fn timeout_yields_unknown_outcome_policy_stays_inflight_no_replay() {
    // Scenario: ACK only — the result never arrives.
    let ack = script_ack(1, &BOOT);
    let frames = [(&ack.0[..], ack.1)];
    let script = ScenarioScript::new(&frames);
    let mut transport = ScenarioTransport::new(&script);

    let mut policy = connected_policy();
    let mut exec = ActExecutor::new(&mut transport, &mut policy, BOOT, SESSION_ID);
    // Budget covers the ACK plus the script's remaining skippable
    // frames; the result never comes.
    let result = exec
        .run(&request(), 8)
        .expect("timeout returns UnknownOutcome");
    let sent = exec.requests_sent();
    drop(exec);

    assert_eq!(result.status, ResultStatus::UnknownOutcome);
    assert_eq!(result.stdout.len, 0);
    assert_eq!(result.stderr.len, 0);
    assert!(!result.stdout.truncated && !result.stderr.truncated);
    assert_eq!(result.correlation.session_id, SESSION_ID);
    assert_eq!(result.correlation.request_seq, 1);
    // No blind replay: the request crossed the transport exactly once.
    assert_eq!(sent, 1);
    // Policy stays action-in-flight (Acknowledged), NOT auto-reset.
    assert_eq!(policy.state(), State::Acknowledged);
    assert_eq!(policy.snapshot().inflight_id, Some(1));
}

/// Test 3 — cancel before ACK: the in-flight action is abandoned, the
/// policy returns to idle (no in-flight slot), and nothing crossed the
/// transport.
#[test]
fn cancel_before_ack_returns_policy_to_idle_no_result() {
    let frames: [(&[u8], usize); 0] = [];
    let script = ScenarioScript::new(&frames);
    let mut transport = ScenarioTransport::new(&script);
    let mut policy = connected_policy();

    // Start the action at id 1 (as the executor would), then cancel it
    // while still Pending — before any ACK lands.
    policy.start_action(device_control::Op::Command, 1).unwrap();
    let mut exec = ActExecutor::new(&mut transport, &mut policy, BOOT, SESSION_ID);
    exec.cancel(1).expect("cancel before ack");
    let sent = exec.requests_sent();
    drop(exec);

    // Idle: the action slot is closed, nothing acked, and the next
    // action can start immediately (Ready-equivalent).
    assert_eq!(policy.state(), State::ResultKnown);
    assert_eq!(policy.snapshot().inflight_id, None);
    assert_eq!(policy.snapshot().acked, None);
    assert_eq!(sent, 0);
    // The cancelled id is consumed; the next action starts at id + 2.
    policy.start_action(device_control::Op::Command, 3).unwrap();
    assert_eq!(policy.state(), State::Pending);
}

/// Test 4 — a result frame arriving BEFORE the ACK is rejected with
/// CorrelationMismatch semantics and the policy is unchanged (still
/// Pending, action still in flight).
#[test]
fn result_before_ack_is_rejected_and_policy_unchanged() {
    // Scenario: RESULT first, then ACK.
    let res = script_result(1, ResultStatus::Success, b"", b"", &BOOT);
    let ack = script_ack(1, &BOOT);
    let frames = [(&res.0[..], res.1), (&ack.0[..], ack.1)];
    let script = ScenarioScript::new(&frames);
    let mut transport = ScenarioTransport::new(&script);

    let mut policy = connected_policy();
    let mut exec = ActExecutor::new(&mut transport, &mut policy, BOOT, SESSION_ID);
    let err = exec.run(&request(), 8).unwrap_err();
    let sent = exec.requests_sent();
    drop(exec);
    assert_eq!(err, ActError::CorrelationMismatch);
    // Policy unchanged: still Pending with the action in flight.
    assert_eq!(policy.state(), State::Pending);
    assert_eq!(policy.snapshot().inflight_id, Some(1));
    assert_eq!(sent, 1);
}

/// Test 5 — an oversize scripted result body (a stream longer than
/// MAX_STREAM_BODY cannot be represented under the frozen contract)
/// yields OversizeResult and the policy is unchanged (still Acknowledged,
/// action in flight).
#[test]
fn oversize_result_body_yields_oversize_error_policy_unchanged() {
    // A stream body of MAX_STREAM_BODY + 1 bytes: unrepresentable. The
    // scripted frame declares the full length (fits u16), exceeding the
    // frozen per-stream cap.
    let big = [0u8; MAX_STREAM_BODY + 1];
    let ack = script_ack(1, &BOOT);
    let res = script_result(1, ResultStatus::Success, &big, b"", &BOOT);
    let frames = [(&ack.0[..], ack.1), (&res.0[..], res.1)];
    let script = ScenarioScript::new(&frames);
    let mut transport = ScenarioTransport::new(&script);

    let mut policy = connected_policy();
    let mut exec = ActExecutor::new(&mut transport, &mut policy, BOOT, SESSION_ID);
    let err = exec.run(&request(), 8).unwrap_err();
    let sent = exec.requests_sent();
    drop(exec);
    assert_eq!(err, ActError::OversizeResult);
    // Policy unchanged: ACK landed, result rejected, action still in flight.
    assert_eq!(policy.state(), State::Acknowledged);
    assert_eq!(policy.snapshot().inflight_id, Some(1));
    assert_eq!(sent, 1);
}

/// Test 6 (D1) — a valid RESULT frame with STATUS_UNKNOWN never closes the
/// policy slot as a known success: run() returns Ok with an UnknownOutcome
/// ActionResult, and the policy stays in-flight exactly like the timeout
/// path (Acknowledged, inflight still Some — NOT ResultKnown).
#[test]
fn unknown_result_no_closure() {
    let ack = script_ack(1, &BOOT);
    let res = script_result(1, ResultStatus::UnknownOutcome, b"", b"", &BOOT);
    let frames = [(&ack.0[..], ack.1), (&res.0[..], res.1)];
    let script = ScenarioScript::new(&frames);
    let mut transport = ScenarioTransport::new(&script);

    let mut policy = connected_policy();
    let mut exec = ActExecutor::new(&mut transport, &mut policy, BOOT, SESSION_ID);
    let result = exec
        .run(&request(), 8)
        .expect("unknown result still returns Ok");
    let sent = exec.requests_sent();
    drop(exec);

    assert_eq!(result.status, ResultStatus::UnknownOutcome);
    assert_eq!(result.correlation.session_id, SESSION_ID);
    assert_eq!(result.correlation.request_seq, 1);
    assert_eq!(sent, 1);
    // Never closed as success: the slot stays in-flight.
    let snap = policy.snapshot();
    assert_ne!(snap.state, State::ResultKnown);
    assert_eq!(snap.state, State::Acknowledged);
    assert_eq!(snap.inflight_id, Some(1));
}

/// Test 7 (D2) — cancel AFTER the request was sent but never acknowledged
/// (run() timed out with an empty script) is refused with SentNoAck and
/// leaves the policy exactly as it was: still in-flight, acked None, and
/// the request crossed the transport exactly once.
#[test]
fn cancel_after_send_refused() {
    let frames: [(&[u8], usize); 0] = [];
    let script = ScenarioScript::new(&frames);
    let mut transport = ScenarioTransport::new(&script);

    let mut policy = connected_policy();
    let mut exec = ActExecutor::new(&mut transport, &mut policy, BOOT, SESSION_ID);
    // Sent, never ACKed: run() ends with NotAcked, slot still Pending.
    let err = exec.run(&request(), 8).unwrap_err();
    assert_eq!(err, ActError::NotAcked);

    // A sent-but-un-ACKed action may have executed on the device: forging
    // an ACK + success closure is not evidence, so cancel is refused.
    let err = exec.cancel(1).unwrap_err();
    let sent = exec.requests_sent();
    drop(exec);
    assert_eq!(err, ActError::SentNoAck);
    // Zero policy mutation: still Pending, in-flight, and the in-flight
    // record was never acked (acked flag stays false — no forged ACK).
    let snap = policy.snapshot();
    assert_eq!(snap.state, State::Pending);
    assert_eq!(snap.inflight_id, Some(1));
    assert_eq!(snap.acked, Some(false));
    assert_eq!(sent, 1);
}

/// Test 8 — pins the timeout path's no-mutation discipline: the timeout
/// returns Ok(UnknownOutcome) with the policy still in-flight, and a
/// second run() is refused by the policy (ActionInProgress) — there is no
/// auto-reset and no blind replay.
#[test]
fn timeout_then_no_mutation() {
    let ack = script_ack(1, &BOOT);
    let frames = [(&ack.0[..], ack.1)];
    let script = ScenarioScript::new(&frames);
    let mut transport = ScenarioTransport::new(&script);

    let mut policy = connected_policy();
    let mut exec = ActExecutor::new(&mut transport, &mut policy, BOOT, SESSION_ID);
    let result = exec
        .run(&request(), 8)
        .expect("timeout returns UnknownOutcome");
    assert_eq!(result.status, ResultStatus::UnknownOutcome);

    // A second run is refused by the frozen policy: the slot was never
    // closed, so no new action may start (no auto-reset).
    let err = exec.run(&request(), 8).unwrap_err();
    let sent = exec.requests_sent();
    drop(exec);
    assert_eq!(
        err,
        ActError::Policy(device_control::session::SessionError::ActionInProgress)
    );
    // Policy stayed in-flight the whole time (no auto-reset), and there is
    // still exactly one request on the wire — no replay.
    let snap = policy.snapshot();
    assert_eq!(snap.state, State::Acknowledged);
    assert_eq!(snap.inflight_id, Some(1));
    assert_eq!(sent, 1);
}

/// Test 9 — DC-CLI-02D: a fresh executor over the SAME policy cannot
/// cancel a request the previous executor already sent with no ACK.
/// The send evidence lives in the policy record (`sent == true`), so
/// executor-local state (or its absence) is irrelevant: the cancel is
/// refused with SentNoAck and the policy snapshot is left unchanged.
#[test]
fn cancel_inherited_pending_refused() {
    let frames: [(&[u8], usize); 0] = [];
    let script = ScenarioScript::new(&frames);
    let mut first_transport = ScenarioTransport::new(&script);
    let mut policy = connected_policy();
    let mut first = ActExecutor::new(&mut first_transport, &mut policy, BOOT, SESSION_ID);
    // The first executor sends once with no ACK: NotAcked, slot in flight.
    assert!(first.run(&request(), 8).is_err());
    assert_eq!(first.requests_sent(), 1);
    drop(first);
    let before = policy.snapshot();
    assert_eq!(before.inflight_id, Some(1));
    assert_eq!(before.sent, Some(true));

    // Reconstruct an executor over the SAME policy: the policy record
    // still says sent, so a proven-unsent closure would be forged.
    let mut second_transport = ScenarioTransport::new(&script);
    let mut second = ActExecutor::new(&mut second_transport, &mut policy, BOOT, SESSION_ID);
    assert_eq!(second.cancel(1), Err(ActError::SentNoAck));

    // Policy snapshot UNCHANGED: still in-flight at id 1, not acked,
    // no result recorded (state still Pending, not ResultKnown).
    let after = policy.snapshot();
    assert_eq!(after.inflight_id, Some(1));
    assert_ne!(after.acked, Some(true));
    assert_eq!(after.state, State::Pending);
    assert_eq!(after, before);
}

/// Test 10 — DC-CLI-02D: `mark_sent` is bound to the in-flight record.
/// It refuses a non-in-flight id (never started, and an id whose slot
/// already closed) with zero policy mutation — a sent stamp may never be
/// written onto a record that does not own the in-flight action.
#[test]
fn mark_sent_only_for_inflight() {
    let mut policy = connected_policy();
    let before = policy.snapshot();

    // No action in flight at all: refused.
    assert_eq!(
        policy.mark_sent(1),
        Err(device_control::session::SessionError::UnknownRequest)
    );
    assert_eq!(policy.snapshot(), before);

    // In flight at id 1: accepted, snapshot shows sent == true.
    policy.start_action(device_control::Op::Command, 1).unwrap();
    policy.mark_sent(1).unwrap();
    assert_eq!(policy.snapshot().sent, Some(true));

    // A different (non-in-flight) id while id 1 is in flight: refused,
    // the in-flight record's sent flag untouched.
    assert_eq!(
        policy.mark_sent(2),
        Err(device_control::session::SessionError::UnknownRequest)
    );
    assert_eq!(policy.snapshot().sent, Some(true));
}
