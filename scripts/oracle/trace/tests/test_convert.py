import sys
import subprocess
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from convert import convert


class ConversionTests(unittest.TestCase):
    def test_rwmmio_mapping_order_width_and_delay(self):
        raw = [
            'task-1 [000] 1.000000: rwmmio_read: caller=geni+0x1 width=4 val=0x0 addr=0xffff1004\n',
            'task-1 [000] 1.000002: rwmmio_post_read: caller=geni+0x1 width=4 val=0xab addr=0xffff1004\n',
            'task-1 [000] 1.000009: rwmmio_write: caller=geni+0x2 width=4 val=0x12345678 addr=0xffff1008\n',
            'task-1 [000] 1.000010: rwmmio_post_write: caller=geni+0x2 width=4 val=0x12345678 addr=0xffff1008\n',
        ]
        result = list(convert(raw, 0xffff1000, 0xa84000, 0x1000))
        self.assertEqual(result, [
            {'op': 'r', 'width': 32, 'addr': '0xa84004', 'value': '0xab', 'delay_us': 0},
            {'op': 'w', 'width': 32, 'addr': '0xa84008', 'value': '0x12345678', 'delay_us': 7},
        ])

    def test_caller_filter_and_window(self):
        raw = ['x 1.0: rwmmio_write: caller=other width=1 val=0x1ff addr=0x1000',
               'x 1.0: rwmmio_write: caller=geni width=1 val=0x1ff addr=0x1001',
               'x 1.0: rwmmio_write: caller=geni width=1 val=0x1 addr=0x2000']
        self.assertEqual(list(convert(raw, 0x1000, 0x4000, 0x100, 'geni')),
                         [{'op': 'w', 'width': 8, 'addr': '0x4001', 'value': '0xff', 'delay_us': 0}])

    def test_malformed_event_fails(self):
        with self.assertRaisesRegex(ValueError, 'incomplete'):
            list(convert(['x: rwmmio_write: width=4 val=0x1'], 0, 0, 0x100))

    def test_cli_dry_run_never_writes_output(self):
        with tempfile.TemporaryDirectory() as directory:
            raw = Path(directory) / 'raw.txt'
            output = Path(directory) / 'replay.jsonl'
            raw.write_text('x 1.000000: rwmmio_write: caller=geni width=4 val=0x2 addr=0x1000\n')
            command = [sys.executable, str(Path(__file__).resolve().parents[1] / 'convert.py'),
                       str(raw), str(output), '--virtual-base', '0x1000',
                       '--physical-base', '0xa84000', '--size', '0x100', '--dry-run']
            subprocess.run(command, check=True, capture_output=True, text=True)
            self.assertFalse(output.exists())


if __name__ == '__main__':
    unittest.main()
