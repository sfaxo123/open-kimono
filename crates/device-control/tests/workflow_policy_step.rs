//! DC-WF-01A: one host-policy workflow test composing the existing
//! device-control session/action rules across three sequential stages:
//! (1) Input staleness before ResultKnown, (2) disconnect -> UnknownOutcome
//! with no blind retry, (3) Command result-before-ACK stays Pending until ACK.

use device_control::session::{FrameRef, PeerId, SessionError, SessionPolicy, State};
use device_control::Op;

#[test]
fn workflow_policy_composition() {
    let build = [1u8; 16];
    let boot = [2u8; 16];
    let peer = PeerId::new(7).expect("nonzero peer");

    let mut policy = SessionPolicy::new(build, boot, peer).expect("valid policy");
    policy.accept_trusted_peer(peer).expect("accept peer");

    // ---- Stage 1: Input request 1.
    // ACK + successful result ALONE leave Acknowledged: an Input result is
    // not complete until a newer-frame observation lands.
    policy.start_action(Op::Input, 1).expect("start input req1");
    policy.mark_sent(1).expect("mark req1 sent");
    policy
        .set_reference_frame(1, 10)
        .expect("reference frame10");
    policy.record_ack(1).expect("ack req1");
    policy
        .record_result(1, true)
        .expect("successful result req1");
    assert_eq!(policy.state(), State::Acknowledged);
    let snapshot = policy.snapshot();

    // Observation with the same identities and frame10 (not newer than the
    // reference) is stale; the snapshot is unchanged.
    let f10 = FrameRef {
        boot,
        build,
        frame: 10,
    };
    let stale = policy.record_observation(f10, 1);
    assert_eq!(stale, Err(SessionError::StaleObservation));
    assert_eq!(policy.snapshot(), snapshot);

    // Observation frame11 (newer) marks observed_newer; state stays
    // Acknowledged until the result arrives.
    let f11 = FrameRef {
        boot,
        build,
        frame: 11,
    };
    policy.record_observation(f11, 1).expect("observe frame11");
    assert_eq!(policy.state(), State::Acknowledged);

    // With ack + newer observation already recorded, the successful result
    // now completes the Input and reaches ResultKnown.
    policy
        .record_result(1, true)
        .expect("completing result req1");
    assert_eq!(policy.state(), State::ResultKnown);

    // ---- Stage 2: Input request 2 — disconnect leaves UnknownOutcome.
    policy.start_action(Op::Input, 2).expect("start input req2");
    policy.mark_sent(2).expect("mark req2 sent");
    policy
        .set_reference_frame(2, 11)
        .expect("reference frame11");
    policy.disconnect();
    assert_eq!(policy.state(), State::UnknownOutcome);
    let snapshot2 = policy.snapshot();

    // Reconnect attempt via the gate check is blocked while the unknown
    // outcome is unresolved, and the snapshot is unchanged. No blind retry.
    assert_eq!(policy.reconnect(), Err(SessionError::AlreadyConnected));
    assert_eq!(policy.snapshot(), snapshot2);
    // A blind new request is likewise refused — the disconnected gate fires
    // before the unresolved-release check — snapshot still unchanged.
    assert_eq!(
        policy.start_action(Op::Input, 3),
        Err(SessionError::NotConnected)
    );
    assert_eq!(policy.snapshot(), snapshot2);

    // Resolve explicitly, then re-accept the peer through the trust boundary.
    policy
        .resolve_unknown_outcome()
        .expect("resolve req2 explicitly");
    assert_eq!(policy.reconnect(), Ok(()));
    policy
        .accept_trusted_peer(peer)
        .expect("re-accept peer after resolve");

    // ---- Stage 3: Command request 3 — successful result BEFORE ACK must
    // leave the action Pending (a pre-ACK result fabricates nothing).
    policy
        .start_action(Op::Command, 3)
        .expect("start command req3");
    policy.mark_sent(3).expect("mark req3 sent");
    policy.record_result(3, true).expect("early result req3");
    assert_eq!(policy.state(), State::Pending);

    // ACK moves to Acknowledged; the early result did not fabricate it.
    policy.record_ack(3).expect("ack req3");
    assert_eq!(policy.state(), State::Acknowledged);

    // The ACK does not retroactively complete the action: a later result is
    // required. With ack now recorded, that result reaches ResultKnown.
    policy
        .record_result(3, true)
        .expect("completing result req3");
    assert_eq!(policy.state(), State::ResultKnown);
}
