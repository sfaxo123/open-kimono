#!/usr/bin/env python3
"""Cross-check: read a Rust-encoded stream with the Python reference.

Usage: check_from_rust.py <rust_out.wyoming> [reference_root]

Compares every event (type, data, payload) against tests/fixtures/golden.json
and additionally checks the Rust byte stream is byte-identical to the Python
golden stream. Exits non-zero on any mismatch.
"""

import json
import sys
import types
from pathlib import Path

HERE = Path(__file__).resolve().parent
REFERENCE_ROOT = Path(
    sys.argv[2] if len(sys.argv) > 2 else "$HOME/Developer/forks/op6-refs/wyoming"
)
rust_path = Path(sys.argv[1] if len(sys.argv) > 1 else HERE / "fixtures/rust_out.wyoming")

sys.path.insert(0, str(REFERENCE_ROOT))
version_stub = types.ModuleType("wyoming.version")
version_stub.__version__ = "1.11.0"
sys.modules["wyoming.version"] = version_stub

from wyoming.event import read_event  # noqa: E402

expected = json.loads((HERE / "fixtures/golden.json").read_text())

failures = 0
with open(rust_path, "rb") as reader:
    for i, want in enumerate(expected):
        event = read_event(reader)
        if event is None:
            print(f"FAIL event {i}: stream ended early")
            failures += 1
            break
        if event.type != want["type"]:
            print(f"FAIL event {i}: type {event.type!r} != {want['type']!r}")
            failures += 1
        if event.data != want["data"]:
            print(f"FAIL event {i} ({want['type']}): data {event.data!r} != {want['data']!r}")
            failures += 1
        want_payload = bytes.fromhex(want["payload_hex"]) if want["payload_hex"] else None
        if event.payload != want_payload:
            print(f"FAIL event {i} ({want['type']}): payload mismatch")
            failures += 1

    extra = read_event(reader)
    if extra is not None:
        print(f"FAIL: extra event at end of stream: {extra.type!r}")
        failures += 1

# Byte-identity: the Rust encoder must produce exactly the Python bytes.
golden_bytes = (HERE / "fixtures/golden.wyoming").read_bytes()
rust_bytes = rust_path.read_bytes()
if golden_bytes == rust_bytes:
    print("byte-identical: Rust encode == Python write_event bytes")
else:
    print(f"FAIL: byte streams differ ({len(golden_bytes)} vs {len(rust_bytes)} bytes)")
    failures += 1

if failures:
    print(f"CROSS-CHECK FAILED: {failures} problem(s)")
    sys.exit(1)
print(f"CROSS-CHECK OK: {len(expected)} events read by Python reference from Rust encoder")
