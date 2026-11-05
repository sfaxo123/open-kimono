"""Packet byte/context gates, using disposable files rather than a Git repository."""
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
import unittest

TOOL = Path(__file__).with_name('verify_source_packet.py')

class PacketGate(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.handoff = self.root/'HANDOFF.md'
        self.handoff.write_text('Current project handoff\n')
        self.source = self.root/'source.rs'
        self.source.write_text('fn target() { let x = 1; }\n')
        digest = hashlib.sha256(self.handoff.read_bytes()).hexdigest()
        self.context = f'Canonical handoff: {self.handoff}\nCanonical handoff SHA256: {digest}\n'
        self.code = '```rust\nfn target() { let x = 1; }\n```\n'

    def verify(self, text, strict=True):
        packet = self.root/'packet.md'; packet.write_text(text)
        cmd = ['python3',str(TOOL)]
        if strict: cmd += ['--handoff',str(self.handoff)]
        run = subprocess.run(cmd+[str(self.root),str(packet),'source.rs'],capture_output=True,text=True)
        return run.returncode,json.loads(run.stdout)

    def test_matching_context_and_code(self):
        rc,result = self.verify(self.context+self.code)
        self.assertEqual(rc,0); self.assertTrue(result['context']['pass'])

    def test_missing_wrong_and_duplicate_path(self):
        for context in ['', self.context.replace(str(self.handoff),str(self.root/'old'/'HANDOFF.md')), self.context+self.context]:
            with self.subTest(context=context): self.assertEqual(self.verify(context+self.code)[0],1)

    def test_changed_handoff_rejected(self):
        self.handoff.write_text('Changed current plan\n')
        rc,result = self.verify(self.context+self.code)
        self.assertEqual(rc,1); self.assertFalse(result['context']['pass'])

    def test_bad_code_still_rejected(self):
        self.assertEqual(self.verify(self.context+self.code.replace('x = 1','x = 2'))[0],1)

    def test_legacy_mode_only_checks_code(self):
        rc,result = self.verify(self.code,strict=False)
        self.assertEqual(rc,0); self.assertIsNone(result['context'])

if __name__ == '__main__': unittest.main()
