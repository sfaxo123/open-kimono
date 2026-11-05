use serde_json::Value;
use std::{fs, path::PathBuf, process::Command};

fn tmp() -> PathBuf {
    let p = std::env::temp_dir().join(format!(
        "redox-rust-parity-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&p).unwrap();
    p
}

#[test]
fn matches_python_oracle_jsonl_and_dry_run_behavior() {
    let dir = tmp();
    let raw = dir.join("raw.txt");
    let rs_out = dir.join("rust.jsonl");
    let py_out = dir.join("python.jsonl");
    fs::write(&raw, "task-1 [000] 1.000000: rwmmio_read: caller=geni+0x1 width=4 val=0x0 addr=0xffff1004\ntask-1 [000] 1.000002: rwmmio_post_read: caller=geni+0x1 width=4 val=0xab addr=0xffff1004\ntask-1 [000] 1.000009: rwmmio_write: caller=geni+0x2 width=4 val=0x12345678 addr=0xffff1008\ntask-1 [000] 1.000010: rwmmio_post_write: caller=geni+0x2 width=4 val=0x12345678 addr=0xffff1008\n").unwrap();
    let common = [
        "--virtual-base",
        "0xffff1000",
        "--physical-base",
        "0xa84000",
        "--size",
        "0x1000",
    ];
    let mut rust = Command::new(env!("CARGO_BIN_EXE_redox-trace-convert"));
    rust.arg(&raw).arg(&rs_out);
    for pair in common.chunks_exact(2) {
        rust.arg(pair[0]).arg(pair[1]);
    }
    assert!(rust.status().unwrap().success());
    let python =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scripts/oracle/trace/convert.py");
    let mut py = Command::new("python3");
    py.arg(python).arg(&raw).arg(&py_out);
    for pair in common.chunks_exact(2) {
        py.arg(pair[0]).arg(pair[1]);
    }
    assert!(py.status().unwrap().success());
    let rust_rows: Vec<Value> = fs::read_to_string(&rs_out)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let py_rows: Vec<Value> = fs::read_to_string(&py_out)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(rust_rows, py_rows);

    let dry = dir.join("dry.jsonl");
    let output = Command::new(env!("CARGO_BIN_EXE_redox-trace-convert"))
        .arg(&raw)
        .arg(&dry)
        .args(common)
        .arg("--dry-run")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(!dry.exists());
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn rejects_malformed_trace_like_python_oracle() {
    let dir = tmp();
    let raw = dir.join("bad.txt");
    let out = dir.join("out.jsonl");
    fs::write(&raw, "x: rwmmio_write: width=4 val=0x1\n").unwrap();
    let args = [&raw, &out];
    let rust = Command::new(env!("CARGO_BIN_EXE_redox-trace-convert"))
        .args(args)
        .args([
            "--virtual-base",
            "0",
            "--physical-base",
            "0",
            "--size",
            "0x100",
        ])
        .output()
        .unwrap();
    assert!(!rust.status.success());
    assert!(String::from_utf8_lossy(&rust.stderr).contains("incomplete"));
    let python =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scripts/oracle/trace/convert.py");
    let py = Command::new("python3")
        .arg(python)
        .arg(&raw)
        .arg(&out)
        .args([
            "--virtual-base",
            "0",
            "--physical-base",
            "0",
            "--size",
            "0x100",
        ])
        .output()
        .unwrap();
    assert!(!py.status.success());
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn caller_filter_and_unmapped_events_match_python_oracle() {
    let dir = tmp();
    let raw = dir.join("raw.txt");
    let rs_out = dir.join("rust.jsonl");
    let py_out = dir.join("python.jsonl");
    fs::write(&raw, "x 1.000001: rwmmio_write: caller=other width=1 val=0x1ff addr=0x1000\nx 1.000009: rwmmio_write: caller=geni width=1 val=0x1ff addr=0x1001\nx 1.000011: rwmmio_write: caller=geni width=8 val=0xffffffffffffffff addr=0x10fc\nx 1.000012: rwmmio_write: caller=geni width=2 val=0x12345 addr=0x2000\n").unwrap();
    let common = [
        "--virtual-base",
        "0x1000",
        "--physical-base",
        "0x4000",
        "--size",
        "0x100",
        "--caller",
        "geni",
    ];
    let rust = Command::new(env!("CARGO_BIN_EXE_redox-trace-convert"))
        .arg(&raw)
        .arg(&rs_out)
        .args(common)
        .output()
        .unwrap();
    let python =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scripts/oracle/trace/convert.py");
    let py = Command::new("python3")
        .arg(python)
        .arg(&raw)
        .arg(&py_out)
        .args(common)
        .output()
        .unwrap();
    assert_eq!(rust.status.success(), py.status.success());
    if rust.status.success() {
        assert_eq!(fs::read(&rs_out).unwrap(), fs::read(&py_out).unwrap());
    } else {
        assert!(String::from_utf8_lossy(&rust.stderr).contains("no mapped"));
    }
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn parity_regression_negative_values() {
    let dir = tmp();
    let raw = dir.join("negative.txt");
    let rust_out = dir.join("rust.jsonl");
    let python_out = dir.join("python.jsonl");
    fs::write(
        &raw,
        "x 1.000001: rwmmio_write: caller=geni width=1 val=-1 addr=0x1000\n",
    )
    .unwrap();
    let common = [
        "--virtual-base",
        "0x1000",
        "--physical-base",
        "0x4000",
        "--size",
        "0x100",
    ];
    let rust = Command::new(env!("CARGO_BIN_EXE_redox-trace-convert"))
        .arg(&raw)
        .arg(&rust_out)
        .args(common)
        .output()
        .unwrap();
    let python =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scripts/oracle/trace/convert.py");
    let py = Command::new("python3")
        .arg(python)
        .arg(&raw)
        .arg(&python_out)
        .args(common)
        .output()
        .unwrap();
    assert_eq!(rust.status.success(), py.status.success());
    assert!(py.status.success());
    assert_eq!(fs::read(rust_out).unwrap(), fs::read(python_out).unwrap());
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn arbitrary_signed_ascii_values_and_malformed_values_match_oracle() {
    let dir = tmp();
    let raw = dir.join("values.txt");
    let rust_out = dir.join("rust.jsonl");
    let python_out = dir.join("python.jsonl");
    let oracle =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scripts/oracle/trace/convert.py");
    let args = [
        "--virtual-base",
        "0x1000",
        "--physical-base",
        "0x4000",
        "--size",
        "0x100",
    ];
    for width in [1, 2, 4, 8] {
        for value in [
            "-1",
            "-0x1f",
            "+0b101",
            "-0o777",
            "0x_1",
            "1_0",
            "-00",
            "0xffffffffffffffffffffffffffffffffffff",
            "-0xffffffffffffffffffffffffffffffffffff",
            "1111111111111111111111111111111111111111111111111111111111111111111111",
            "010",
            "0_1",
            "1__0",
            "_1",
            "1_",
            "0x__1",
            "0x-1",
            "--1",
            "0x",
            "0b2",
        ] {
            let input = format!(
                "x 1.000001: rwmmio_write: caller=geni width={width} val={value} addr=0x1000\n"
            );
            fs::write(&raw, &input).unwrap();
            // Sentinel contents detect writes on rejected input, not just exit parity.
            fs::write(&rust_out, "sentinel\n").unwrap();
            fs::write(&python_out, "sentinel\n").unwrap();
            let rust = Command::new(env!("CARGO_BIN_EXE_redox-trace-convert"))
                .arg(&raw)
                .arg(&rust_out)
                .args(args)
                .output()
                .unwrap();
            let python = Command::new("python3")
                .arg(&oracle)
                .arg(&raw)
                .arg(&python_out)
                .args(args)
                .output()
                .unwrap();
            assert_eq!(
                rust.status.success(),
                python.status.success(),
                "{width}/{value}"
            );
            assert_eq!(
                fs::read(&rust_out).unwrap(),
                fs::read(&python_out).unwrap(),
                "{width}/{value}"
            );
            assert_eq!(fs::read_to_string(&raw).unwrap(), input);
        }
    }
    fs::remove_dir_all(dir).unwrap();
}
