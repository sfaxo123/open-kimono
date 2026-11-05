#!/usr/bin/env python3
"""Hands-free exit from Qualcomm crash-dump mode (USB 05c6:900e) for the OnePlus 6.

When a watchdog bite or kernel crash leaves the phone in crash-dump ("memory debug")
mode, the boot ROM/XBL speaks the Sahara protocol on a vendor bulk interface and waits
for a host to collect RAM. This tool collects NOTHING: it answers the device's HELLO in
memory-debug mode, ignores the memory table it is offered, and sends Sahara RESET, which
reboots the SoC into the normal boot chain (pmOS on flash).

Envelope: no memory reads, no image transfer, no firehose programmer, nothing written to
the phone's storage. Only 05c6:900e in Sahara mode 2 (memory debug) is accepted; EDL
(05c6:9008, image-transfer mode) and any other mode are refused without a reset.

Packets (little-endian u32 words): HELLO 0x01 (48 B), HELLO_RESP 0x02 (48 B),
RESET 0x07 (8 B), RESET_RESP 0x08 (8 B), MEMORY_DEBUG 0x09 (16 B), MEMORY_DEBUG_64 0x10
(24 B). Run with the venv that has pyusb + libusb-package: ~/op6-r1/venv/bin/python.
"""
import argparse
import json
import os
import struct
import sys
import time
from typing import Optional

VID, PID_CRASHDUMP, PID_EDL = 0x05C6, 0x900E, 0x9008

CMD_HELLO, CMD_HELLO_RESP = 0x01, 0x02
CMD_RESET, CMD_RESET_RESP = 0x07, 0x08
CMD_MEMORY_DEBUG, CMD_MEMORY_DEBUG_64 = 0x09, 0x10
MODE_MEMORY_DEBUG = 2


class SaharaError(Exception):
    pass


def hello_resp(mode: int) -> bytes:
    # cmd, len, version, version_supported, status(0 = success), mode, reserved[6]
    return struct.pack("<12I", CMD_HELLO_RESP, 0x30, 2, 1, 0, mode, 0, 0, 0, 0, 0, 0)


def reset_req() -> bytes:
    return struct.pack("<2I", CMD_RESET, 8)


def parse(pkt: bytes) -> dict:
    if len(pkt) < 8:
        raise SaharaError(f"short packet ({len(pkt)} B)")
    cmd, length = struct.unpack_from("<2I", pkt)
    out = {"cmd": cmd, "len": length}
    if cmd == CMD_HELLO and len(pkt) >= 24:
        _, _, ver, ver_sup, max_len, mode = struct.unpack_from("<6I", pkt)
        out.update(version=ver, version_supported=ver_sup, max_cmd_len=max_len, mode=mode)
    elif cmd == CMD_MEMORY_DEBUG and len(pkt) >= 16:
        out.update(table_addr=struct.unpack_from("<I", pkt, 8)[0],
                   table_len=struct.unpack_from("<I", pkt, 12)[0])
    elif cmd == CMD_MEMORY_DEBUG_64 and len(pkt) >= 24:
        out.update(table_addr=struct.unpack_from("<Q", pkt, 8)[0],
                   table_len=struct.unpack_from("<Q", pkt, 16)[0])
    return out


def run_reset(xport, log) -> dict:
    """Drive one Sahara exchange to RESET. xport: read(timeout_ms) -> bytes|None, write(bytes)."""
    result = {"hello": None, "offer": None, "reset_sent": False, "reset_resp": False}
    pkt = xport.read(3000)
    if pkt is None:
        # HELLO may have been consumed by an earlier reader; the device then waits for
        # HELLO_RESP. Answer in memory-debug mode, but only reset if the device confirms
        # that mode by offering its memory table.
        log("no HELLO within 3 s; answering mode 2 and requiring a MEMORY_DEBUG offer")
    else:
        hello = parse(pkt)
        result["hello"] = hello
        log(f"HELLO {hello}")
        if hello["cmd"] != CMD_HELLO:
            raise SaharaError(f"expected HELLO, got cmd 0x{hello['cmd']:x}")
        if hello.get("mode") != MODE_MEMORY_DEBUG:
            raise SaharaError(f"Sahara mode {hello.get('mode')} is not memory-debug (2); refusing")
    xport.write(hello_resp(MODE_MEMORY_DEBUG))
    log("HELLO_RESP sent (mode 2)")
    pkt = xport.read(3000)
    if pkt is None and result["hello"] is None:
        raise SaharaError("no HELLO and no MEMORY_DEBUG offer; mode unconfirmed, refusing reset")
    if pkt is not None:
        offer = parse(pkt)
        result["offer"] = offer
        log(f"device offered {offer} (not read)")
        if offer["cmd"] not in (CMD_MEMORY_DEBUG, CMD_MEMORY_DEBUG_64):
            raise SaharaError(f"expected MEMORY_DEBUG, got cmd 0x{offer['cmd']:x}; refusing")
    xport.write(reset_req())
    result["reset_sent"] = True
    log("RESET sent")
    pkt = xport.read(2000)
    if pkt is not None and parse(pkt)["cmd"] == CMD_RESET_RESP:
        result["reset_resp"] = True
        log("RESET_RESP received")
    return result


class UsbTransport:
    def __init__(self, dev):
        import usb.util
        self.dev = dev
        cfg = dev.get_active_configuration()
        for intf in cfg:
            eps = list(intf)
            ep_in = usb.util.find_descriptor(intf, custom_match=lambda e: usb.util.endpoint_direction(
                e.bEndpointAddress) == usb.util.ENDPOINT_IN and usb.util.endpoint_type(e.bmAttributes) == usb.util.ENDPOINT_TYPE_BULK)
            ep_out = usb.util.find_descriptor(intf, custom_match=lambda e: usb.util.endpoint_direction(
                e.bEndpointAddress) == usb.util.ENDPOINT_OUT and usb.util.endpoint_type(e.bmAttributes) == usb.util.ENDPOINT_TYPE_BULK)
            if eps and ep_in is not None and ep_out is not None:
                usb.util.claim_interface(dev, intf.bInterfaceNumber)
                self.intf, self.ep_in, self.ep_out = intf.bInterfaceNumber, ep_in, ep_out
                return
        raise SaharaError("no bulk IN/OUT interface on the crash-dump device")

    def read(self, timeout_ms: int) -> Optional[bytes]:
        import usb.core
        try:
            return bytes(self.ep_in.read(0x1000, timeout=timeout_ms))
        except usb.core.USBTimeoutError:
            return None

    def write(self, data: bytes) -> None:
        self.ep_out.write(data, timeout=3000)

    def close(self) -> None:
        import usb.util
        try:
            usb.util.release_interface(self.dev, self.intf)
        finally:
            usb.util.dispose_resources(self.dev)


def find(backend, pid):
    import usb.core
    return usb.core.find(idVendor=VID, idProduct=pid, backend=backend)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--probe", action="store_true", help="only report whether 05c6:900e is present")
    ap.add_argument("--receipt-dir", default=os.path.expanduser("~/op6-r1/out/sahara-receipts"))
    args = ap.parse_args()

    import libusb_package
    backend = libusb_package.get_libusb1_backend()
    stamp = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
    lines = []

    def log(msg):
        line = f"{time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())} {msg}"
        lines.append(line)
        print(line, flush=True)

    if find(backend, PID_EDL) is not None:
        log("05c6:9008 (EDL) present - out of envelope, refusing")
        return 4
    dev = find(backend, PID_CRASHDUMP)
    if dev is None:
        log("no 05c6:900e device")
        return 3
    if args.probe:
        log("05c6:900e present")
        return 0

    receipt = {"tool": "op6-sahara-reset", "version": 1, "started": stamp, "outcome": "ERROR"}
    xport = None
    try:
        xport = UsbTransport(dev)
        receipt.update(run_reset(xport, log))
        receipt["outcome"] = "RESET_SENT"
        rc = 0
    except Exception as exc:  # report every failure in the receipt
        log(f"ERROR {exc}")
        receipt["error"] = str(exc)
        rc = 2
    finally:
        if xport is not None:
            try:
                xport.close()
            except Exception:
                pass
    receipt["log"] = lines
    os.makedirs(args.receipt_dir, exist_ok=True)
    path = os.path.join(args.receipt_dir, f"sahara-{stamp}.json")
    with open(path, "w") as fh:
        json.dump(receipt, fh, indent=1)
    log(f"receipt {path} outcome={receipt['outcome']}")
    return rc


if __name__ == "__main__":
    sys.exit(main())
