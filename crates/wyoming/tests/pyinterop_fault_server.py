#!/usr/bin/env python3
"""Fault-injection variants of the pyinterop server (Cursor review W-1/W-3/W-4/W-5).

Speaks Wyoming framing at the byte level (header line + optional data block
+ trailing newline) and echoes frames back verbatim — no reference import,
so these tests exercise the *Rust harness* without the reference checkout.

Modes (argv[1]):

  stall-port     sleep forever before printing PORT
                 (W-5: the harness PORT read must time out, not hang)
  stall-served   serve + echo normally, then sleep forever instead of
                 printing SERVED on EOF
                 (W-1: simulates a close() that never delivers EOF)
  crash-stderr   raise on stderr and exit nonzero before printing PORT
                 (W-4: the harness must surface child stderr, not swallow it)

Every mode takes ``--tag <t>``; the Rust test greps ``pgrep -f`` for the tag
to prove no child process survives the harness (W-3).
"""

import json
import sys
import time


def read_exactly(fobj, n):
    """Read exactly n bytes; None on premature EOF."""
    buf = b""
    while len(buf) < n:
        chunk = fobj.read(n - len(buf))
        if not chunk:
            return None
        buf += chunk
    return buf


def read_frame(rfile):
    """Read one Wyoming frame as raw bytes; None on clean EOF.

    Wire format (matches the Rust codec and the Python reference): header
    line + optional data block (``data_length`` bytes) + optional payload
    block (``payload_length`` bytes); the blocks have no trailing newline.
    """
    line = rfile.readline()
    if not line:
        return None
    frame = line
    try:
        header = json.loads(line)
    except ValueError:
        return frame
    for field in ("data_length", "payload_length"):
        length = header.get(field, 0)
        if length:
            block = read_exactly(rfile, length)
            if block is None:
                return None
            frame += block
    return frame


def main() -> int:
    mode = sys.argv[1]
    # --tag is echoed in argv so pgrep can find exactly this child.
    if mode == "crash-stderr":
        raise RuntimeError("PYINTEROP-CRASH-MARKER: simulated python startup failure")

    import socket

    server = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    server.bind(("127.0.0.1", 0))
    server.listen(1)

    if mode == "stall-port":
        time.sleep(300)  # never reaches the PORT line

    print(f"PORT {server.getsockname()[1]}", flush=True)

    conn, _peer = server.accept()
    reader = conn.makefile("rb")
    writer = conn.makefile("wb")

    count = 0
    while True:
        frame = read_frame(reader)
        if frame is None:
            break
        count += 1
        writer.write(frame)
        writer.flush()

    if mode == "stall-served":
        time.sleep(300)  # never prints SERVED, like a close() that lost EOF

    print(f"SERVED {count}", flush=True)
    conn.close()
    server.close()
    return 0


if __name__ == "__main__":
    sys.exit(main())
