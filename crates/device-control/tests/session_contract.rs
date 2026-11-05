//! Frozen action sequences and negative controls for the session policy
//! state machine. Every rejected transition asserts full source-snapshot
//! equality. No fixture here claims a human actually performed an action.

use device_control::session::{
    FrameRef, PeerId, ReleasePlan, SessionError, SessionPolicy, State, MAX_KEY_WORDS, POINTER_SLOTS,
};
use device_control::Op;

const BUILD: [u8; 16] = [0xa1; 16];
const BOOT: [u8; 16] = [0xb2; 16];

fn peer(n: u64) -> PeerId {
    PeerId::new(n).unwrap()
}

fn policy() -> SessionPolicy {
    SessionPolicy::new(BUILD, BOOT, peer(7)).unwrap()
}

/// Fresh session with an accepted trusted connection (external precondition).
fn connected() -> SessionPolicy {
    let mut s = policy();
    s.accept_trusted_peer(peer(7)).unwrap();
    s
}

// ---------- construction boundary ----------

#[test]
fn constructor_requires_nonzero_distinct_ids_and_peer() {
    assert_eq!(
        SessionPolicy::new([0; 16], BOOT, peer(7)).err(),
        Some(SessionError::ZeroBuildId)
    );
    assert_eq!(
        SessionPolicy::new(BUILD, [0; 16], peer(7)).err(),
        Some(SessionError::ZeroBuildId)
    );
    assert_eq!(
        SessionPolicy::new(BUILD, BUILD, peer(7)).err(),
        Some(SessionError::BuildBootCollision)
    );
    assert_eq!(PeerId::new(0), Err(SessionError::ZeroIdentity));
    let s = policy();
    assert_eq!(s.state(), State::Disconnected);
}

// ---------- trust boundary: accept_trusted_peer ----------

#[test]
fn wrong_peer_rejected_without_mutation() {
    let mut s = policy();
    let before = s.snapshot();
    assert_eq!(s.accept_trusted_peer(peer(8)), Err(SessionError::WrongPeer));
    assert_eq!(s.snapshot(), before);
    assert_eq!(s.state(), State::Disconnected);
}

#[test]
fn duplicate_accept_rejected_without_mutation() {
    let mut s = connected();
    let before = s.snapshot();
    assert_eq!(
        s.accept_trusted_peer(peer(7)),
        Err(SessionError::AlreadyConnected)
    );
    assert_eq!(s.snapshot(), before);
    assert_eq!(s.state(), State::Ready);
}

// ---------- request id discipline ----------

#[test]
fn ids_must_be_nonzero_strictly_increasing() {
    let mut s = connected();
    let before = s.snapshot();
    assert_eq!(
        s.start_action(Op::Command, 0),
        Err(SessionError::NonMonotonicRequest)
    );
    assert_eq!(s.snapshot(), before);
    s.start_action(Op::Command, 5).unwrap();
    let mid = s.snapshot();
    assert_eq!(
        s.start_action(Op::Command, 5),
        Err(SessionError::ActionInProgress)
    );
    assert_eq!(s.snapshot(), mid);
}

#[test]
fn backwards_and_reused_ids_rejected_after_completion() {
    let mut s = connected();
    s.start_action(Op::Logs, 10).unwrap();
    s.record_result(10, true).unwrap();
    assert_eq!(s.state(), State::ResultKnown);
    // A completed action leaves the connection live; next id may start from
    // ResultKnown. Backwards/reused ids are still refused without mutation.
    let ready = s.snapshot();
    assert_eq!(
        s.start_action(Op::Logs, 9),
        Err(SessionError::NonMonotonicRequest)
    );
    assert_eq!(s.snapshot(), ready);
    assert_eq!(
        s.start_action(Op::Logs, 10),
        Err(SessionError::NonMonotonicRequest)
    );
    assert_eq!(s.snapshot(), ready);
}

#[test]
fn exhaustion_refuses_new_ids_without_storage() {
    let mut s = policy();
    s.accept_trusted_peer(peer(7)).unwrap();
    s.start_action(Op::Logs, u64::MAX).unwrap();
    s.record_result(u64::MAX, true).unwrap();
    let before = s.snapshot();
    assert_eq!(
        s.start_action(Op::Logs, 1),
        Err(SessionError::RequestExhausted)
    );
    assert_eq!(s.snapshot(), before);
}

#[test]
fn parallel_start_rejected_without_mutation() {
    let mut s = connected();
    s.start_action(Op::Command, 1).unwrap();
    let before = s.snapshot();
    assert_eq!(
        s.start_action(Op::Input, 2),
        Err(SessionError::ActionInProgress)
    );
    assert_eq!(s.snapshot(), before);
    assert_eq!(s.state(), State::Pending);
}

// ---------- ACK alone never completes ----------

#[test]
fn ack_alone_never_marks_completed() {
    let mut s = connected();
    s.start_action(Op::Command, 1).unwrap();
    s.set_reference_frame(1, 100).unwrap();
    s.record_ack(1).unwrap();
    assert_eq!(s.state(), State::Acknowledged);
    // ACK for the same id again is a duplicate end-transition: rejected.
    let before = s.snapshot();
    assert_eq!(s.record_ack(1), Err(SessionError::UnknownRequest));
    assert_eq!(s.snapshot(), before);
}

#[test]
fn ack_for_wrong_id_rejected_without_mutation() {
    let mut s = connected();
    s.start_action(Op::Command, 1).unwrap();
    let before = s.snapshot();
    assert_eq!(s.record_ack(2), Err(SessionError::UnknownRequest));
    assert_eq!(s.snapshot(), before);
}

// ---------- result semantics: command needs ack; input needs observation ----------

#[test]
fn command_result_without_ack_does_not_complete() {
    let mut s = connected();
    s.start_action(Op::Command, 1).unwrap();
    s.record_result(1, true).unwrap();
    // Root correction: the frozen version of this test asserted Acknowledged
    // here, codifying a defect — a result is not an ACK and must not
    // fabricate the Acknowledged state. State stays Pending.
    assert_eq!(s.state(), State::Pending);
}

#[test]
fn command_with_ack_and_result_completes() {
    let mut s = connected();
    s.start_action(Op::Command, 1).unwrap();
    s.record_ack(1).unwrap();
    s.record_result(1, true).unwrap();
    assert_eq!(s.state(), State::ResultKnown);
}

#[test]
fn input_result_is_not_ui_effect_proof() {
    let mut s = connected();
    s.start_action(Op::Input, 1).unwrap();
    s.set_reference_frame(1, 50).unwrap();
    s.record_ack(1).unwrap();
    s.record_result(1, true).unwrap();
    // Result alone is not proof: still awaiting a newer observation.
    assert_eq!(s.state(), State::Acknowledged);
}

// ---------- observation discipline ----------

#[test]
fn stale_observation_rejected_without_mutation() {
    let mut s = connected();
    s.start_action(Op::Input, 1).unwrap();
    s.set_reference_frame(1, 50).unwrap();
    let before = s.snapshot();
    assert_eq!(
        s.record_observation(
            FrameRef {
                boot: BOOT,
                build: BUILD,
                frame: 50
            },
            1
        ),
        Err(SessionError::StaleObservation)
    );
    assert_eq!(s.snapshot(), before);
}

#[test]
fn observation_with_wrong_boot_or_build_rejected() {
    let mut s = connected();
    s.start_action(Op::Input, 1).unwrap();
    s.set_reference_frame(1, 50).unwrap();
    let before = s.snapshot();
    assert_eq!(
        s.record_observation(
            FrameRef {
                boot: [0xbb; 16],
                build: BUILD,
                frame: 99
            },
            1
        ),
        Err(SessionError::IdentityMismatch)
    );
    assert_eq!(s.snapshot(), before);
    assert_eq!(
        s.record_observation(
            FrameRef {
                boot: BOOT,
                build: [0xcc; 16],
                frame: 99
            },
            1
        ),
        Err(SessionError::IdentityMismatch)
    );
    assert_eq!(s.snapshot(), before);
}

#[test]
fn observation_for_wrong_id_rejected() {
    let mut s = connected();
    s.start_action(Op::Input, 1).unwrap();
    s.set_reference_frame(1, 50).unwrap();
    let before = s.snapshot();
    assert_eq!(
        s.record_observation(
            FrameRef {
                boot: BOOT,
                build: BUILD,
                frame: 99
            },
            2
        ),
        Err(SessionError::UnknownRequest)
    );
    assert_eq!(s.snapshot(), before);
}

#[test]
fn input_completes_with_ack_result_and_newer_matching_observation() {
    let mut s = connected();
    s.start_action(Op::Input, 1).unwrap();
    s.set_reference_frame(1, 50).unwrap();
    s.record_ack(1).unwrap();
    s.record_observation(
        FrameRef {
            boot: BOOT,
            build: BUILD,
            frame: 51,
        },
        1,
    )
    .unwrap();
    s.record_result(1, true).unwrap();
    assert_eq!(s.state(), State::ResultKnown);
}

// ---------- disconnect behaviour ----------

#[test]
fn disconnect_pending_command_goes_unknown_with_release_plan() {
    let mut s = connected();
    s.start_action(Op::Command, 3).unwrap();
    // Root correction: the plan is exactly the held inputs registered for
    // this action — never a value derived from the request id.
    let mut keys = [0u64; MAX_KEY_WORDS];
    keys[0] = 1 << 5;
    let mut pointers = [false; POINTER_SLOTS];
    pointers[1] = true;
    s.register_held_inputs(3, keys, pointers).unwrap();
    let plan = s.disconnect().expect("release plan");
    assert_eq!(s.state(), State::UnknownOutcome);
    assert_eq!(plan.keys, keys);
    assert_eq!(plan.pointers, pointers);
    // A returned plan is a request, NOT proof of actual release.
    assert_eq!(s.release_plan(), Some(plan));
}

#[test]
fn disconnect_pending_input_goes_unknown_and_refuses_blind_reconnect_start() {
    let mut s = connected();
    s.start_action(Op::Input, 4).unwrap();
    s.disconnect().expect("plan");
    assert_eq!(s.state(), State::UnknownOutcome);
    // No automatic retry: cannot reconnect or start while unknown is unresolved.
    assert_eq!(s.reconnect(), Err(SessionError::AlreadyConnected));
    let before = s.snapshot();
    assert_eq!(
        s.accept_trusted_peer(peer(7)),
        Err(SessionError::AlreadyConnected)
    );
    assert_eq!(
        s.start_action(Op::Input, 5),
        Err(SessionError::NotConnected)
    );
    assert_eq!(s.snapshot(), before);
}

#[test]
fn disconnect_pending_logs_cancels_not_fabricates_success() {
    let mut s = connected();
    s.start_action(Op::Logs, 6).unwrap();
    assert_eq!(s.disconnect(), None);
    // Cancelled: no UnknownOutcome, no ResultKnown.
    assert_eq!(s.state(), State::Ready);
    assert_eq!(s.release_plan(), None);
}

#[test]
fn disconnect_acked_command_goes_unknown_too() {
    let mut s = connected();
    s.start_action(Op::Command, 7).unwrap();
    s.record_ack(7).unwrap();
    assert!(s.disconnect().is_some());
    assert_eq!(s.state(), State::UnknownOutcome);
}

#[test]
fn disconnect_when_disconnected_is_noop() {
    let mut s = policy();
    assert_eq!(s.disconnect(), None);
    assert_eq!(s.state(), State::Disconnected);
}

// ---------- explicit external resolution ----------

#[test]
fn unknown_outcome_resolves_only_by_explicit_decision() {
    let mut s = connected();
    s.start_action(Op::Command, 8).unwrap();
    s.disconnect().expect("plan");
    assert_eq!(s.state(), State::UnknownOutcome);
    let before = s.snapshot();
    // Repeated resolve attempts after the first succeed only once.
    assert_eq!(before.state, State::UnknownOutcome);
    s.resolve_unknown_outcome().unwrap();
    assert_eq!(s.state(), State::Disconnected);
    assert_eq!(s.release_plan(), None);
    // Root correction: the frozen test discarded its snapshot. The failed
    // second resolve must provably change nothing.
    let settled = s.snapshot();
    assert_eq!(
        s.resolve_unknown_outcome(),
        Err(SessionError::NothingToResolve)
    );
    assert_eq!(s.snapshot(), settled);
}

#[test]
fn resolve_wrong_state_rejected_without_mutation() {
    let mut s = connected();
    let before = s.snapshot();
    assert_eq!(
        s.resolve_unknown_outcome(),
        Err(SessionError::NothingToResolve)
    );
    assert_eq!(s.snapshot(), before);
}

#[test]
fn after_resolution_reconnect_and_new_higher_id_progress() {
    let mut s = connected();
    s.start_action(Op::Command, 9).unwrap();
    s.disconnect().expect("plan");
    s.resolve_unknown_outcome().unwrap();
    s.accept_trusted_peer(peer(7)).unwrap();
    assert_eq!(s.state(), State::Ready);
    s.start_action(Op::Logs, 10).unwrap();
    assert_eq!(s.state(), State::Pending);
}

// ---------- pending release failure stops progress ----------

#[test]
fn unresolved_pending_release_blocks_new_actions() {
    let mut s = connected();
    s.start_action(Op::Input, 11).unwrap();
    let mut keys = [0u64; MAX_KEY_WORDS];
    keys[2] = 1 << 3;
    s.register_held_inputs(11, keys, [false; POINTER_SLOTS])
        .unwrap();
    let plan = s.disconnect().expect("plan");
    // Bounded plan shape: key bitset words + pointer slots only.
    assert_eq!(plan.keys.len(), MAX_KEY_WORDS);
    assert_eq!(plan.pointers.len(), POINTER_SLOTS);
    let before = s.snapshot();
    assert!(before.pending_release);
    assert_eq!(
        s.start_action(Op::Input, 12),
        Err(SessionError::NotConnected)
    );
    assert_eq!(s.snapshot(), before);
}

// ---------- duplicate end transitions and reset limits ----------

#[test]
fn duplicate_result_end_transition_rejected() {
    let mut s = connected();
    s.start_action(Op::Logs, 12).unwrap();
    s.record_result(12, true).unwrap();
    assert_eq!(s.state(), State::ResultKnown);
    let before = s.snapshot();
    assert_eq!(s.record_result(12, true), Err(SessionError::UnknownRequest));
    assert_eq!(s.snapshot(), before);
}

#[test]
fn reconnect_from_ready_or_pending_rejected() {
    let mut s = connected();
    // Root correction: the frozen test discarded its snapshot. Each rejected
    // reconnect must provably change nothing.
    let before = s.snapshot();
    assert_eq!(s.reconnect(), Err(SessionError::AlreadyConnected));
    assert_eq!(s.snapshot(), before);
    s.start_action(Op::Logs, 13).unwrap();
    let before = s.snapshot();
    assert_eq!(s.reconnect(), Err(SessionError::AlreadyConnected));
    assert_eq!(s.snapshot(), before);
}

#[test]
fn start_requires_connection() {
    let mut s = policy();
    let before = s.snapshot();
    assert_eq!(s.start_action(Op::Logs, 1), Err(SessionError::NotConnected));
    assert_eq!(s.snapshot(), before);
}

// ---------- release plan boundedness ----------

#[test]
fn release_plan_is_bounded_and_copyable() {
    let p = ReleasePlan::default();
    assert!(p.is_empty());
    assert_eq!(p.keys.len(), MAX_KEY_WORDS);
    assert_eq!(p.pointers.len(), POINTER_SLOTS);
}

// ---------- root regression tests: defects found in review of c2e1538 ----------

#[test]
fn disconnect_from_unknown_outcome_does_not_exit_or_unblock() {
    let mut s = connected();
    s.start_action(Op::Input, 21).unwrap();
    let mut keys = [0u64; MAX_KEY_WORDS];
    keys[0] = 1 << 9;
    s.register_held_inputs(21, keys, [false; POINTER_SLOTS])
        .unwrap();
    let plan = s.disconnect().expect("plan");
    assert_eq!(s.state(), State::UnknownOutcome);
    // A second disconnect is not an exit: same state, same plan, start blocked.
    assert_eq!(s.disconnect(), Some(plan));
    assert_eq!(s.state(), State::UnknownOutcome);
    let before = s.snapshot();
    assert_eq!(
        s.start_action(Op::Logs, 22),
        Err(SessionError::NotConnected)
    );
    assert_eq!(s.snapshot(), before);
    // The one legitimate exit still works afterwards.
    s.resolve_unknown_outcome().unwrap();
    assert_eq!(s.state(), State::Disconnected);
}

#[test]
fn disconnect_with_nothing_in_flight_actually_disconnects() {
    let mut s = connected();
    assert_eq!(s.disconnect(), None);
    assert_eq!(s.state(), State::Disconnected);
    let before = s.snapshot();
    assert_eq!(s.start_action(Op::Logs, 1), Err(SessionError::NotConnected));
    assert_eq!(s.snapshot(), before);
    // And the peer can be accepted again afterwards.
    s.accept_trusted_peer(peer(7)).unwrap();
    assert_eq!(s.state(), State::Ready);
}

#[test]
fn disconnect_from_result_known_goes_disconnected() {
    let mut s = connected();
    s.start_action(Op::Logs, 31).unwrap();
    s.record_result(31, true).unwrap();
    assert_eq!(s.state(), State::ResultKnown);
    assert_eq!(s.disconnect(), None);
    assert_eq!(s.state(), State::Disconnected);
}

#[test]
fn premature_result_then_ack_then_result_completes() {
    let mut s = connected();
    s.start_action(Op::Command, 41).unwrap();
    s.record_result(41, true).unwrap();
    assert_eq!(s.state(), State::Pending);
    s.record_ack(41).unwrap();
    assert_eq!(s.state(), State::Acknowledged);
    s.record_result(41, true).unwrap();
    assert_eq!(s.state(), State::ResultKnown);
}

#[test]
fn release_plan_for_unregistered_action_is_empty_not_fabricated() {
    let mut s = connected();
    s.start_action(Op::Command, 64).unwrap();
    // id 64 would fabricate keys[0] bit 0 / pointer slot 4 under the old code.
    let plan = s.disconnect().expect("plan");
    assert_eq!(s.state(), State::UnknownOutcome);
    assert!(plan.is_empty());
    assert_eq!(s.release_plan(), None);
}

#[test]
fn reference_frame_locks_after_observation_and_never_goes_backwards() {
    let mut s = connected();
    s.start_action(Op::Input, 51).unwrap();
    s.set_reference_frame(51, 50).unwrap();
    let before = s.snapshot();
    assert_eq!(
        s.set_reference_frame(51, 50),
        Err(SessionError::ReferenceFrameLocked)
    );
    assert_eq!(
        s.set_reference_frame(51, 49),
        Err(SessionError::ReferenceFrameLocked)
    );
    assert_eq!(s.snapshot(), before);
    s.record_ack(51).unwrap();
    s.record_observation(
        FrameRef {
            boot: BOOT,
            build: BUILD,
            frame: 51,
        },
        51,
    )
    .unwrap();
    // After an observation, the reference frame is locked outright: it must
    // not be rewritten to launder a stale observation into a newer one.
    let before = s.snapshot();
    assert_eq!(
        s.set_reference_frame(51, 9999),
        Err(SessionError::ReferenceFrameLocked)
    );
    assert_eq!(s.snapshot(), before);
}

#[test]
fn register_held_inputs_wrong_id_rejected_without_mutation() {
    let mut s = connected();
    s.start_action(Op::Input, 61).unwrap();
    let before = s.snapshot();
    assert_eq!(
        s.register_held_inputs(62, [0u64; MAX_KEY_WORDS], [false; POINTER_SLOTS]),
        Err(SessionError::UnknownRequest)
    );
    assert_eq!(s.snapshot(), before);
}
