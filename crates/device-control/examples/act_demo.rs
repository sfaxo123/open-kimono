//! DC-CLI-02 act-slice demo: one scripted SUCCESS and one scripted
//! TIMEOUT through the bounded act executor. Deterministic prints only —
//! no clock, no randomness, no heap.
//!
//! Scenario A: ACK then RESULT("hello") completes the action and the
//! policy returns to idle. Scenario B: ACK only, budget spent — the
//! outcome is UnknownOutcome, the policy stays in-flight, and the
//! request was sent exactly once (no blind replay).

use device_control::act::{
    script_ack, script_result, ActExecutor, ScenarioScript, ScenarioTransport,
};
use device_control::command_record::result_contract::ResultStatus;
use device_control::command_record::CommandRequest;
use device_control::session::{PeerId, SessionPolicy, State};

const BOOT: [u8; 16] = [3u8; 16];
const BUILD: [u8; 16] = [7u8; 16];
const SESSION_ID: u64 = 42;

fn connected_policy() -> SessionPolicy {
    let mut p = SessionPolicy::new(BUILD, BOOT, PeerId::new(9).unwrap()).unwrap();
    p.accept_trusted_peer(PeerId::new(9).unwrap()).unwrap();
    p
}

fn main() {
    static ARGS: [&[u8]; 1] = [b"ls".as_slice()];
    let request = CommandRequest::new(5000, &ARGS).unwrap();

    // Scenario A — success: ACK then RESULT carrying "hello" on stdout.
    let ack = script_ack(1, &BOOT);
    let res = script_result(1, ResultStatus::Success, b"hello", b"", &BOOT);
    let frames = [(&ack.0[..], ack.1), (&res.0[..], res.1)];
    let script = ScenarioScript::new(&frames);
    let mut transport = ScenarioTransport::new(&script);
    let mut policy = connected_policy();
    let mut exec = ActExecutor::new(&mut transport, &mut policy, BOOT, SESSION_ID);
    let result = exec.run(&request, 8).expect("scenario A succeeds");
    println!(
        "A status={} stdout=\"{}\" truncated={} seq={} sent={} state={:?}",
        match result.status {
            ResultStatus::Success => "success",
            ResultStatus::Failure => "failure",
            ResultStatus::UnknownOutcome => "unknown",
        },
        core::str::from_utf8(&result.stdout.bytes[..result.stdout.len]).unwrap(),
        result.stdout.truncated,
        result.correlation.request_seq,
        exec.requests_sent(),
        policy.state(),
    );

    // Scenario B — timeout: ACK only, the result never arrives. Fresh
    // policy and fresh transport, so the executor issues request id 1.
    let ack = script_ack(1, &BOOT);
    let frames = [(&ack.0[..], ack.1)];
    let script = ScenarioScript::new(&frames);
    let mut transport = ScenarioTransport::new(&script);
    let mut policy = connected_policy();
    let mut exec = ActExecutor::new(&mut transport, &mut policy, BOOT, SESSION_ID);
    let result = exec.run(&request, 8).expect("scenario B reports unknown");
    println!(
        "B status={} stdout_len={} seq={} sent={} state={:?}",
        match result.status {
            ResultStatus::Success => "success",
            ResultStatus::Failure => "failure",
            ResultStatus::UnknownOutcome => "unknown",
        },
        result.stdout.len,
        result.correlation.request_seq,
        exec.requests_sent(),
        policy.state(),
    );
    assert_eq!(result.status, ResultStatus::UnknownOutcome);
    assert_eq!(policy.state(), State::Acknowledged);
}
