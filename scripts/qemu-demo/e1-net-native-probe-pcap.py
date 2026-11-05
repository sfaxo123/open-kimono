#!/usr/bin/env python3
"""E1-NET-NATIVE-PROBE pcap payload extractor (host-only, stdlib only).

Directional: keeps ONLY guest_ip -> host_ip TCP payload destined to `port`.
Conservative: the nonce must appear inside ONE real TCP payload; no stream
reassembly is claimed or performed.

Validations (a counterfeit payload from padding/truncation is refused):
- pcap linktype must be Ethernet (1)
- record uses the THIRD header field (incl_len) as captured length
- IPv4 version nibble, IHL >= 20, total_length sanity
- fragments rejected (MF or frag offset nonzero)
- truncated captures rejected (captured < total_length)
- TCP data offset >= 20 and within the IP payload
- payload sliced to IP total_length, so Ethernet padding cannot counterfeit
"""
import struct
import sys


def main():
    pcap, outf, gip, hip, port = sys.argv[1:6]
    port = int(port)
    try:
        d = open(pcap, "rb").read()
    except OSError:
        return
    if len(d) < 24:
        return
    m = d[:4]
    if m in (b"\xd4\xc3\xb2\xa1", b"\x4d\x3c\xb2\xa1"):
        e = "<"
    elif m in (b"\xa1\xb2\xc3\xd4", b"\xa1\xb2\x3c\x4d"):
        e = ">"
    else:
        return
    if struct.unpack(e + "I", d[20:24])[0] != 1:  # DLT_EN10MB only
        return
    o, lines = 24, []
    while o + 16 <= len(d):
        _, _, incl, _orig = struct.unpack(e + "IIII", d[o:o + 16])  # incl = 3rd
        o += 16
        if incl > len(d) - o:
            return  # truncated record cannot supply evidence
        p = d[o:o + incl]
        o += incl
        if len(p) < 14 or p[12:14] != b"\x08\x00":
            continue
        ip = p[14:]
        if len(ip) < 20 or (ip[0] >> 4) != 4:
            continue
        ihl = (ip[0] & 0xF) * 4
        if ihl < 20:
            continue
        tot = struct.unpack(">H", ip[2:4])[0]
        if tot < ihl + 20 or tot > 65535 or len(ip) < tot:  # truncated capture
            continue
        if struct.unpack(">H", ip[6:8])[0] & 0x3FFF:  # fragment
            continue
        if ip[9] != 6:  # not TCP
            continue
        if ".".join(map(str, ip[12:16])) != gip:
            continue
        if ".".join(map(str, ip[16:20])) != hip:
            continue
        tcp = ip[ihl:tot]
        if len(tcp) < 20:
            continue
        doff = (tcp[12] >> 4) * 4
        if doff < 20 or len(tcp) < doff:
            continue
        if struct.unpack(">H", tcp[2:4])[0] != port:
            continue
        pay = tcp[doff:]  # already bounded by tot: padding beyond is excluded
        if pay:
            # Preserve the real app protocol framing: the Rust probe writes
            # nonce+NEWLINE. Newlines are kept; a single trailing newline is
            # stripped so the payload line equals the protocol line exactly.
            # Other control bytes become dots. No stream reassembly.
            text = "".join(chr(c) if (32 <= c < 127 or c == 10) else "." for c in pay)
            if text.endswith("\n"):
                text = text[:-1]
            lines.append(text)
    open(outf, "w").write("\n".join(lines) + ("\n" if lines else ""))


if __name__ == "__main__":
    main()
