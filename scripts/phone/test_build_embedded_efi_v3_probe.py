#!/usr/bin/env python3
"""Stub-fdtget regression tests for build-embedded-efi-v3.py.

Pins the fail-closed DTB /config probe (Wonder diff 278daaf):
- FDT_ERR_NOTFOUND (no /config node)  -> measured False
- bootdelay present in /config        -> True
- /config present, no bootdelay       -> False
- any other fdtget failure (BADMAGIC) -> SystemExit (abort, no False)
Also pins DTB_SHA256_PIN against the real DTB input when present.
Run: python3 scripts/phone/test_build_embedded_efi_v3_probe.py
"""
import hashlib
import importlib.util
import os
from pathlib import Path
import stat
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location(
    "build_embedded_efi_v3", HERE / "build-embedded-efi-v3.py")
builder = importlib.util.module_from_spec(spec)
spec.loader.exec_module(builder)


def stub_fdtget(tmp: Path, stdout: str, stderr: str, rc: int) -> str:
    stub = tmp / "fdtget"
    stub.write_text(
        "#!/bin/sh\n"
        f"printf '%s' '{stdout}'\n"
        f"printf '%s' '{stderr}' >&2\n"
        f"exit {rc}\n"
    )
    stub.chmod(stub.stat().st_mode | stat.S_IEXEC)
    return str(stub)


class ProbeTests(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp())
        self.dtb = self.tmp / "x.dtb"
        self.dtb.write_bytes(b"\xd0\x0d\xfe\xed fake")

    def probe(self, stdout, stderr, rc):
        return builder.dtb_config_bootdelay_override(
            stub_fdtget(self.tmp, stdout, stderr, rc), self.dtb, "test")

    def test_notfound_is_measured_false(self):
        self.assertFalse(self.probe("", "Error at '/config': FDT_ERR_NOTFOUND\n", 1))

    def test_bootdelay_present_is_true(self):
        self.assertTrue(self.probe("bootdelay\nother\n", "", 0))

    def test_config_without_bootdelay_is_false(self):
        self.assertFalse(self.probe("other\n", "", 0))

    def test_badmagic_aborts(self):
        with self.assertRaises(SystemExit):
            self.probe("", "FDT_ERR_BADMAGIC\n", 1)

    def test_generic_failure_aborts(self):
        with self.assertRaises(SystemExit):
            self.probe("", "some other tool error\n", 1)


class DtbPinTests(unittest.TestCase):
    def test_dtb_pin_matches_real_input_when_present(self):
        if not builder.DTB.exists():
            self.skipTest("pinned DTB input not present on this host")
        digest = hashlib.sha256(builder.DTB.read_bytes()).hexdigest()
        self.assertEqual(digest, builder.DTB_SHA256_PIN)

    def test_cmd_c_hash_matches_pin(self):
        # Wonder diff 10f4fe0: well-formedness alone does not pin the
        # generated glue. Runtime main() enforces this same check;
        # the test must assert it too, not overstate pinning.
        digest = hashlib.sha256(builder.CMD_C.encode()).hexdigest()
        self.assertEqual(digest, builder.CMD_C_SHA256_PIN)

    def test_env_hash_matches_pin(self):
        digest = hashlib.sha256(builder.ENV.encode()).hexdigest()
        self.assertEqual(digest, builder.ENV_SHA256_PIN)

    def test_overlay_pins_well_formed(self):
        for pin in (builder.CMD_C_SHA256_PIN, builder.ENV_SHA256_PIN,
                    builder.DTB_SHA256_PIN):
            self.assertEqual(len(pin), 64)
            int(pin, 16)


if __name__ == "__main__":
    unittest.main(verbosity=2)
