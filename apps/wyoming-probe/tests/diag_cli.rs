//! CLI-level regression for the diagnostic: what the operator actually sees and
//! the exit code the harness acts on. Run against a listener that accepts (via
//! the backlog) and never answers — the silent-peer shape from the guest.

use std::net::TcpListener;
use std::process::Command;

fn diag_output(timeout_secs: &str) -> (Option<i32>, String) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    let out = Command::new(env!("CARGO_BIN_EXE_wyoming-probe"))
        .args(["diag", &addr.to_string(), timeout_secs])
        .output()
        .expect("run wyoming-probe diag");
    drop(listener);
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

#[test]
fn a_bounded_silent_timeout_exits_0_and_is_not_called_a_hang() {
    let (code, stdout) = diag_output("1");
    assert_eq!(
        code,
        Some(0),
        "a correct bounded timeout must not exit as a failure:\n{stdout}"
    );
    assert!(
        stdout.contains("BOUNDED SILENT TIMEOUT OK"),
        "summary must name the bounded timeout as the control trail:\n{stdout}"
    );
    assert!(
        !stdout.contains("NEVER RETURNED") && !stdout.contains("PATHOLOGY"),
        "false pathology claim for a read that returned WouldBlock:\n{stdout}"
    );
    assert!(stdout.contains("deadline_hit"), "{stdout}");
}

#[test]
fn every_phase_gets_an_untimed_raw_line_before_its_stamped_line() {
    // Clock-safe emission: if the clock read hangs, the raw line is already on
    // the wire, so the log cannot blame the preceding primitive.
    let (code, stdout) = diag_output("1");
    assert_eq!(code, Some(0), "{stdout}");
    let raw = stdout.matches("DIAG-R ").count();
    let stamped = stdout.matches("\nDIAG ").count();
    assert!(
        raw >= 15,
        "expected a raw pre-stamp per phase, got {raw}:\n{stdout}"
    );
    assert!(
        raw >= stamped,
        "raw lines ({raw}) must cover every stamped line ({stamped}):\n{stdout}"
    );
    assert!(
        stdout.contains("DIAG-R 0 clock_init_before")
            && stdout.contains("DIAG-R 0 clock_init_after"),
        "the initial Instant construction is not bracketed:\n{stdout}"
    );
}

#[test]
fn positional_probe_cli_is_still_accepted_unchanged() {
    // Backward compatibility of the legacy guest invocation: usage paths only,
    // so no Python fixture is needed here.
    let out = Command::new(env!("CARGO_BIN_EXE_wyoming-probe"))
        .output()
        .expect("run wyoming-probe");
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert_eq!(out.status.code(), Some(2), "{stderr}");
    assert!(
        stderr.contains("usage: wyoming-probe <numeric-ip:port> [nonce] [deadline_secs]"),
        "positional usage string changed: {stderr}"
    );

    let out = Command::new(env!("CARGO_BIN_EXE_wyoming-probe"))
        .args(["not-an-address"])
        .output()
        .expect("run wyoming-probe");
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert_eq!(out.status.code(), Some(2), "{stderr}");
    assert!(
        stderr.contains("numeric ip:port required"),
        "positional address guard changed: {stderr}"
    );
}
