use device_control::act::{script_ack, script_result, ActExecutor, ScenarioScript, ScenarioTransport};
use device_control::command_record::{CommandRequest, result_contract::ResultStatus};
use device_control::session::{PeerId, SessionPolicy, State};

#[test]
fn known_failure_is_terminal_without_fabricated_success() {
    let boot = [3; 16];
    let peer = PeerId::new(9).unwrap();
    let mut policy = SessionPolicy::new([7; 16], boot, peer).unwrap();
    policy.accept_trusted_peer(peer).unwrap();
    let args: [&[u8]; 1] = [b"fixture"];
    let request = CommandRequest::new(5000, &args).unwrap();
    let ack = script_ack(1, &boot);
    let result = script_result(1, ResultStatus::Failure, b"", b"failed", &boot);
    let frames = [(&ack.0[..], ack.1), (&result.0[..], result.1)];
    let script = ScenarioScript::new(&frames);
    let mut transport = ScenarioTransport::new(&script);
    {
        let mut executor = ActExecutor::new(&mut transport, &mut policy, boot, 42);
        let result = executor.run(&request, 8).unwrap();
        assert_eq!(result.status, ResultStatus::Failure);
        assert_eq!(executor.requests_sent(), 1);
    }
    assert_eq!(policy.state(), State::ResultKnown,
        "Failure is a completed known outcome, not an unresolved action");
    assert_eq!(policy.snapshot().inflight_id, None);
}

// --- DC-ACT-FAILURE negative controls --------------------------------------
// The root probe above is saved VERBATIM from ACT-FAILURE-ROOT-PROBE.rs.
// Everything below pins the narrowness of the new known-command-failure
// edge: it fires for a matching, ACKed, in-flight Command and nothing else,
// and every rejection leaves the policy snapshot byte-for-byte equal.

use device_control::act::ActError;
use device_control::session::SessionError;
use device_control::Op;

const BOOT: [u8; 16] = [3; 16];

fn connected_policy() -> SessionPolicy {
    let peer = PeerId::new(9).unwrap();
    let mut policy = SessionPolicy::new([7; 16], BOOT, peer).unwrap();
    policy.accept_trusted_peer(peer).unwrap();
    policy
}

/// Negative control 1 — an un-ACKed failure never closes the slot. At the
/// policy level the edge refuses while the action is still Pending (no ACK
/// recorded). Through the executor, a scripted Failure arriving before its
/// ACK is a `CorrelationMismatch`: the action stays in flight, un-ACKed.
#[test]
fn unacked_failure_rejects_with_snapshot_unchanged() {
    let mut policy = connected_policy();
    policy.start_action(Op::Command, 10).unwrap();
    policy.mark_sent(10).unwrap();
    let before = policy.snapshot();
    assert_eq!(before.state, State::Pending);
    assert_eq!(
        policy.record_command_failure(10),
        Err(SessionError::UnknownRequest)
    );
    assert_eq!(
        policy.snapshot(),
        before,
        "an un-ACKed failure must not mutate the policy"
    );

    // The same condition through the public executor path.
    let failure = script_result(1, ResultStatus::Failure, b"", b"failed", &BOOT);
    let frames = [(&failure.0[..], failure.1)];
    let script = ScenarioScript::new(&frames);
    let mut transport = ScenarioTransport::new(&script);
    let args: [&[u8]; 1] = [b"fixture"];
    let request = CommandRequest::new(5000, &args).unwrap();
    let mut policy = connected_policy();
    {
        let mut executor = ActExecutor::new(&mut transport, &mut policy, BOOT, 42);
        assert_eq!(
            executor.run(&request, 8).unwrap_err(),
            ActError::CorrelationMismatch
        );
    }
    let snap = policy.snapshot();
    assert_eq!(snap.state, State::Pending);
    assert_eq!(snap.inflight_id, Some(1));
    assert_eq!(snap.acked, Some(false), "a failure may never forge an ACK");
}

/// Negative control 2 — a request id that is not the in-flight action is
/// refused, whether it sits above or below the in-flight id.
#[test]
fn wrong_request_rejects_with_snapshot_unchanged() {
    let mut policy = connected_policy();
    policy.start_action(Op::Command, 10).unwrap();
    policy.mark_sent(10).unwrap();
    policy.record_ack(10).unwrap();
    let before = policy.snapshot();
    assert_eq!(before.state, State::Acknowledged);
    assert_eq!(
        policy.record_command_failure(11),
        Err(SessionError::UnknownRequest)
    );
    assert_eq!(
        policy.record_command_failure(9),
        Err(SessionError::UnknownRequest)
    );
    assert_eq!(policy.snapshot(), before);
}

/// Negative control 3 — the edge is Command-only. An ACKed Input, Logs or
/// Screenshot action is refused with zero mutation: a known command failure
/// must not become a back door for closing other operations.
#[test]
fn wrong_operation_rejects_with_snapshot_unchanged() {
    for op in [Op::Input, Op::Logs, Op::Screenshot] {
        let mut policy = connected_policy();
        policy.start_action(op, 10).unwrap();
        policy.mark_sent(10).unwrap();
        policy.record_ack(10).unwrap();
        let before = policy.snapshot();
        assert_eq!(before.state, State::Acknowledged);
        assert_eq!(
            policy.record_command_failure(10),
            Err(SessionError::UnknownRequest),
            "the known-failure edge must be Command-only"
        );
        assert_eq!(policy.snapshot(), before);
    }
}

/// Negative control 4 — duplicate closure. Once the slot closed on the
/// matching failure, a second closure attempt is refused and the snapshot
/// stays exactly at the closed state (no re-open, no re-close).
#[test]
fn duplicate_closure_rejects_with_snapshot_unchanged() {
    let mut policy = connected_policy();
    policy.start_action(Op::Command, 10).unwrap();
    policy.mark_sent(10).unwrap();
    policy.record_ack(10).unwrap();
    assert_eq!(policy.record_command_failure(10), Ok(()));
    let closed = policy.snapshot();
    assert_eq!(closed.state, State::ResultKnown);
    assert_eq!(closed.inflight_id, None);
    assert_eq!(
        policy.record_command_failure(10),
        Err(SessionError::UnknownRequest)
    );
    assert_eq!(policy.snapshot(), closed);
}

/// Positive control — a matching known failure closes the slot while the
/// reported outcome stays a Failure (stderr preserved, no fabricated
/// Success), and the closed slot permits the next strictly higher request.
#[test]
fn matching_failure_closes_and_permits_later_higher_request() {
    let ack = script_ack(1, &BOOT);
    let failure = script_result(1, ResultStatus::Failure, b"partial", b"boom", &BOOT);
    let frames = [(&ack.0[..], ack.1), (&failure.0[..], failure.1)];
    let script = ScenarioScript::new(&frames);
    let mut transport = ScenarioTransport::new(&script);
    let args: [&[u8]; 1] = [b"fixture"];
    let request = CommandRequest::new(5000, &args).unwrap();
    let mut policy = connected_policy();
    {
        let mut executor = ActExecutor::new(&mut transport, &mut policy, BOOT, 42);
        let out = executor.run(&request, 8).unwrap();
        assert_eq!(out.status, ResultStatus::Failure);
        assert_eq!(&out.stderr.bytes[..out.stderr.len], b"boom");
        assert_eq!(executor.requests_sent(), 1);
    }
    let snap = policy.snapshot();
    assert_eq!(snap.state, State::ResultKnown);
    assert_eq!(snap.inflight_id, None);
    assert!(
        policy.start_action(Op::Command, 5001).is_ok(),
        "a closed known failure must not strand the session"
    );
    assert_eq!(policy.snapshot().inflight_id, Some(5001));
}

/// Negative control 5 — UnknownOutcome is untouched: a scripted
/// STATUS_UNKNOWN result after its ACK, and a timeout (ACK, no result),
/// both leave the action in flight with its send evidence intact.
#[test]
fn unknown_outcome_and_timeout_stay_inflight() {
    let ack = script_ack(1, &BOOT);
    let unknown = script_result(1, ResultStatus::UnknownOutcome, b"", b"", &BOOT);
    let args: [&[u8]; 1] = [b"fixture"];
    let request = CommandRequest::new(5000, &args).unwrap();

    let frames = [(&ack.0[..], ack.1), (&unknown.0[..], unknown.1)];
    let script = ScenarioScript::new(&frames);
    let mut transport = ScenarioTransport::new(&script);
    let mut policy = connected_policy();
    {
        let mut executor = ActExecutor::new(&mut transport, &mut policy, BOOT, 42);
        let out = executor.run(&request, 8).unwrap();
        assert_eq!(out.status, ResultStatus::UnknownOutcome);
    }
    let snap = policy.snapshot();
    assert_eq!(snap.state, State::Acknowledged);
    assert_eq!(snap.inflight_id, Some(1));
    assert_eq!(snap.sent, Some(true));

    let ack_only = [(&ack.0[..], ack.1)];
    let script2 = ScenarioScript::new(&ack_only);
    let mut transport2 = ScenarioTransport::new(&script2);
    let mut policy2 = connected_policy();
    {
        let mut executor = ActExecutor::new(&mut transport2, &mut policy2, BOOT, 42);
        let out = executor.run(&request, 8).unwrap();
        assert_eq!(out.status, ResultStatus::UnknownOutcome);
    }
    let snap = policy2.snapshot();
    assert_eq!(snap.state, State::Acknowledged);
    assert_eq!(snap.inflight_id, Some(1));
    assert_eq!(snap.sent, Some(true));
}

/// Negative control 6 — rebuilding an executor cannot erase sent evidence:
/// after a sent-but-un-ACKed timeout, a fresh executor over the SAME policy
/// still refuses the proven-unsent closure with `SentNoAck`, and the policy
/// snapshot is unchanged.
#[test]
fn executor_rebuild_cannot_erase_sent_evidence() {
    let frames: [(&[u8], usize); 0] = [];
    let script = ScenarioScript::new(&frames);
    let mut first_transport = ScenarioTransport::new(&script);
    let args: [&[u8]; 1] = [b"fixture"];
    let request = CommandRequest::new(5000, &args).unwrap();
    let mut policy = connected_policy();
    {
        let mut first = ActExecutor::new(&mut first_transport, &mut policy, BOOT, 42);
        assert_eq!(first.run(&request, 8).unwrap_err(), ActError::NotAcked);
    }
    let before = policy.snapshot();
    assert_eq!(before.sent, Some(true));
    let mut second_transport = ScenarioTransport::new(&script);
    {
        let mut second = ActExecutor::new(&mut second_transport, &mut policy, BOOT, 42);
        assert_eq!(second.cancel(1), Err(ActError::SentNoAck));
    }
    assert_eq!(policy.snapshot(), before);
}
