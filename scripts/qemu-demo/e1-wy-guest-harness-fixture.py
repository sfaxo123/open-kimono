#!/usr/bin/env python3
"""E1-WY-GUEST-HARNESS host fixture: TWO actual sockets, one JSON receipt.

Socket A (answering): the Wyoming fixture. Reuses the PINNED Python reference
API verbatim — ``wyoming.event.read_event`` / ``write_event`` and
``wyoming.ping.Pong`` — exactly as crates/wyoming/tests/pyinterop_server.py
starts it (PYTHONPATH = reference checkout : crates/wyoming/tests/pyref_meta).
ping -> Pong with the text copied back; every other event echoed.

Socket B (silent peer): accepts, records a monotonic accept stamp, then sends
NOTHING until stopped. This is the read-timeout leg's peer.

Both sockets bind 127.0.0.1:0 (OS-assigned ports). Readiness is a single JSON
line on stdout AFTER both are listening — never a fixed sleep. On stop
(SIGTERM/SIGINT or --deadline) a structured JSON receipt is written: actual
Describe/Info/Ping/Pong types, the nonce seen in ping text, and the silent
accept stamps. Stdlib + the pinned reference only; no downloads, no new deps.

Exit: 0 served-and-receipted | 2 bad args / reference import refused.
"""
import argparse
import json
import os
import signal
import socket
import sys
import threading
import time

try:
    import wyoming
    from wyoming.event import read_event, write_event
    from wyoming.ping import Pong
except ImportError as e:  # fail closed: the pinned reference must be importable
    print("REF_IMPORT_FAIL %s" % e, file=sys.stderr)
    sys.exit(2)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--receipt", required=True)
    ap.add_argument("--deadline", type=float, required=True)  # epoch seconds
    ap.add_argument("--bind", default="127.0.0.1")
    args = ap.parse_args()

    stop = threading.Event()
    state = {
        "ref_path": getattr(wyoming, "__file__", ""),
        "wy_version": getattr(wyoming, "__version__", ""),
        "answer_events": [],          # {type, at_wall, at_mono, nonce?}
        "answer_counts": {},          # event type -> count
        "answer_writes": 0,           # reference write_event calls that succeeded
        "answer_eof": False,
        "silent_accepts": [],         # {at_wall, at_mono}
        "silent_events": [],          # types READ (never answered) at the peer
        "silent_writes": 0,           # must stay 0: the peer sends NOTHING
        "nonce_seen": None,
        "stopped_by": "run",
    }
    lock = threading.Lock()

    def sig(_s, _f):
        state["stopped_by"] = "signal"
        stop.set()
    signal.signal(signal.SIGTERM, sig)
    signal.signal(signal.SIGINT, sig)

    def bounded():
        t = args.deadline - time.time()
        return max(0.05, min(t, 120.0))

    def answer_loop(srv):
        srv.settimeout(bounded())
        while not stop.is_set():
            try:
                conn, _ = srv.accept()
            except (socket.timeout, OSError):
                if time.time() >= args.deadline:
                    state["stopped_by"] = "deadline"
                    return
                continue
            conn.settimeout(bounded())
            reader, writer = conn.makefile("rb"), conn.makefile("wb")
            while not stop.is_set():
                try:
                    ev = read_event(reader)
                except (socket.timeout, OSError):
                    break
                if ev is None:
                    with lock:
                        state["answer_eof"] = True
                    break
                rec = {"type": ev.type, "at_wall": time.time(),
                       "at_mono": time.monotonic()}
                # Reference write path, exactly as pyinterop_server.py:
                # ping -> Pong(text copied); anything else -> echo the event.
                if ev.type == "ping":
                    text = ev.data.get("text")
                    rec["nonce"] = text
                    with lock:
                        state["nonce_seen"] = text
                    out = Pong(text=text).event()
                else:
                    out = ev
                try:
                    write_event(out, writer)
                except OSError:
                    break
                with lock:
                    state["answer_writes"] += 1
                    state["answer_events"].append(rec)
                    k = ev.type
                    state["answer_counts"][k] = state["answer_counts"].get(k, 0) + 1
            conn.close()

    def silent_loop(srv):
        srv.settimeout(bounded())
        conns = []
        while not stop.is_set():
            try:
                conn, _ = srv.accept()
            except (socket.timeout, OSError):
                if time.time() >= args.deadline:
                    state["stopped_by"] = "deadline"
                    return
                continue
            # Record the accept with a monotonic stamp, READ the frames the
            # guest actually sent (reference read_event — proof the guest
            # really connected and sent its probe), then send NOTHING.
            with lock:
                state["silent_accepts"].append(
                    {"at_wall": time.time(), "at_mono": time.monotonic()})
            conn.settimeout(bounded())

            def silent_read(c=conn):
                reader = c.makefile("rb")
                while not stop.is_set():
                    try:
                        ev = read_event(reader)
                    except (socket.timeout, OSError):
                        return
                    if ev is None:
                        return
                    with lock:
                        state["silent_events"].append(
                            {"type": ev.type, "at_wall": time.time(),
                             "at_mono": time.monotonic()})
            threading.Thread(target=silent_read, daemon=True).start()
            conns.append(conn)
        for c in conns:
            try:
                c.close()
            except OSError:
                pass

    def bind0():
        s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        s.bind((args.bind, 0))
        s.listen(2)
        return s

    try:
        srv_a, srv_b = bind0(), bind0()
    except OSError as e:
        print("SOCKET_BIND_FAIL %s" % e, file=sys.stderr)
        return 2

    ports = {"answer_port": srv_a.getsockname()[1],
             "silent_port": srv_b.getsockname()[1]}
    state.update(ports)
    # Readiness AFTER both sockets listen — the harness waits for this line.
    print(json.dumps({"ready": True, **ports, "ref": state["ref_path"],
                      "wy_version": state["wy_version"]}), flush=True)

    ta = threading.Thread(target=answer_loop, args=(srv_a,), daemon=True)
    tb = threading.Thread(target=silent_loop, args=(srv_b,), daemon=True)
    ta.start(); tb.start()
    while not stop.is_set() and time.time() < args.deadline:
        stop.wait(min(0.25, max(0.05, args.deadline - time.time())))
    if time.time() >= args.deadline and state["stopped_by"] == "run":
        state["stopped_by"] = "deadline"
    stop.set()
    for s in (srv_a, srv_b):
        try:
            s.close()
        except OSError:
            pass
    state["stopped_at_wall"] = time.time()
    tmp = args.receipt + ".tmp"
    with open(tmp, "w") as f:
        json.dump(state, f, indent=1)
    os.replace(tmp, args.receipt)
    return 0


if __name__ == "__main__":
    sys.exit(main())
