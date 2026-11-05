#!/usr/bin/env bash
# V2 QEMU negative-path staging, step 3: boot the qemu-gadget RAM image on
# QEMU aarch64 virt with no network device, capture the RAW serial, and
# accept the run only if validate-usbgadgetd.sh says the daemon failed
# cleanly and the debug ramsh still reached ram:~$.
# Adapted from scripts/qemu-demo/boot.sh (same bounded-child discipline).
set -euo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
out=${QEMU_GADGET_OUT:-$repo/out/qemu-gadget}
esp="$out/esp.img"
dtb="$out/simplefb-virt.dtb"
serial="$out/serial.raw"
qlog="$out/qemu.log"
pidfile="$out/qemu.pid"
validate="$repo/scripts/qemu-demo/validate-usbgadgetd.sh"
boot_timeout=${BOOT_TIMEOUT:-300}
poll_limit=$((boot_timeout / 5))
qemu_bin=${QEMU_BIN:-qemu-system-aarch64}

for item in "$out/bootloader-e1/build/bootloader-ramboot.efi" "$out/ramboot-initfs.img" "$esp"; do
    test -f "$item" || { printf 'missing %s - run scripts/qemu-demo/stage-usbgadgetd.sh and stage-usbgadgetd-boot.sh first\n' "$item" >&2; exit 1; }
done
test -x "$validate" || { printf 'missing validator %s\n' "$validate" >&2; exit 1; }

# Hash-chain preflight (Wonder diff review of c959a28): QEMU boots ONLY
# esp.img, so existence checks on the standalone EFI/initfs prove
# nothing about what will actually run. stage-usbgadgetd-boot.sh wrote
# esp-chain.txt after reading the ESP-resident EFI back and proving the
# EFI embeds the initfs; require the CURRENT esp.img, built EFI and
# initfs to hash exactly as recorded, or a stale ESP from a previous
# initfs would pass V2 while claiming the newly staged binary booted.
chain="$out/esp-chain.txt"
test -f "$chain" || { printf 'missing %s - run scripts/qemu-demo/stage-usbgadgetd-boot.sh first (it writes the chain record)\n' "$chain" >&2; exit 1; }
chain_val() { grep "^$1 " "$chain" | cut -d ' ' -f2; }
test "$(sha256sum "$esp" | cut -d ' ' -f1)" = "$(chain_val esp_sha256)" \
    || { printf 'esp.img hash differs from esp-chain.txt - stale ESP; rerun stage-usbgadgetd-boot.sh\n' >&2; exit 1; }
test "$(sha256sum "$out/bootloader-e1/build/bootloader-ramboot.efi" | cut -d ' ' -f1)" = "$(chain_val efi_sha256)" \
    || { printf 'built EFI hash differs from esp-chain.txt - rerun stage-usbgadgetd-boot.sh\n' >&2; exit 1; }
test "$(sha256sum "$out/ramboot-initfs.img" | cut -d ' ' -f1)" = "$(chain_val initfs_sha256)" \
    || { printf 'initfs hash differs from esp-chain.txt - rerun stage-usbgadgetd.sh + stage-usbgadgetd-boot.sh\n' >&2; exit 1; }
printf 'hash chain verified: esp.img -> ESP-resident EFI (read back at stage time) -> built EFI -> embedded initfs, all hashes match esp-chain.txt\n'

"$qemu_bin" -machine virt,acpi=off,gic-version=3,dumpdtb="$out/virt.dtb" \
    -cpu max -smp 1 -m 2048 >/dev/null 2>&1
dtc -@ -I dts -O dtb -o "$out/simplefb.dtbo" "$repo/docs/workbench/K8B-SIMPLEFB.dts"
fdtoverlay -i "$out/virt.dtb" -o "$dtb" "$out/simplefb.dtbo"

qpid=''
stop_qemu() {
    [[ -n "$qpid" ]] || return 0
    local qpid2=''
    if [[ -f "$pidfile" ]]; then qpid2=$(cat "$pidfile" 2>/dev/null || true); fi
    kill -TERM "$qpid" 2>/dev/null || true
    if [[ -n "$qpid2" ]]; then kill -TERM "$qpid2" 2>/dev/null || true; fi
    for _ in 1 2 3 4 5; do
        alive=0
        if kill -0 "$qpid" 2>/dev/null; then alive=1; fi
        if [[ -n "$qpid2" ]] && kill -0 "$qpid2" 2>/dev/null; then alive=1; fi
        (( alive )) || break
        sleep 1
    done
    if kill -0 "$qpid" 2>/dev/null; then kill -KILL "$qpid" 2>/dev/null || true; fi
    if [[ -n "$qpid2" ]] && kill -0 "$qpid2" 2>/dev/null; then kill -KILL "$qpid2" 2>/dev/null || true; fi
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

sleep 3
if ! kill -0 "$qpid" 2>/dev/null; then
    printf 'QEMU exited immediately (see %s):\n' "$qlog" >&2
    cat "$qlog" >&2
    exit 1
fi

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
"$validate" "$serial" || { printf 'capture REJECTED by the V2 validator\n' >&2; exit 1; }
printf 'capture accepted: %s\n' "$serial"
