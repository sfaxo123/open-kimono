#!/usr/bin/env python3
"""E1-WY-GUEST-HARNESS pcap verifier — ONE pinned TCP connection + Wyoming wire parse.

Conservative stdlib parse of the QEMU filter-dump capture (DLT_EN10MB):
  * IPv4/TCP only; fragments, bad magic, truncated record headers (incl. a
    trailing partial header), ip gaps, and CONFLICTING retransmits fail closed (2)
  * flows are keyed by the FULL 4-tuple; one guestIP/guestPort <-> hostIP/hostPort
    connection is pinned and BOTH directions must belong to that same tuple
  * each direction is reassembled by TCP seq (gap -> 2; retransmit whose overlap
    differs from the already-buffered bytes -> 2)
  * streams are parsed SEQUENTIALLY as the Wyoming wire contract: a JSON header
    line ending in \\n, then exactly data_length payload bytes. Arbitrary bytes
    are never regex-searched for a header that happens to match.
PASS (0) requires one pinned connection whose up stream contains a Wyoming
"ping" event whose text == nonce and whose down stream (same tuple, reversed)
contains a "pong" event with the same nonce. Negatives that MUST fail: wrong
client port/IP, nonce only inside an unrelated payload, missing pong, text !=
nonce, truncated header/record, padding-only.
Exit: 0 pass | 1 evidence missing | 2 parse refused / unsupported.
"""
import argparse
import ipaddress
import json
import struct
import sys


def parse(path, host_ip, port):
    try:
        d = open(path, "rb").read()
    except OSError:
        return 2, {"error": "pcap unreadable"}
    if len(d) < 24:
        return 2, {"error": "pcap too short"}
    m = d[:4]
    if m in (b"\xd4\xc3\xb2\xa1", b"\x4d\x3c\xb2\xa1"):
        e = "<"
    elif m in (b"\xa1\xb2\xc3\xd4", b"\xa1\xb2\x3c\x4d"):
        e = ">"
    else:
        return 2, {"error": "bad pcap magic"}
    if struct.unpack(e + "I", d[20:24])[0] != 1:      # DLT_EN10MB only
        return 2, {"error": "unsupported link type"}
    o = 24
    flows = {}                                        # (src,dst,sport,dport) -> [(seq,pl)]
    while o < len(d):
        if o + 16 > len(d):                           # trailing truncated record header
            return 2, {"error": "truncated pcap record header"}
        _, _, incl, _ = struct.unpack(e + "IIII", d[o:o + 16])
        o += 16
        if incl > len(d) - o:                         # truncated record data
            return 2, {"error": "truncated pcap record"}
        p = d[o:o + incl]
        o += incl
        if len(p) < 14 or p[12:14] != b"\x08\x00":
            continue
        ip = p[14:]
        if len(ip) < 20 or (ip[0] >> 4) != 4:
            continue
        ihl = (ip[0] & 0xF) * 4
        if ihl < 20 or len(ip) < ihl:
            continue
        tot = struct.unpack(">H", ip[2:4])[0]
        if tot < ihl + 20 or len(ip) < tot:
            return 2, {"error": "truncated ip packet"}
        if struct.unpack(">H", ip[6:8])[0] & 0x3FFF:   # fragment: unsupported
            return 2, {"error": "ip fragment unsupported"}
        if ip[9] != 6:
            continue
        tcp = ip[ihl:tot]
        doff = (tcp[12] >> 4) * 4
        if doff < 20 or doff > len(tcp):
            continue
        payload = tcp[doff:]
        if not payload:
            continue
        sport, dport = struct.unpack(">HH", tcp[0:4])
        seq = struct.unpack(">I", tcp[4:8])[0]
        src = str(ipaddress.IPv4Address(ip[12:16]))
        dst = str(ipaddress.IPv4Address(ip[16:20]))
        flows.setdefault((src, dst, sport, dport), []).append((seq, payload))
    return 0, {"flows": flows, "host": (host_ip, port)}


def reassemble(chunks):
    """Contiguous seq reassembly. A gap or a retransmit whose overlapping bytes
    conflict with what was already buffered fails closed (None)."""
    if not chunks:
        return b""
    chunks = sorted(chunks, key=lambda c: c[0])
    out = bytearray()
    base = chunks[0][0]
    for seq, payload in chunks:
        end = base + len(out)
        if seq == end:
            out += payload
        elif seq < end:                                # retransmit: must agree
            delta = end - seq
            overlap = min(delta, len(payload))
            start = seq - base
            if payload[:overlap] != bytes(out[start:start + overlap]):
                return None                            # conflicting retransmit
            if delta < len(payload):
                out += payload[delta:]
        else:
            return None                                # gap: unsupported
    return bytes(out)


def wy_events(stream):
    """Sequential Wyoming wire parse: JSON header line + data_length payload.
    Returns [(header, data)] or None if the stream is not a clean Wyoming
    sequence (truncated header, bad JSON, truncated payload) -> fail closed."""
    evs = []
    i = 0
    n = len(stream)
    while i < n:
        j = stream.find(b"\n", i)
        if j == -1:
            return None                                # truncated header line
        try:
            obj = json.loads(stream[i:j])
        except ValueError:
            return None
        if not isinstance(obj, dict):
            return None
        dl = obj.get("data_length", 0)
        if not isinstance(dl, int) or dl < 0:
            return None
        start = j + 1
        if start + dl > n:
            return None                                # truncated payload
        evs.append((obj, stream[start:start + dl] if dl else None))
        i = start + dl
    return evs


def stream_has_nonce(evs, etype, nonce):
    if evs is None:
        return False
    for obj, data in evs:
        if obj.get("type") != etype:
            continue
        if obj.get("text") == nonce:
            return True
        if data is not None:                           # nonce only counts in a
            try:                                       # parsed Wyoming payload
                if json.loads(data).get("text") == nonce:
                    return True
            except ValueError:
                pass
    return False


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("pcap")
    ap.add_argument("port", type=int)
    ap.add_argument("nonce")
    ap.add_argument("--host-ip", default="10.0.2.2")
    args = ap.parse_args()
    rc, ev = parse(args.pcap, args.host_ip, args.port)
    if rc:
        print("PCAP_VERIFY_REFUSED\t%s" % ev.get("error", "?"))
        return rc
    host_ip, port = ev["host"]
    # Pin ONE full connection: the unique guest tuple whose up stream carries
    # the Wyoming ping for this nonce; the pong must be on that SAME tuple.
    pinned = []
    for (src, dst, sport, dport), chunks in ev["flows"].items():
        if dst != host_ip or dport != port:
            continue
        up = reassemble(chunks)
        if up is None:
            print("PCAP_VERIFY_REFUSED\tgap/conflicting retransmit on up flow %s:%d" % (src, sport))
            return 2
        evs = wy_events(up)
        if evs is None:
            print("PCAP_VERIFY_REFUSED\ttruncated/invalid wyoming header on up flow %s:%d" % (src, sport))
            return 2
        if stream_has_nonce(evs, "ping", args.nonce):
            pinned.append((src, sport, up))
    print("pcap.candidate_conns\t%d" % len(pinned))
    up_ok = down_ok = False
    up_bytes = down_bytes = 0
    for src, sport, up in pinned:
        down_chunks = ev["flows"].get((host_ip, src, port, sport), [])
        down = reassemble(down_chunks)
        if down is None:
            print("PCAP_VERIFY_REFUSED\tgap/conflicting retransmit on down flow to %s:%d" % (src, sport))
            return 2
        devs = wy_events(down)
        if devs is None:
            print("PCAP_VERIFY_REFUSED\ttruncated/invalid wyoming header on down flow to %s:%d" % (src, sport))
            return 2
        up_ok = True
        up_bytes = len(up)
        down_bytes = max(down_bytes, len(down))
        if stream_has_nonce(devs, "pong", args.nonce):
            down_ok = True
            break
    print("pcap.ping_on_exact_conn\t%s" % ("yes" if up_ok else "no"))
    print("pcap.pong_on_exact_conn\t%s" % ("yes" if down_ok else "no"))
    # legacy-compatible keys (same meaning: pinned-connection evidence)
    print("pcap.ping_on_exact_port\t%s" % ("yes" if up_ok else "no"))
    print("pcap.pong_on_exact_port\t%s" % ("yes" if down_ok else "no"))
    print("pcap.up_bytes\t%d" % up_bytes)
    print("pcap.down_bytes\t%d" % down_bytes)
    return 0 if (up_ok and down_ok) else 1


if __name__ == "__main__":
    sys.exit(main())
