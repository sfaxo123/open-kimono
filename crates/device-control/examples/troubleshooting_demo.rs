//! DC-WF-01B partial 0.2.4 host troubleshooting walkthrough.
//!
//! A bounded, deterministic walk of the host-side observe + act slices
//! against SCRIPTED fixtures only: no device, no transport, no clock, no
//! randomness. `cargo run --example troubleshooting_demo` twice yields
//! byte-identical stdout, and every stated count/byte/state invariant is
//! asserted in Rust (a violation panics), not merely printed.
//!
//! Stage 1 decodes the scripted LogRecord/ImageRecord frames through the
//! same public codec paths the CLI uses. Stage 2 drives one scripted ACK +
//! successful RESULT through the bounded act executor. Stage 3 drives a
//! fresh ACK-only script with a finite executor budget and asserts the
//! UnknownOutcome result while the session policy stays Acknowledged
//! (in-flight) — the executor policy does NOT turn SessionPolicy into
//! UnknownOutcome by itself. Stage 4 prints the honest boundaries.

use device_control::act::{
    script_ack, script_result, ActExecutor, ScenarioScript, ScenarioTransport,
};
use device_control::command_record::result_contract::ResultStatus;
use device_control::command_record::CommandRequest;
use device_control::fake_transport::{open, Script, TransportKind};
use device_control::image_record::ImageRecord;
use device_control::log_record::LogRecord;
use device_control::session::{PeerId, SessionPolicy, State};
use device_control::{Op, Policy};

const BOOT: [u8; 16] = [3u8; 16];
const BUILD: [u8; 16] = [7u8; 16];
const SESSION_ID: u64 = 42;
const PEER: u64 = 9;

/// Pinned geometry of the scripted screenshot fixture.
const EXPECTED_LOG_FRAMES: usize = 2;
const EXPECTED_IMAGE_FRAMES: usize = 1;
const EXPECTED_WIDTH: u32 = 2;
const EXPECTED_HEIGHT: u32 = 2;
const EXPECTED_STRIDE: u32 = 8;
const EXPECTED_PIXEL_BYTES: usize = 16;

/// Same FNV-1a-64 payload checksum the binary prints.
fn payload_checksum(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn connected_policy() -> SessionPolicy {
    let peer = PeerId::new(PEER).expect("test peer id");
    let mut p = SessionPolicy::new(BUILD, BOOT, peer).expect("policy builds");
    p.accept_trusted_peer(peer).expect("peer accepted");
    p
}

fn status_label(status: ResultStatus) -> &'static str {
    match status {
        ResultStatus::Success => "success",
        ResultStatus::Failure => "failure",
        ResultStatus::UnknownOutcome => "unknown",
    }
}

fn main() {
    println!("SCRIPTED HOST FIXTURE — no device, no transport, no clock, no randomness");

    // ---- Stage 1: scripted observe (logs + screenshot) ------------------
    let script = Script::new();
    let policy = Policy::default();

    println!("== stage 1: scripted observe ==");
    let mut log_frames = 0usize;
    let mut image_frames = 0usize;
    let mut other_frames = 0usize;
    let mut pixel_bytes = 0usize;
    let mut screenshot_checksum = 0u64;
    let mut pixel_pattern: [u8; EXPECTED_PIXEL_BYTES] = [0u8; EXPECTED_PIXEL_BYTES];

    let mut t = open(TransportKind::Scripted, &script).expect("scripted transport opens");
    while let Some(frame) = t.serve_frame(&policy).expect("script frames decode") {
        match frame.op() {
            Op::Logs => {
                log_frames += 1;
                let logged = LogRecord::from_frame(&frame).expect("log frame decodes");
                let rec = logged.record();
                println!(
                    "log seq={} mono_us={} sev={:?} sub={} msg={}",
                    rec.sequence(),
                    rec.monotonic_us(),
                    rec.severity(),
                    rec.subsystem(),
                    rec.message(),
                );
            }
            Op::Screenshot => {
                image_frames += 1;
                let img = ImageRecord::from_frame(&frame).expect("image frame decodes");
                let rec = img.record();
                pixel_bytes = rec.pixels().len();
                screenshot_checksum = payload_checksum(rec.pixels());
                println!(
                    "screenshot frame={} {}x{} stride={} fmt={:?} rot={:?} pixels={} checksum_fnv1a64={:016x}",
                    rec.frame_seq(),
                    rec.width(),
                    rec.height(),
                    rec.stride(),
                    rec.pixel_format(),
                    rec.rotation(),
                    pixel_bytes,
                    screenshot_checksum,
                );
                assert_eq!(rec.width(), EXPECTED_WIDTH, "pinned screenshot width");
                assert_eq!(rec.height(), EXPECTED_HEIGHT, "pinned screenshot height");
                assert_eq!(rec.stride(), EXPECTED_STRIDE, "pinned screenshot stride");
                assert_eq!(
                    pixel_bytes, EXPECTED_PIXEL_BYTES,
                    "pinned screenshot pixel byte count"
                );
                assert_eq!(
                    pixel_bytes,
                    (rec.width() as usize) * (rec.height() as usize) * 4,
                    "2x2 RGBA8 identity: pixels == width*height*4"
                );
                pixel_pattern.copy_from_slice(rec.pixels());
            }
            other => {
                other_frames += 1;
                println!("other op={:?} (ignored by this walkthrough)", other);
            }
        }
    }

    println!(
        "stage1 counts logs={} images={} other={}",
        log_frames, image_frames, other_frames
    );
    assert_eq!(
        log_frames, EXPECTED_LOG_FRAMES,
        "scripted fixture carries exactly two LogRecord frames"
    );
    assert_eq!(
        image_frames, EXPECTED_IMAGE_FRAMES,
        "scripted fixture carries exactly one ImageRecord frame"
    );
    assert_eq!(
        other_frames, 0,
        "scripted fixture carries no other ops here"
    );
    assert_eq!(
        pixel_pattern, [0x10u8; EXPECTED_PIXEL_BYTES],
        "pinned screenshot payload identity: 16 bytes of 0x10"
    );
    assert_eq!(
        screenshot_checksum,
        payload_checksum(&[0x10u8; EXPECTED_PIXEL_BYTES]),
        "pinned screenshot payload checksum matches the pinned 0x10 pattern"
    );

    // ---- Stage 2: scripted successful command result --------------------
    static ARGS: [&[u8]; 1] = [b"ls".as_slice()];
    let request = CommandRequest::new(5000, &ARGS).expect("command request builds");

    println!("== stage 2: scripted command result (not an OS execution) ==");
    let ack = script_ack(1, &BOOT);
    let res = script_result(1, ResultStatus::Success, b"hello", b"", &BOOT);
    let frames = [(&ack.0[..], ack.1), (&res.0[..], res.1)];
    let scenario = ScenarioScript::new(&frames);
    let mut transport = ScenarioTransport::new(&scenario);
    let mut policy = connected_policy();
    // The executor holds `&mut policy` for its whole lifetime, so the run and
    // its request count are captured inside a scope that owns the executor;
    // `State` is `Copy`, so the policy is read after that borrow ends.
    let (result, requests_sent) = {
        let mut exec = ActExecutor::new(&mut transport, &mut policy, BOOT, SESSION_ID);
        let result = exec.run(&request, 8).expect("scripted success completes");
        (result, exec.requests_sent())
    };
    let stdout = core::str::from_utf8(&result.stdout.bytes[..result.stdout.len])
        .expect("scripted stdout is utf-8");
    println!(
        "stage2 scripted_result={} stdout=\"{}\" truncated={} seq={} requests_sent={} state={:?}",
        status_label(result.status),
        stdout,
        result.stdout.truncated,
        result.correlation.request_seq,
        requests_sent,
        policy.state(),
    );
    assert_eq!(
        result.status,
        ResultStatus::Success,
        "scripted result is success"
    );
    assert_eq!(
        stdout, "hello",
        "scripted stdout bytes are exactly \"hello\""
    );
    assert_eq!(result.stdout.len, 5, "scripted stdout length");
    assert!(!result.stdout.truncated, "scripted stdout not truncated");
    assert_eq!(requests_sent, 1, "exactly one request sent");
    assert_eq!(
        policy.state(),
        State::ResultKnown,
        "scripted success must close the policy slot"
    );

    // ---- Stage 3: ACK-only script, finite budget -> UnknownOutcome ------
    println!("== stage 3: ACK-only script + finite budget (no retry) ==");
    let ack = script_ack(1, &BOOT);
    let frames = [(&ack.0[..], ack.1)];
    let scenario = ScenarioScript::new(&frames);
    let mut transport = ScenarioTransport::new(&scenario);
    let mut policy = connected_policy();
    // Same scoping as stage 2: the executor's mutable borrow of the policy
    // ends before `policy.state()` is read and asserted.
    let (result, requests_sent) = {
        let mut exec = ActExecutor::new(&mut transport, &mut policy, BOOT, SESSION_ID);
        let result = exec
            .run(&request, 8)
            .expect("ack-only scenario reports unknown");
        (result, exec.requests_sent())
    };
    println!(
        "stage3 scripted_result={} stdout_len={} seq={} requests_sent={} state={:?}",
        status_label(result.status),
        result.stdout.len,
        result.correlation.request_seq,
        requests_sent,
        policy.state(),
    );
    assert_eq!(
        result.status,
        ResultStatus::UnknownOutcome,
        "ack-only budget exhaustion reports UnknownOutcome"
    );
    assert_eq!(result.stdout.len, 0, "no stdout bytes arrived");
    assert_eq!(requests_sent, 1, "no blind replay: one request sent");
    assert_eq!(
        policy.state(),
        State::Acknowledged,
        "session policy stays Acknowledged/in-flight — the executor does not rewrite it to UnknownOutcome"
    );

    // ---- Stage 4: honest boundaries -------------------------------------
    println!("== stage 4: boundaries ==");
    println!("boundary: scripted host fixture only — no auth, no real transport, no device logs, no UI-effect proof");
    println!("boundary: stage 2 prints a SCRIPTED result, not a command executed on an OS or a transport send");
    println!("boundary: no full troubleshooting_workflow / 0.2.4 acceptance; malformed-input, full-resource and end-to-end gates remain open");
}
