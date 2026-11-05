use device_control::act::{
    script_ack, script_result, ActExecutor, ScenarioScript, ScenarioTransport,
};
use device_control::command_record::{result_contract::ResultStatus, CommandRequest};
use device_control::session::{PeerId, SessionPolicy};

fn ready() -> SessionPolicy {
    let peer = PeerId::new(9).unwrap();
    let mut policy = SessionPolicy::new([7; 16], [3; 16], peer).unwrap();
    policy.accept_trusted_peer(peer).unwrap();
    policy
}
fn request() -> CommandRequest<'static> {
    static ARGS: [&[u8]; 1] = [b"prototype-action"];
    CommandRequest::new(5000, &ARGS).unwrap()
}

#[test]
fn explicit_unknown_result_must_not_clear_inflight_as_success() {
    let ack = script_ack(1, &[3; 16]);
    let unknown = script_result(1, ResultStatus::UnknownOutcome, b"", b"", &[3; 16]);
    let frames = [(&ack.0[..], ack.1), (&unknown.0[..], unknown.1)];
    let script = ScenarioScript::new(&frames);
    let mut transport = ScenarioTransport::new(&script);
    let mut policy = ready();
    let mut executor = ActExecutor::new(&mut transport, &mut policy, [3; 16], 42);
    let result = executor.run(&request(), 8).unwrap();
    assert_eq!(result.status, ResultStatus::UnknownOutcome);
    drop(executor);
    assert_eq!(
        policy.snapshot().inflight_id,
        Some(1),
        "unknown execution cannot become a success closure"
    );
}

#[test]
fn sent_without_ack_cannot_be_cancelled_by_forging_success() {
    let frames = [];
    let script = ScenarioScript::new(&frames);
    let mut transport = ScenarioTransport::new(&script);
    let mut policy = ready();
    let mut executor = ActExecutor::new(&mut transport, &mut policy, [3; 16], 42);
    assert!(executor.run(&request(), 8).is_err());
    assert_eq!(executor.requests_sent(), 1);
    assert!(
        executor.cancel(1).is_err(),
        "sent/no-ACK is ambiguous, not proven unsent cancellation"
    );
}

#[test]
fn rebuilding_executor_cannot_erase_sent_without_ack_evidence() {
    let frames = [];
    let script = ScenarioScript::new(&frames);
    let mut first_transport = ScenarioTransport::new(&script);
    let mut policy = ready();
    let mut first = ActExecutor::new(&mut first_transport, &mut policy, [3; 16], 42);
    assert!(first.run(&request(), 8).is_err());
    assert_eq!(first.requests_sent(), 1);
    drop(first);
    assert_eq!(policy.snapshot().inflight_id, Some(1));
    let mut second_transport = ScenarioTransport::new(&script);
    let mut second = ActExecutor::new(&mut second_transport, &mut policy, [3; 16], 42);
    assert!(
        second.cancel(1).is_err(),
        "reconstructing an executor does not prove its pending action was unsent"
    );
}
