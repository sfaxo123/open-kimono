#!/usr/bin/env python3
"""Exercise the production embedded serial matcher against actual byte offsets."""
from pathlib import Path
import subprocess
import tempfile

source = Path(__file__).with_name('e1-wy-guest-harness.sh').read_text()
matcher = source.split("<<'PWLINE'\n", 1)[1].split('\nPWLINE', 1)[0]
for prefix in (b'', b'\x1b[32m', b'\x1b]0;console\x07\x1b[32m'):
    raw = prefix + b'root:~# \x1b[0mecho WYARM\r\nWYARM\r\nroot:~# '
    expected = raw.index(b'\r\nroot:~# ') + 2
    with tempfile.NamedTemporaryFile() as serial:
        serial.write(raw)
        serial.flush()
        result = subprocess.run(
            ['python3', '-', serial.name, '0', str(len(raw)), r'^WYARM(?=[\r\n])'],
            input=matcher, text=True, capture_output=True, check=True)
    assert int(result.stdout) == expected, (result.stdout, expected)
    assert raw[int(result.stdout):] == b'root:~# '
print('PASS: plain, CSI and OSC serial offsets retain the fresh prompt')

# A fully contained retransmit must match its original position, not the suffix.
import runpy
pcap = runpy.run_path(str(Path(__file__).with_name('e1-wy-guest-harness-pcap.py')))
reassemble = pcap['reassemble']
assert reassemble([(100, b'abcdef'), (101, b'bc')]) == b'abcdef'
assert reassemble([(100, b'abcdef'), (101, b'XX')]) is None
assert reassemble([(100, b'abc'), (102, b'cdef')]) == b'abcdef'
print('PASS: contained/extended retransmits accepted; conflicting overlap refused')
