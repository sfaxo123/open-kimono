//! Acceptance for the still-open base-0 CLI numeric grammar contract.
use std::{fs, path::PathBuf, process::Command};

#[test]
fn cli_underscored_mapping_size_matches_frozen_oracle() {
    let dir = std::env::temp_dir().join(format!(
        "redox-numeric-parity-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    let raw = dir.join("raw.txt");
    fs::write(
        &raw,
        "x 1.000001: rwmmio_write: width=4 val=0xab addr=0x1004\n",
    )
    .unwrap();
    let rust_out = dir.join("rust.jsonl");
    let python_out = dir.join("python.jsonl");
    // Differential grammar cases: (literal, expected value or None = invalid).
    // Python int(s, 0) and Rust parse_int must agree on success/output.
    let cases: [(&str, Option<u32>); 13] = [
        ("0x10_00", Some(4096)),
        ("+0x1000", Some(4096)),
        ("0x_1000", Some(4096)),
        ("4_096", Some(4096)),
        ("00", Some(0)),
        ("00_0", Some(0)),
        ("0_0", Some(0)),
        ("0x__10", None),
        ("1__0", None),
        ("010", None),
        ("0x10_", None),
        ("_10", None),
        ("0x_", None),
    ];
    for (literal, expected) in cases {
        let oracle =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scripts/oracle/trace/convert.py");
        // Seed sentinel output so failure paths can prove pre-existing output is preserved.
        let sentinel = format!("sentinel-{literal}\n");
        fs::write(&python_out, &sentinel).unwrap();
        fs::write(&rust_out, &sentinel).unwrap();
        let args = [
            "--virtual-base",
            "0x1000",
            "--physical-base",
            literal,
            "--size",
            "0x10_00",
        ];
        let python = Command::new("python3")
            .arg(&oracle)
            .arg(&raw)
            .arg(&python_out)
            .args(args)
            .output()
            .unwrap();
        let rust = Command::new(env!("CARGO_BIN_EXE_redox-trace-convert"))
            .arg(&raw)
            .arg(&rust_out)
            .args(args)
            .output()
            .unwrap();
        match expected {
            Some(phys) => {
                assert!(python.status.success(), "oracle must accept {literal}");
                assert!(
                    rust.status.success(),
                    "Rust rejected valid base-0 CLI integer {literal}: {}",
                    String::from_utf8_lossy(&rust.stderr)
                );
                let py = fs::read_to_string(&python_out).unwrap();
                let rs = fs::read_to_string(&rust_out).unwrap();
                assert_eq!(rs, py, "outputs differ for {literal}");
                // addr = physicalBase + (0x1004 - 0x1000) = phys + 4.
                assert!(
                    py.contains(&format!("\"addr\":\"0x{:x}\"", phys + 4)),
                    "expected mapped trace event for {literal}"
                );
            }
            None => {
                assert!(!python.status.success(), "oracle must reject {literal}");
                assert!(
                    !rust.status.success(),
                    "Rust accepted invalid base-0 CLI integer {literal}"
                );
                let py_err = String::from_utf8_lossy(&python.stderr).to_string();
                let rs_err = String::from_utf8_lossy(&rust.stderr).to_string();
                assert!(
                    py_err.contains(literal) && rs_err.contains(literal),
                    "both must report the rejected literal {literal}: py={py_err} rs={rs_err}"
                );
                // Both tools must leave pre-existing output untouched on failure.
                assert_eq!(
                    fs::read_to_string(&python_out).unwrap(),
                    sentinel,
                    "oracle clobbered existing output for {literal}"
                );
                assert_eq!(
                    fs::read_to_string(&rust_out).unwrap(),
                    sentinel,
                    "Rust clobbered existing output for {literal}"
                );
            }
        }
    }
    let args = [
        "--virtual-base",
        "0x1000",
        "--physical-base",
        "0x4000",
        "--size",
        "0x10_00",
    ];
    let oracle =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scripts/oracle/trace/convert.py");
    let python = Command::new("python3")
        .arg(oracle)
        .arg(&raw)
        .arg(&python_out)
        .args(args)
        .output()
        .unwrap();
    let rust = Command::new(env!("CARGO_BIN_EXE_redox-trace-convert"))
        .arg(&raw)
        .arg(&rust_out)
        .args(args)
        .output()
        .unwrap();
    let py_bytes = fs::read(&python_out).ok();
    let rs_bytes = fs::read(&rust_out).ok();
    fs::remove_dir_all(dir).unwrap();
    assert!(python.status.success(), "frozen oracle must accept fixture");
    assert!(
        rust.status.success(),
        "Rust rejected valid base-0 CLI integer: {}",
        String::from_utf8_lossy(&rust.stderr)
    );
    assert_eq!(rs_bytes, py_bytes);
}
