//! Three focused tests for the `redox-devicectl` observe slice (DC-CLI-01E).
//!
//! Tests 1 and 2 drive the observe slice in-process by replicating the
//! exact serve/decode/format pipeline the binary implements, pinned to
//! literal expected outputs; test 1 ALSO runs the REAL binary twice via
//! `env!("CARGO_BIN_EXE_redox-devicectl")` and requires byte-identical
//! stdout matching the in-process render. Test 3 executes the REAL
//! binary with `--transport usb` and asserts the UNSUPPORTED-TRANSPORT
//! line on stdout with nothing else on stdout/stderr, plus a genuinely
//! observed nonzero process exit code — a bin that fell back to the
//! scripted transport on Usb would fail here.

use std::process::Command;

use device_control::fake_transport::{open, Script, TransportKind};
use device_control::image_record::ImageRecord;
use device_control::log_record::LogRecord;
use device_control::{Op, Policy};

/// Mirror of the binary's FNV-1a-64 payload checksum.
fn payload_checksum(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Mirror of the binary's `logs tail` rendering: serve ALL scripted
/// frames, decode every `Logs` body via log_record, one line per record.
fn logs_tail_output(script: &Script) -> String {
    let policy = Policy::default();
    let mut transport = open(TransportKind::Scripted, script).expect("scripted opens");
    let mut out = String::new();
    while let Some(frame) = transport
        .serve_frame(&policy)
        .expect("script frames decode")
    {
        if frame.op() != Op::Logs {
            continue;
        }
        let logged = LogRecord::from_frame(&frame).expect("log decodes");
        let rec = logged.record();
        out.push_str(&format!(
            "seq={} mono_us={} sev={:?} sub={} msg={}\n",
            rec.sequence(),
            rec.monotonic_us(),
            rec.severity(),
            rec.subsystem(),
            rec.message(),
        ));
    }
    out
}

/// Mirror of the binary's `screenshot capture` rendering: serve ALL
/// scripted frames, decode the `Screenshot` body via image_record, print
/// one deterministic summary line.
fn screenshot_capture_output(script: &Script) -> String {
    let policy = Policy::default();
    let mut transport = open(TransportKind::Scripted, script).expect("scripted opens");
    let mut out = String::new();
    while let Some(frame) = transport
        .serve_frame(&policy)
        .expect("script frames decode")
    {
        if frame.op() != Op::Screenshot {
            continue;
        }
        let img = ImageRecord::from_frame(&frame).expect("image decodes");
        let rec = img.record();
        out.push_str(&format!(
            "screenshot frame={} {}x{} stride={} fmt={:?} rot={:?} pixels={} checksum_fnv1a64={:016x}\n",
            rec.frame_seq(),
            rec.width(),
            rec.height(),
            rec.stride(),
            rec.pixel_format(),
            rec.rotation(),
            rec.pixels().len(),
            payload_checksum(rec.pixels()),
        ));
    }
    out
}

/// Test 1: `logs tail` output is byte-identical across two runs of the
/// REAL binary (CARGO_BIN_EXE), matches the in-process render of the
/// exact decode pipeline, AND matches the expected decoded content.
#[test]
fn logs_tail_is_deterministic_and_matches_decoded_script_content() {
    // Two separate process invocations of the actual CLI binary.
    let exe = env!("CARGO_BIN_EXE_redox-devicectl");
    let first = Command::new(exe)
        .args(["logs", "tail"])
        .output()
        .expect("spawn redox-devicectl logs tail");
    let second = Command::new(exe)
        .args(["logs", "tail"])
        .output()
        .expect("spawn redox-devicectl logs tail");

    assert!(
        first.status.success(),
        "first run exited {:?}",
        first.status
    );
    assert!(
        second.status.success(),
        "second run exited {:?}",
        second.status
    );
    assert!(
        first.stderr.is_empty(),
        "stderr not empty: {:?}",
        first.stderr
    );

    // Byte-identical stdout across two process runs (no wall-clock, no
    // randomness).
    assert_eq!(first.stdout, second.stdout);

    // The real binary's bytes equal the in-process render of the exact
    // serve/decode pipeline, and both equal the expected decoded content.
    // (Script: seq 1 mono_us 1_000+1, seq 2 mono_us 1_000+2,
    // Severity::Info, subsystem "fake", messages "fake boot"/"fake ready".)
    let expected = "seq=1 mono_us=1001 sev=Info sub=fake msg=fake boot\n\
                    seq=2 mono_us=1002 sev=Info sub=fake msg=fake ready\n";
    assert_eq!(
        String::from_utf8(first.stdout.clone()).expect("utf8 stdout"),
        expected
    );
    let script = Script::new();
    assert_eq!(logs_tail_output(&script), expected);
    assert_eq!(expected.lines().count(), 2);
}

/// Test 2: `screenshot capture` output from TWO separate invocations of
/// the REAL binary (CARGO_BIN_EXE) is byte-identical AND equals the
/// pinned deterministic summary line — the exact text, not just mutual
/// equality. (The checksum derivation and the FNV-1a-64 known-answer
/// below pin the value independently, so a wrong payload or checksum
/// function fails even if the bin merely self-agrees.)
#[test]
fn screenshot_capture_is_deterministic_summary() {
    let exe = env!("CARGO_BIN_EXE_redox-devicectl");
    let first = Command::new(exe)
        .args(["screenshot", "capture"])
        .output()
        .expect("spawn redox-devicectl screenshot capture");
    let second = Command::new(exe)
        .args(["screenshot", "capture"])
        .output()
        .expect("spawn redox-devicectl screenshot capture");

    assert!(
        first.status.success(),
        "first run exited {:?}",
        first.status
    );
    assert!(
        second.status.success(),
        "second run exited {:?}",
        second.status
    );
    assert!(
        first.stderr.is_empty(),
        "stderr not empty: {:?}",
        first.stderr
    );

    // Byte-identical stdout across two process runs (no wall-clock, no
    // randomness).
    assert_eq!(first.stdout, second.stdout);

    // The scripted frame is 2x2 RGBX8888, stride 8, 16 pixel bytes 0x10.
    // The checksum is independently derived from the pixel bytes via the
    // same pure FNV-1a-64 the CLI prints, so a wrong payload or a wrong
    // checksum function both fail here.
    let checksum = payload_checksum(&[0x10u8; 16]);
    let expected = format!(
        "screenshot frame=1 2x2 stride=8 fmt=Rgbx8888 rot=None pixels=16 checksum_fnv1a64={checksum:016x}\n"
    );

    // The REAL binary's stdout equals the exact summary text.
    assert_eq!(
        String::from_utf8(first.stdout.clone()).expect("utf8 stdout"),
        expected
    );

    // Independent known-answer for FNV-1a-64 (offset basis 0xcbf29ce484222325,
    // prime 0x100000001b3: "a" = 0xaf63dc4c8601ec8c) pins the checksum as
    // the standard 64-bit function, and the full line is pinned as a
    // literal so any format drift fails.
    assert_eq!(payload_checksum(b"a"), 0xaf63_dc4c_8601_ec8c);
    let pinned = "screenshot frame=1 2x2 stride=8 fmt=Rgbx8888 rot=None pixels=16 checksum_fnv1a64=da6cd29d6f401465\n";
    assert_eq!(
        String::from_utf8(first.stdout).expect("utf8 stdout"),
        pinned
    );

    // The in-process mirror renders the identical line (shared contract).
    let script = Script::new();
    assert_eq!(screenshot_capture_output(&script), pinned);
}

/// Test 3: asking the REAL binary for the Usb transport yields exactly
/// the UNSUPPORTED-TRANSPORT line and an observed nonzero process exit
/// code — no fallback. A bin mutated to fall back to the scripted
/// transport on Usb fails here on both stdout and the exit code.
#[test]
fn usb_request_yields_unsupported_transport_line_and_nonzero_exit() {
    let exe = env!("CARGO_BIN_EXE_redox-devicectl");
    let out = Command::new(exe)
        .args(["--transport", "usb"])
        .output()
        .expect("spawn redox-devicectl --transport usb");

    // Observed, not hardcoded: the process itself exited nonzero.
    assert_ne!(out.status.code(), Some(0), "Usb must not exit 0");
    assert!(out.status.code().is_some(), "child must exit, not signal");

    // stdout is EXACTLY the one UNSUPPORTED-TRANSPORT line — nothing
    // else, and no scripted-transport output leaked in (no fallback).
    assert_eq!(
        String::from_utf8(out.stdout.clone()).expect("utf8 stdout"),
        "UNSUPPORTED-TRANSPORT\n"
    );
    assert!(
        out.stderr.is_empty(),
        "stderr must be empty, got {:?}",
        String::from_utf8_lossy(&out.stderr)
    );

    // The library-level contract the bin routes through still holds:
    // Usb is a typed error, Scripted still opens.
    let script = Script::new();
    assert!(open(TransportKind::Usb, &script).is_err());
    assert!(open(TransportKind::Scripted, &script).is_ok());
}
