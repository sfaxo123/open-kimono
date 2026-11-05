#!/usr/bin/env python3
"""Loopback Wyoming server speaking the upstream Python reference.

Run with PYTHONPATH pointing at the pinned reference checkout
($HOME/Developer/forks/op6-refs/wyoming @ 08a0c25, wyoming 1.11.0).

Binds 127.0.0.1:0, prints "PORT <n>", accepts one connection, then for
every event read with the reference ``wyoming.event.read_event``:

  * ``ping`` -> answers with the reference ``wyoming.ping.Pong`` (text
    copied), exactly like the reference ``PingHandler``
  * anything else -> echoes the event back with ``wyoming.event.write_event``

On clean EOF prints "SERVED <n>" and exits 0. Driven by
``tests/pyinterop.rs`` (Rust client) as the Python-interoperability gate.
"""

import socket
import sys

import wyoming
from wyoming.event import read_event, write_event
from wyoming.ping import Pong


def main() -> int:
    server = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    # Bound the child's own lifetime even if the Rust harness dies without
    # killing us (Cursor review W-3 belt-and-braces; the RAII guard is the
    # primary fix).
    server.settimeout(120)
    server.bind(("127.0.0.1", 0))
    server.listen(1)
    # Identity first, then readiness: the Rust harness asserts the imported
    # module really comes from the reference checkout (Cursor review W-6)
    # instead of trusting the pinned version string.
    print(f"REF {wyoming.__file__} {wyoming.__version__}", flush=True)
    print(f"PORT {server.getsockname()[1]}", flush=True)

    conn, _peer = server.accept()
    conn.settimeout(120)
    reader = conn.makefile("rb")
    writer = conn.makefile("wb")

    count = 0
    while True:
        event = read_event(reader)
        if event is None:
            break
        count += 1
        if event.type == "ping":
            write_event(Pong(text=event.data.get("text")).event(), writer)
        else:
            write_event(event, writer)

    print(f"SERVED {count}", flush=True)
    conn.close()
    server.close()
    return 0


if __name__ == "__main__":
    sys.exit(main())
