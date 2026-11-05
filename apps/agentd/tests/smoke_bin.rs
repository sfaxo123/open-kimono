//! A0: the smoke binary end-to-end — it emits one journal line, and its
//! `--validate` mode is the gate that proves such lines are real records.

use std::io::Write;
use std::process::{Command, Stdio};

const SMOKE: &str = env!("CARGO_BIN_EXE_smoke");

fn run_validate(input: &str) -> (i32, String, String) {
    let mut child = Command::new(SMOKE)
        .arg("--validate")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn smoke --validate");
    child.stdin.take().expect("stdin").write_all(input.as_bytes()).expect("write stdin");
    let out = child.wait_with_output().expect("wait");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

#[test]
fn smoke_emits_one_line_that_validates() {
    let out = Command::new(SMOKE).output().expect("run smoke");
    assert!(out.status.success(), "smoke must exit 0: {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 1, "the smoke binary prints exactly one journal line");

    let (code, out_text, err_text) = run_validate(&stdout);
    assert_eq!(code, 0, "the emitted line must validate: {err_text}");
    assert!(out_text.contains("valid: 1 journal line"), "got: {out_text}");
}

#[test]
fn validate_rejects_a_corrupted_journal_line() {
    let good = Command::new(SMOKE).output().expect("run smoke");
    let good = String::from_utf8_lossy(&good.stdout).to_string();
    let corrupted = good.replace("\"payload\":{", "\"payload\":{\"kind\":\"bogus_vocabulary\",");
    let (code, _, err) = run_validate(&corrupted);
    assert_eq!(code, 1, "a corrupted line must fail validation");
    assert!(err.contains("not a valid journal event"), "got: {err}");
    assert!(err.contains("unknown variant"), "the error must name the schema failure: {err}");
}

#[test]
fn validate_rejects_empty_input_rather_than_passing_vacuously() {
    let (code, _, err) = run_validate("");
    assert_eq!(code, 1, "zero lines is not proof of anything");
    assert!(err.contains("no journal lines"), "got: {err}");
}
