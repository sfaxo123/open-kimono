//! HW-27 tests for the attempt process seam. PURE in-process fake runner —
//! ZERO process spawn, ZERO fastboot, ZERO phone contact. Run with:
//! cargo test --offline

use std::cell::Cell;
use std::path::Path;

use op6_boot::{attempt_run, AttemptOutcome, FastbootRunner, ProcOutcome};

// ---------------------------------------------------------------------------
// The fake runner: records every invocation, replays scripted outcomes.
// ---------------------------------------------------------------------------

/// One scripted (args, outcome) pair matched by the fastboot verb found
/// anywhere in the recorded args (HW-30 put `-s <serial>` before the boot
/// verb, so the verb is no longer always args[0]).
struct Scripted {
    verb: &'static str,
    outcome: Result<ProcOutcome, std::io::Error>,
}

struct FakeRunner {
    script: Vec<Scripted>,
    calls: std::cell::RefCell<Vec<Vec<String>>>,
}

impl FakeRunner {
    fn new(script: Vec<Scripted>) -> Self {
        FakeRunner {
            script,
            calls: std::cell::RefCell::new(Vec::new()),
        }
    }
    /// The fastboot verb of every invocation, in order (e.g. ["devices"] if
    /// boot never ran). HW-30 shape: ["devices", "boot"] still, even though
    /// the boot argv is now ["-s", serial, "boot", img].
    fn verbs(&self) -> Vec<String> {
        self.calls
            .borrow()
            .iter()
            .map(|a| verb_of_strs(a).to_string())
            .collect()
    }
    /// Full recorded argv for every invocation (args WITHOUT the program
    /// name), in order. HW-30: used to assert the exact boot arg shape.
    fn calls(&self) -> Vec<Vec<String>> {
        self.calls.borrow().clone()
    }
}

/// The whitelisted verb inside a recorded argv, if any.
fn verb_of_strs(args: &[String]) -> &'static str {
    for a in args {
        if a == "devices" {
            return "devices";
        }
        if a == "boot" {
            return "boot";
        }
    }
    ""
}

impl FastbootRunner for FakeRunner {
    fn run(&self, args: &[&str]) -> Result<ProcOutcome, std::io::Error> {
        self.calls
            .borrow_mut()
            .push(args.iter().map(|s| s.to_string()).collect());
        let verb = args
            .iter()
            .find(|a| **a == "devices" || **a == "boot")
            .copied()
            .unwrap_or("");
        // Match the first scripted entry for this verb; fall back to a
        // generic success so happy paths need only script what matters.
        for s in &self.script {
            if s.verb == verb {
                return match &s.outcome {
                    Ok(o) => Ok(o.clone()),
                    Err(_) => Err(std::io::Error::other("fake spawn failure")),
                };
            }
        }
        Ok(ProcOutcome {
            ok: true,
            stdout: String::new(),
            stderr: String::new(),
        })
    }
}

// helper constructors
fn ok_devices(stdout: &str) -> Scripted {
    Scripted {
        verb: "devices",
        outcome: Ok(ProcOutcome {
            ok: true,
            stdout: stdout.to_string(),
            stderr: String::new(),
        }),
    }
}
fn ok_boot() -> Scripted {
    Scripted {
        verb: "boot",
        outcome: Ok(ProcOutcome {
            ok: true,
            stdout: "OKAY [  0.123s] booting".to_string(),
            stderr: String::new(),
        }),
    }
}

const IMG: &str = "/tmp/op6-fake-image.img";

// ---------------------------------------------------------------------------
// HW-27 bug 1: devices exit status must be honored.
// ---------------------------------------------------------------------------

/// `fastboot devices` exits NONZERO with non-empty stdout: the old code
/// proceeded to boot on the stdout alone. The fix aborts and never boots.
#[test]
fn devices_nonzero_status_aborts() {
    let runner = FakeRunner::new(vec![Scripted {
        verb: "devices",
        outcome: Ok(ProcOutcome {
            ok: false,                                 // nonzero exit — the field the old code IGNORED
            stdout: "0a1b2c3d fastboot\n".to_string(), // non-empty!
            stderr: "error: device not found consistently".to_string(),
        }),
    }]);
    let out = attempt_run(&runner, Path::new(IMG), None, || true);
    match &out {
        AttemptOutcome::DevicesAbort { reason } => {
            assert!(reason.contains("nonzero"), "reason: {reason}");
            assert!(
                reason.contains("device not found consistently"),
                "stderr must be surfaced: {reason}"
            );
        }
        other => panic!("must be DevicesAbort, got {other:?}"),
    }
    assert_eq!(out.exit_code(), 1);
    // boot was NEVER called.
    assert_eq!(runner.verbs(), vec!["devices".to_string()]);
}

// ---------------------------------------------------------------------------
// The device-selection contract.
// ---------------------------------------------------------------------------

/// Two lines in state `fastboot` -> ambiguous target -> abort, no boot.
#[test]
fn devices_multi_line_aborts() {
    let runner = FakeRunner::new(vec![ok_devices("0a1b2c3d fastboot\nabcd1234 fastboot\n")]);
    let out = attempt_run(&runner, Path::new(IMG), None, || true);
    match &out {
        AttemptOutcome::SelectionAbort { reason } => {
            assert!(reason.contains("exactly one"), "reason: {reason}");
        }
        other => panic!("must be SelectionAbort, got {other:?}"),
    }
    assert_eq!(out.exit_code(), 1);
    assert_eq!(runner.verbs(), vec!["devices".to_string()]);
}

/// The only enumerated line has state `offline`, not `fastboot` -> zero
/// fastboot-state devices -> abort, no boot.
#[test]
fn devices_wrong_state_aborts() {
    let runner = FakeRunner::new(vec![ok_devices("0a1b2c3d offline\n")]);
    let out = attempt_run(&runner, Path::new(IMG), None, || true);
    match &out {
        AttemptOutcome::SelectionAbort { reason } => {
            assert!(reason.contains("no device"), "reason: {reason}");
        }
        other => panic!("must be SelectionAbort, got {other:?}"),
    }
    assert_eq!(out.exit_code(), 1);
    assert_eq!(runner.verbs(), vec!["devices".to_string()]);
}

/// Exactly one fastboot device, but `--expect-serial` names a different
/// serial -> abort, no boot.
#[test]
fn expect_serial_mismatch_aborts() {
    let runner = FakeRunner::new(vec![ok_devices("0a1b2c3d fastboot\n")]);
    let out = attempt_run(&runner, Path::new(IMG), Some("ffffffff"), || true);
    match &out {
        AttemptOutcome::SelectionAbort { reason } => {
            assert!(reason.contains("0a1b2c3d"), "reason: {reason}");
            assert!(reason.contains("ffffffff"), "reason: {reason}");
        }
        other => panic!("must be SelectionAbort, got {other:?}"),
    }
    assert_eq!(out.exit_code(), 1);
    assert_eq!(runner.verbs(), vec!["devices".to_string()]);
}

// ---------------------------------------------------------------------------
// HW-27 bug 2: boot process failure must NOT map to exit 0.
// ---------------------------------------------------------------------------

/// devices ok + selection ok, but the boot process reported ok == false:
/// outcome is BootFailed (exit 4) and its message carries the report.
#[test]
fn boot_failure_maps_to_exit4() {
    let runner = FakeRunner::new(vec![
        ok_devices("0a1b2c3d fastboot\n"),
        Scripted {
            verb: "boot",
            outcome: Ok(ProcOutcome {
                ok: false, // the old code returned 0 regardless of this
                stdout: String::new(),
                stderr: "FAILED (remote: 'Boot image invalid')".to_string(),
            }),
        },
    ]);
    let out = attempt_run(&runner, Path::new(IMG), None, || true);
    match &out {
        AttemptOutcome::BootFailed { message } => {
            assert!(message.contains("FAILED"), "message: {message}");
            assert!(message.contains("Boot image invalid"), "message: {message}");
        }
        other => panic!("must be BootFailed, got {other:?}"),
    }
    assert_eq!(out.exit_code(), 4);
    assert_eq!(
        runner.verbs(),
        vec!["devices".to_string(), "boot".to_string()]
    );
    // HW-30: the boot invocation is bound to the selected serial.
    let boot_call = runner.calls().pop().expect("boot was called");
    assert_eq!(
        boot_call,
        vec![
            "-s".to_string(),
            "0a1b2c3d".to_string(),
            "boot".to_string(),
            IMG.to_string()
        ]
    );
}

// ---------------------------------------------------------------------------
// Happy path + the double verify.
// ---------------------------------------------------------------------------

/// Full happy path: devices ok, one fastboot line (multiple spaces tolerated),
/// boot ok -> Booted / exit 0. The verify closure must have been called
/// EXACTLY TWICE (pre-flight + immediately before boot).
#[test]
fn boot_success_exit0() {
    let runner = FakeRunner::new(vec![
        ok_devices("0a1b2c3d    fastboot\n"), // multiple spaces tolerated
        ok_boot(),
    ]);
    let verify_calls = Cell::new(0u32);
    let out = attempt_run(&runner, Path::new(IMG), Some("0a1b2c3d"), || {
        verify_calls.set(verify_calls.get() + 1);
        true
    });
    assert_eq!(out, AttemptOutcome::Booted);
    assert_eq!(out.exit_code(), 0);
    assert_eq!(
        verify_calls.get(),
        2,
        "double verify: verify must run pre-flight AND immediately before boot"
    );
    assert_eq!(
        runner.verbs(),
        vec!["devices".to_string(), "boot".to_string()]
    );
    // HW-30: exact boot argv — the selected serial is bound via -s.
    let boot_call = runner.calls().pop().expect("boot was called");
    assert_eq!(
        boot_call,
        vec![
            "-s".to_string(),
            "0a1b2c3d".to_string(),
            "boot".to_string(),
            IMG.to_string()
        ]
    );
}

/// HW-30: the boot invocation MUST carry `-s <selected_serial>` — the serial
/// parsed from the devices line, verbatim — so the selection check binds the
/// eventual target instead of being advisory.
#[test]
fn boot_args_include_selected_serial() {
    let runner = FakeRunner::new(vec![ok_devices("SERIAL123\tfastboot\n"), ok_boot()]);
    let out = attempt_run(&runner, Path::new(IMG), None, || true);
    assert_eq!(out, AttemptOutcome::Booted);
    let calls = runner.calls();
    assert_eq!(calls.len(), 2, "devices then boot");
    assert_eq!(calls[0], vec!["devices".to_string()]);
    assert_eq!(
        calls[1],
        vec![
            "-s".to_string(),
            "SERIAL123".to_string(),
            "boot".to_string(),
            IMG.to_string()
        ]
    );
}

/// HW-30: with two enumerated lines of which exactly one is in state
/// `fastboot`, and `--expect-serial` naming that serial, the boot is bound
/// to the expect-serial match — not to the other enumerated device.
#[test]
fn expect_serial_selected_and_bound() {
    let runner = FakeRunner::new(vec![
        ok_devices("OTHERDEV\toffline\nSERIAL999\tfastboot\n"),
        ok_boot(),
    ]);
    let out = attempt_run(&runner, Path::new(IMG), Some("SERIAL999"), || true);
    assert_eq!(out, AttemptOutcome::Booted);
    let calls = runner.calls();
    assert_eq!(calls.len(), 2, "devices then boot");
    assert_eq!(
        calls[1],
        vec![
            "-s".to_string(),
            "SERIAL999".to_string(),
            "boot".to_string(),
            IMG.to_string()
        ]
    );
}

/// The SECOND verify (after devices enumeration, before boot) returns false:
/// TOCTOU mitigation fires -> VerifyAbort, exit 1, boot NEVER called.
#[test]
fn verify_rerun_before_boot_aborts() {
    let runner = FakeRunner::new(vec![ok_devices("0a1b2c3d fastboot\n"), ok_boot()]);
    let n = Cell::new(0u32);
    let out = attempt_run(&runner, Path::new(IMG), None, || {
        n.set(n.get() + 1);
        // first verify passes, second (pre-boot) fails — the file drifted.
        n.get() != 2
    });
    match &out {
        AttemptOutcome::VerifyAbort { reason } => {
            assert!(reason.contains("second"), "reason: {reason}");
        }
        other => panic!("must be VerifyAbort, got {other:?}"),
    }
    assert_eq!(out.exit_code(), 1);
    assert_eq!(n.get(), 2, "both verifies ran before the abort");
    // boot was NEVER called.
    assert_eq!(runner.verbs(), vec!["devices".to_string()]);
}
