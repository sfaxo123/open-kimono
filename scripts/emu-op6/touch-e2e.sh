#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
#
# End-to-end touch check for an EMU_TOUCH=1 ramboot payload on the external
# GPL qemu-oneplus6 `-M oneplus6` (invoked through run.sh; no QEMU code here).
# After rmi4d arms its attention IRQ, QMP input-send-event delivers a press and
# then a release at (X,Y) to the board's rmi4-s3706 input handler. The events
# carry no `device`, so the model receives raw panel coordinates. The serial
# log must then show PDT discovery, the TLMM->PDC->GIC wake route, both
# attention IRQs in rmi4d, and the pointer events inside inputd, in order.
#
# Usage: scripts/emu-op6/touch-e2e.sh [payload [run.sh options ...]]
# Extra options go to run.sh, e.g. --qemu-arg -trace --qemu-arg 'rmi4_s3706_*'.
set -euo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
payload=${1:-$repo/out/phone-boot/redox-ramboot-emu-touch.efi}
shift || true
x=${EMU_TOUCH_X:-540}
y=${EMU_TOUCH_Y:-1140}
timeout_s=${EMU_TOUCH_TIMEOUT:-150}
run_id=${EMU_OP6_RUN_ID:-touch-e2e-$(date +%Y%m%d-%H%M%S)}
run_dir=${EMU_OP6_OUT_ROOT:-$repo/out/emu-op6}/$run_id
# AF_UNIX socket paths are limited to 108 bytes; the run directory can exceed that.
sock_dir=$(mktemp -d /tmp/emu-op6-qmp.XXXXXX)
sock=$sock_dir/qmp.sock
injector_pid=""
cleanup() {
    if [[ -n $injector_pid ]]; then
        kill "$injector_pid" 2>/dev/null || true
    fi
    rm -rf "$sock_dir"
}
trap cleanup EXIT

mkdir -p "$run_dir"
rm -f "$run_dir/serial.log" "$run_dir/qmp.txt"

python3 - "$run_dir/serial.log" "$sock" "$run_dir/qmp.txt" "$x" "$y" "$timeout_s" <<'PY' &
import json
import socket
import sys
import time

serial, sock_path, log_path = sys.argv[1:4]
x, y, timeout_s = int(sys.argv[4]), int(sys.argv[5]), float(sys.argv[6])
deadline = time.monotonic() + timeout_s


def wait_for(marker, after=0):
    needle = marker.encode()
    while time.monotonic() < deadline:
        try:
            with open(serial, "rb") as f:
                data = f.read()
        except FileNotFoundError:
            data = b""
        found = data.find(needle, after)
        if found >= 0:
            return found + len(needle)
        time.sleep(0.1)
    sys.exit(f"touch-e2e: timed out waiting for {marker!r}")


pos = wait_for("rmi4d: initial drain")
with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as sock, open(log_path, "w") as log:
    sock.settimeout(10)
    sock.connect(sock_path)
    stream = sock.makefile("rwb", buffering=0)

    def receive():
        while True:
            message = json.loads(stream.readline())
            if "event" not in message:
                return message

    log.write(f"greeting {json.dumps(json.loads(stream.readline()))}\n")

    def command(request):
        stream.write((json.dumps(request) + "\n").encode())
        response = receive()
        log.write(f"request {json.dumps(request)}\nresponse {json.dumps(response)}\n")
        log.flush()
        if "error" in response:
            sys.exit(f"touch-e2e: QMP {request['execute']} failed: {response['error']}")

    def touch(down):
        events = [
            {"type": "btn", "data": {"button": "left", "down": down}},
            {"type": "abs", "data": {"axis": "x", "value": x}},
            {"type": "abs", "data": {"axis": "y", "value": y}},
        ]
        command({"execute": "input-send-event", "arguments": {"events": events}})

    command({"execute": "qmp_capabilities"})
    touch(True)
    pos = wait_for("inputd: producer button left=true", pos)
    # The model only clears a slot on an ABS sync, so the release repeats X/Y.
    touch(False)
    wait_for("inputd: producer button left=false", pos)
PY
injector_pid=$!

run_rc=0
"$repo/scripts/emu-op6/run.sh" "$payload" --run-id "$run_id" --timeout "$timeout_s" \
    --qemu-arg -qmp --qemu-arg "unix:$sock,server=on,wait=off" "$@" \
    --marker 'PDC probe: phys=0xb220000' \
    --marker 'TLMM probe: phys=0x3400000' \
    --marker 'rmi4d: PDT at I2C 0xa90000 address 0x20: F01 query=0x10 command=0x28 control=0x29 data=0x06' \
    --marker 'F12 query=0x50 command=0x00 control=0x70 data=0x80' \
    --marker 'IRQ wake route: controller 0x4a L2(125, 2) -> controller 0x8d pin 100 -> GIC L3(0, 615, 1) virq 647' \
    --marker 'rmi4d: attention IRQ /scheme/irq/phandle-74/125,2 armed' \
    --marker 'rmi4d: initial drain contacts=[]' \
    --marker "rmi4d: attention IRQ count=1 contacts=[Contact { slot: 0, x: $x, y: $y," \
    --marker "inputd: producer pointer x=$x y=$y" \
    --marker 'inputd: producer button left=true' \
    --marker 'rmi4d: attention IRQ count=2 contacts=[]' \
    --marker 'inputd: producer button left=false' || run_rc=$?

injector_rc=0
wait "$injector_pid" || injector_rc=$?
injector_pid=""
printf 'touch-e2e: run.sh rc=%s injector rc=%s serial=%s\n' "$run_rc" "$injector_rc" "$run_dir/serial.log"
(( run_rc == 0 && injector_rc == 0 ))
