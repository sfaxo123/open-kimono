#!/usr/bin/env bash
# E1 QEMU gate, step 3: boot the RAM image on QEMU aarch64 virt with no network
# device at all, capture the RAW serial, and accept the run only if the strict
# validator says the app declared success after every assertion.
#
# The firmware is loaded from a FAT ESP on virtio-blk (bootindex=1), so there is
# no virtio-net and no TFTP: the guest runs with no network device present.
set -euo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
# Worktree-local artifact dir. QEMU_DEMO_OUT lets the tests run boot.sh against a
# scratch directory so a test can never destroy a real capture.
out=${QEMU_DEMO_OUT:-$repo/out/qemu-demo}
esp="$out/esp.img"
dtb="$out/simplefb-virt.dtb"
serial="$out/serial.raw"
qlog="$out/qemu.log"
pidfile="$out/qemu.pid"
validate="$repo/scripts/qemu-demo/validate.sh"
boot_timeout=${BOOT_TIMEOUT:-300}
poll_limit=$((boot_timeout / 5))

# The guest under test. Overridable so the timeout/cleanup path can be exercised
# by a stub in tests.sh without booting anything.
qemu_bin=${QEMU_BIN:-qemu-system-aarch64}

for item in "$out/bootloader-e1/build/bootloader-ramboot.efi" "$out/ramboot-initfs.img" "$esp"; do
    test -f "$item" || { printf 'missing %s — run scripts/qemu-demo/stage.sh first\n' "$item" >&2; exit 1; }
done

# Device tree: QEMU's own virt DTB plus the simple-framebuffer node the kernel
# needs to render the boot console (docs/workbench/K8B-SIMPLEFB.dts).
qemu-system-aarch64 -machine virt,acpi=off,gic-version=3,dumpdtb="$out/virt.dtb" \
    -cpu max -smp 1 -m 2048 >/dev/null 2>&1
dtc -@ -I dts -O dtb -o "$out/simplefb.dtbo" "$repo/docs/workbench/K8B-SIMPLEFB.dts"
fdtoverlay -i "$out/virt.dtb" -o "$dtb" "$out/simplefb.dtbo"

# Bounded child, two layers:
#   * `timeout --signal=TERM --kill-after=5` sends TERM at the deadline and KILL
#     five seconds later if QEMU ignored it;
#   * the EXIT trap stops the child if this script dies first (interrupted, or a
#     failure between launch and validation). QEMU records its own pid in
#     -pidfile, so the trap can reach the guest process itself and not only the
#     timeout wrapper that fronts it.
qpid=''
stop_qemu() {
    [[ -n "$qpid" ]] || return 0
    local qpid2=''
    if [[ -f "$pidfile" ]]; then qpid2=$(cat "$pidfile" 2>/dev/null || true); fi
    kill -TERM "$qpid" 2>/dev/null || true
    if [[ -n "$qpid2" ]]; then kill -TERM "$qpid2" 2>/dev/null || true; fi
    # Wait on BOTH pids: the `timeout` wrapper can die while the QEMU it fronts
    # survives, and that surviving QEMU is exactly the orphan this trap exists to
    # prevent — so liveness is judged by the pair, and KILL goes to each of them.
    for _ in 1 2 3 4 5; do
        alive=0
        if kill -0 "$qpid" 2>/dev/null; then alive=1; fi
        if [[ -n "$qpid2" ]] && kill -0 "$qpid2" 2>/dev/null; then alive=1; fi
        (( alive )) || break
        sleep 1
    done
    if kill -0 "$qpid" 2>/dev/null; then
        printf 'timeout wrapper (pid %s) ignored TERM; sending KILL\n' "$qpid" >&2
        kill -KILL "$qpid" 2>/dev/null || true
    fi
    if [[ -n "$qpid2" ]] && kill -0 "$qpid2" 2>/dev/null; then
        printf 'QEMU (pid %s) ignored TERM; sending KILL\n' "$qpid2" >&2
        kill -KILL "$qpid2" 2>/dev/null || true
    fi
    wait "$qpid" 2>/dev/null || true
    qpid=''
}
trap stop_qemu EXIT
trap 'exit 1' INT TERM

rm -f "$serial" "$qlog" "$pidfile"
timeout --signal=TERM --kill-after=5 "$boot_timeout" "$qemu_bin" \
    -machine virt,acpi=off,gic-version=3 -cpu max -smp 1 -m 2048 \
    -bios /usr/share/qemu-efi-aarch64/QEMU_EFI.fd \
    -drive if=none,id=esp,file="$esp",format=raw,readonly=on \
    -device virtio-blk-pci,drive=esp,bootindex=1 \
    -device ramfb -dtb "$dtb" \
    -pidfile "$pidfile" \
    -display none -serial file:"$serial" -monitor none \
    -no-reboot >"$qlog" 2>&1 &
qpid=$!

# Fail fast: if QEMU rejected the command line there is no serial to wait for.
sleep 3
if ! kill -0 "$qpid" 2>/dev/null; then
    printf 'QEMU exited immediately (see %s):\n' "$qlog" >&2
    cat "$qlog" >&2
    exit 1
fi

# Poll with the SAME validator that decides acceptance, so the loop can never stop
# early on a capture that the gate would reject. Exit 0 = accepted, 2 = the app
# printed FAIL (stop now, it will never recover), 1 = keep waiting.
for _ in $(seq 1 "$poll_limit"); do
    sleep 5
    rc=0; poll_msg=$("$validate" --poll "$serial" 2>&1) || rc=$?
    if (( rc == 0 )); then printf '%s\nstopping QEMU\n' "$poll_msg"; break; fi
    if (( rc == 2 )); then printf '%s\nstopping QEMU early\n' "$poll_msg" >&2; break; fi
    if ! kill -0 "$qpid" 2>/dev/null; then
        printf 'QEMU exited before the capture was accepted (see %s)\n' "$qlog" >&2
        break
    fi
done

stop_qemu

test -s "$serial" || { printf 'no serial output captured\n' >&2; cat "$qlog" >&2; exit 1; }
"$validate" "$serial" || { printf 'capture REJECTED by the strict validator\n' >&2; exit 1; }
printf 'capture accepted: %s\n' "$serial"
