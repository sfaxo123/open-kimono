#!/usr/bin/env bash
set -euo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
fork=${REDOX_FORKS:-$HOME/Developer/forks/redox}
image="$fork/redox/build/aarch64/desktop-minimal/e1-live.iso"
tftp="$fork/bootloader-e1/build/tftp-live"
dtb="$repo/out/integration/simplefb-virt.dtb"
output="$repo/out/integration"
monitor="$output/e1-live-monitor.sock"
serial="$output/e1-live-qemu.log"
ppm="$output/e1-timeline.ppm"
png="$repo/docs/workbench/E1-REDOX-TIMELINE.png"
test -f "$image" && test -f "$tftp/bootaa64.efi" && test -f "$dtb"
mkdir -p "$output"
rm -f "$monitor" "$serial" "$ppm"

qemu-system-aarch64 -machine virt,acpi=off,gic-version=3 -cpu max -smp 1 -m 2048 \
    -bios /usr/share/qemu-efi-aarch64/QEMU_EFI.fd \
    -drive "if=none,id=e1disk,file=$image,format=raw" \
    -device virtio-blk-pci,drive=e1disk,bootindex=2 \
    -device ramfb -dtb "$dtb" \
    -netdev "user,id=e1net,tftp=$tftp,bootfile=bootaa64.efi" \
    -device virtio-net-pci,netdev=e1net,bootindex=1 \
    -display none -serial "file:$serial" \
    -monitor "unix:$monitor,server,nowait" >"$output/e1-qemu-stderr.log" 2>&1 &
qemu_pid=$!
stop_qemu() {
    if [[ -S "$monitor" ]]; then
        printf 'quit\n' | socat - "UNIX-CONNECT:$monitor" >/dev/null 2>&1 || true
    fi
    wait "$qemu_pid" 2>/dev/null || true
}
trap stop_qemu EXIT

for _ in {1..120}; do
    if grep -q 'timeline-demo-publish: sent 28 events to trace:publish' "$serial" 2>/dev/null; then
        break
    fi
    kill -0 "$qemu_pid" 2>/dev/null || { echo "QEMU exited early" >&2; exit 1; }
    sleep 1
done
grep -q 'timeline-demo-publish: sent 28 events to trace:publish' "$serial"
grep -q 'vesad: simplefb 800x600' "$serial"
grep -q 'init: switchroot to /usr' "$serial"
ready=0
for _ in {1..40}; do
    printf 'screendump %s\n' "$ppm" | socat - "UNIX-CONNECT:$monitor" >/dev/null
    python3 - "$ppm" "$png" <<'PY'
import sys
from PIL import Image
Image.open(sys.argv[1]).convert("RGB").save(sys.argv[2])
PY
    if python3 - "$png" <<'PY'
import sys
from PIL import Image
im = Image.open(sys.argv[1]).convert("RGB")
orange = sum(r > 170 and 65 < g < 190 and b < 80 for r, g, b in im.get_flattened_data())
sys.exit(0 if orange >= 20 else 1)
PY
    then
        ready=1
        break
    fi
    sleep 1
done
test "$ready" = 1
python3 - "$serial" "$repo/docs/workbench/E1-REDOX-SERIAL.txt" <<'PY'
import re
import sys
from pathlib import Path
raw = Path(sys.argv[1]).read_bytes().decode("utf-8", "replace")
plain = re.sub(r"\x1b\[[0-?]*[ -/]*[@-~]", "", raw)
Path(sys.argv[2]).write_text("\n".join(line.rstrip() for line in plain.splitlines()) + "\n")
PY
printf 'E1 screenshot: %s\n' "$png"
