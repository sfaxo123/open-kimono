#!/usr/bin/env python3
"""Offline source-rooted TLMM trace parity; fake MMIO, no VM/device proof.

The immutable before.rs fixture is the actual accepted25 source, not a model.
It avoids depending on an unpushed local kernel commit in fresh git-am clones.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

HERE = Path(__file__).resolve().parent
BEFORE_SHA = "fad54896de565fb8ffb394c686a252d37cf80cd38a9413e3b4d4ac03cdf91142"
MANIFEST = '''[package]
name = "tlmm-step-harness"
version = "0.0.0"
edition = "2021"
publish = false
[lib]
path = "src/lib.rs"
[workspace]
'''


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("kernel", type=Path)
    parser.add_argument("evidence", type=Path, help="new output directory; refuses reuse")
    args = parser.parse_args()
    root, out = args.kernel.resolve(), args.evidence.resolve()
    before = HERE / "before.rs"
    assert hashlib.sha256(before.read_bytes()).hexdigest() == BEFORE_SHA, "baseline fixture changed"
    source = root / "src/arch/aarch64/device/irqchip/tlmm.rs"
    mapping = source.with_name("qcom_irq.rs")
    assert source.is_file() and mapping.is_file(), "wrong kernel root"
    out.mkdir(parents=True, exist_ok=False)
    crate = out / "harness"
    (crate / "src").mkdir(parents=True)
    (crate / "Cargo.toml").write_text(MANIFEST)
    template = (HERE / "lib.rs.in").read_text()
    (crate / "src/lib.rs").write_text(template.replace('"__QCOM_SOURCE__"', json.dumps(str(mapping))))
    for name in ("shim.rs", "tests.rs", "step_tests.rs"):
        shutil.copy2(HERE / name, crate / "src" / name)

    cases = [("original", before, None, False), ("current", source, None, True)]
    cases += [(name, source, name, True) for name in ("omit-clear", "late-clear", "skip-next", "drop-later")]
    for label, current, control, step in cases:
        folder = out / label
        folder.mkdir()
        for name, path in (("old.rs", before), ("new.rs", current)):
            cmd = [sys.executable, str(HERE / "extract.py"), str(path), str(folder / name)]
            if name == "new.rs" and control:
                cmd += ["--control", control]
            subprocess.run(cmd, check=True, capture_output=True, text=True)
        env = {**os.environ, "CARGO_NET_OFFLINE": "true", "TLMM_GEN_DIR": str(folder),
               "TLMM_TRACE_OUT": str(folder / "original-traces.txt"),
               "CARGO_TARGET_DIR": str(folder / "target"),
               "RUSTFLAGS": "--cfg tlmm_step" if step else ""}
        cmd = [str(Path.home() / ".cargo/bin/cargo"), "+nightly-2026-05-24", "test", "--offline",
               "--manifest-path", str(crate / "Cargo.toml"), "--", "--test-threads=1"]
        result = subprocess.run(cmd, cwd=crate, env=env, text=True, stdout=subprocess.PIPE,
                                stderr=subprocess.STDOUT, timeout=120)
        text = result.stdout
        (folder / "result.log").write_text(text + f"\nexit={result.returncode}\n")
        if control:
            test = ("ordering_clear_precedes_each_event" if control in ("omit-clear", "late-clear")
                    else "coverage_resumes_after_each_pin")
            assert result.returncode == 101 and "test result: FAILED" in text, folder
            assert f"tests::{test} ... FAILED" in text, folder
            assert "error[E" not in text, "compiler error is not a behavioral negative control"
        else:
            count = 10 if step else 8
            assert result.returncode == 0 and f"{count} passed; 0 failed" in text, folder
        print(f"PASS {label}: exit={result.returncode}", flush=True)
    print("PASS source trace parity only; fake W1C/volatile hooks, outer core borrow unchanged")


if __name__ == "__main__":
    main()
