use std::{fs, path::PathBuf, process::Command};

fn temp_dir() -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "redox-path-parity-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&path).unwrap();
    path
}

fn common() -> [&'static str; 6] {
    [
        "--virtual-base",
        "0x1000",
        "--physical-base",
        "0x4000",
        "--size",
        "0x100",
    ]
}

fn python_oracle() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scripts/oracle/trace/convert.py")
}

#[test]
fn bare_relative_output_matches_python_oracle() {
    let dir = temp_dir();
    fs::write(
        dir.join("raw.txt"),
        "x 1.000001: rwmmio_write: caller=geni width=4 val=0x1234 addr=0x1000\n",
    )
    .unwrap();
    let rust = Command::new(env!("CARGO_BIN_EXE_redox-trace-convert"))
        .current_dir(&dir)
        .args(["raw.txt", "rust.jsonl"])
        .args(common())
        .output()
        .unwrap();
    let python = Command::new("python3")
        .current_dir(&dir)
        .arg(python_oracle())
        .args(["raw.txt", "python.jsonl"])
        .args(common())
        .output()
        .unwrap();
    assert!(
        python.status.success(),
        "{}",
        String::from_utf8_lossy(&python.stderr)
    );
    assert!(
        rust.status.success(),
        "{}",
        String::from_utf8_lossy(&rust.stderr)
    );
    assert_eq!(
        fs::read(dir.join("rust.jsonl")).unwrap(),
        fs::read(dir.join("python.jsonl")).unwrap()
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn refuses_same_input_output_alias_without_changing_input() {
    let dir = temp_dir();
    fs::create_dir(dir.join("sub")).unwrap();
    let original = b"x 1.000001: rwmmio_write: caller=geni width=4 val=0x1234 addr=0x1000\n";
    fs::write(dir.join("raw.txt"), original).unwrap();
    for alias in ["./raw.txt", "sub/../raw.txt"] {
        let output = Command::new(env!("CARGO_BIN_EXE_redox-trace-convert"))
            .current_dir(&dir)
            .args(["raw.txt", alias])
            .args(common())
            .output()
            .unwrap();
        assert!(
            !output.status.success(),
            "alias unexpectedly accepted: {alias}"
        );
        assert!(String::from_utf8_lossy(&output.stderr).contains("paths must differ"));
        assert_eq!(fs::read(dir.join("raw.txt")).unwrap(), original);
    }
    let _ = fs::remove_dir_all(dir);
}
