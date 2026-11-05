#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
#
# Fake-QEMU fixture suite for scripts/emu-op6/run.sh (no real QEMU boot).
# Proves:
#   1. Marker order enforced (in-order passes, out-of-order fails, missing fails,
#      U-Boot serial prompt handshake drives bootefi).
#   2. Timeout kills (stubborn fake-QEMU ignoring SIGTERM is killed on timeout).
#   3. No orphan qemu (on normal marker-matched exit and on external SIGTERM to
#      run.sh, even when fake-QEMU ignores SIGTERM).
#   4. Receipt written (receipt.tsv records exact SHA-256 hashes of emulator
#      binary, payload, and modified ESP copy; base ESP stays untouched).
set -euo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
run_sh="$repo/scripts/emu-op6/run.sh"
scratch="$repo/out/emu-op6/test-scratch-$$"
out_root="$scratch/runs"
rm -rf "$scratch"
mkdir -p "$out_root"

cleanup_scratch() {
    rm -rf "$scratch"
}
trap cleanup_scratch EXIT

pass=0
fail=0
ok() {
    printf '  PASS  %s\n' "$1"
    pass=$((pass + 1))
}
bad() {
    printf '  FAIL  %s\n        %s\n' "$1" "${2:-}"
    fail=$((fail + 1))
}
expect_rc() {
    local label="$1" expected="$2" actual="$3" detail="${4:-}"
    if [[ "$expected" == "$actual" ]]; then
        ok "$label"
    else
        bad "$label" "expected exit $expected, got $actual ($detail)"
    fi
}

phone_boot_dir="${EMU_OP6_PHONE_BOOT:-$HOME/artifacts/handoff/oneplus-redox-20261024/phone-boot}"
mformat_bin=""
mmd_bin=""
mcopy_bin=""
if command -v mformat >/dev/null 2>&1; then
    mformat_bin="$(command -v mformat)"
    mmd_bin="$(command -v mmd)"
    mcopy_bin="$(command -v mcopy)"
elif [[ -x "$phone_boot_dir/host-tools/usr/bin/mformat" ]]; then
    mformat_bin="$phone_boot_dir/host-tools/usr/bin/mformat"
    mmd_bin="$phone_boot_dir/host-tools/usr/bin/mmd"
    mcopy_bin="$phone_boot_dir/host-tools/usr/bin/mcopy"
fi

# Build lightweight synthetic fixtures (fake U-Boot .gz-dtb, fake ESP image, fake payload)
# so tests.sh is fast and completely self-contained.
fake_payload="$scratch/fake-payload.efi"
printf 'MZ-FAKE-EFI-PAYLOAD-%s\n' "$$" >"$fake_payload"
fake_payload_sha=$(sha256sum "$fake_payload" | awk '{print $1}')

fake_uboot_gz_dtb="$scratch/fake-u-boot.bin.gz-dtb"
python3 - "$fake_uboot_gz_dtb" <<'PY'
import gzip
import pathlib
import subprocess
import sys
import tempfile

out_path = pathlib.Path(sys.argv[1])
with tempfile.TemporaryDirectory() as td:
    td = pathlib.Path(td)
    dts = td / "test.dts"
    dtb = td / "test.dtb"
    dts.write_text(
        "/dts-v1/;\n"
        "/ {\n"
        "    model = \"OnePlus 6 Fake Fixture\";\n"
        "    #address-cells = <2>;\n"
        "    #size-cells = <2>;\n"
        "    memory@80000000 {\n"
        "        device_type = \"memory\";\n"
        "        reg = <0x0 0x80000000 0x0 0x40000000>;\n"
        "    };\n"
        "};\n"
    )
    subprocess.run(["dtc", "-I", "dts", "-O", "dtb", "-o", str(dtb), str(dts)], check=True)
    gz_bytes = gzip.compress(b"FAKE-UBOOT-NODTB-BINARY\n")
    out_path.write_bytes(gz_bytes + dtb.read_bytes())
PY
fake_uboot_sha=$(sha256sum "$fake_uboot_gz_dtb" | awk '{print $1}')

fake_esp="$scratch/fake-esp.img"
# Create a small 4 MiB image with a FAT filesystem at offset 1 MiB (1048576)
truncate -s 4M "$fake_esp"
"$mformat_bin" -i "${fake_esp}@@1048576" -F -v FAKEESP ::
"$mmd_bin" -i "${fake_esp}@@1048576" ::/EFI ::/EFI/BOOT
printf 'INITIAL-BOOTAA64\n' >"$scratch/initial.efi"
"$mcopy_bin" -i "${fake_esp}@@1048576" "$scratch/initial.efi" ::/EFI/BOOT/BOOTAA64.EFI
fake_esp_base_sha=$(sha256sum "$fake_esp" | awk '{print $1}')

# Helper to create a fake QEMU script that records its PID to -pidfile and $FAKE_PID_RECORD
make_fake_qemu() {
    local path="$1"
    local body="$2"
    cat >"$path" <<EOF
#!/usr/bin/env bash
set -euo pipefail
pidfile=""
args=("\$@")
for i in "\${!args[@]}"; do
    if [[ "\${args[\$i]}" == "-pidfile" ]]; then
        pidfile="\${args[\$((i + 1))]}"
    fi
done
if [[ -n "\$pidfile" ]]; then
    printf '%s\n' "\$\$" >"\$pidfile"
fi
if [[ -n "\${FAKE_PID_RECORD:-}" ]]; then
    printf '%s\n' "\$\$" >"\$FAKE_PID_RECORD"
fi
$body
EOF
    chmod +x "$path"
}

run_harness() {
    env \
        EMU_OP6_UBOOT_GZ_DTB="$fake_uboot_gz_dtb" \
        EMU_OP6_UBOOT_SHA256="$fake_uboot_sha" \
        EMU_OP6_ESP_BASE="$fake_esp" \
        EMU_OP6_OUT_ROOT="$out_root" \
        EMU_OP6_KILL_AFTER=1 \
        "$@"
}

# ==============================================================================
# 1. Marker order enforced & U-Boot serial prompt driving
# ==============================================================================
printf '1. marker order & serial prompt driving\n'

fake_uboot_qemu="$scratch/fake-qemu-uboot.sh"
make_fake_qemu "$fake_uboot_qemu" '
python3 -c '\''
import os
def w(b):
    os.write(1, b)
w(b"U-Boot 2026.10-fake\r\n")
w(b"Press any key to continue...")
os.read(0, 1)
w(b"\r\n*** U-Boot Boot Menu ***\r\n")
key = os.read(0, 1)
if key == b"5":
    w(b"=> ")
cmd = bytearray()
while True:
    ch = os.read(0, 1)
    if not ch or ch in (b"\r", b"\n"):
        break
    cmd.extend(ch)
w(bytes(cmd) + b"\r\n")
if b"bootefi" in cmd:
    w(b"OnePlus 6 EFI diagnostic\r\nCurrentEL: EL1\r\n=> ")
'\''
'

rc=0
out=$(QEMU_OP6="$fake_uboot_qemu" run_harness "$run_sh" "$fake_payload" \
    --run-id test-order-pass \
    --marker "U-Boot 2026.10-fake" \
    --marker "=> " \
    --marker "OnePlus 6 EFI diagnostic" \
    --marker "CurrentEL: EL1" \
    --timeout 10 2>&1) || rc=$?
expect_rc "in-order markers with U-Boot prompt handshake pass" 0 "$rc" "$out"

# Out-of-order markers must fail
fake_ooo_qemu="$scratch/fake-qemu-ooo.sh"
make_fake_qemu "$fake_ooo_qemu" '
printf "STAGE_TWO\r\n"
printf "STAGE_ONE\r\n"
'

rc=0
out=$(QEMU_OP6="$fake_ooo_qemu" run_harness "$run_sh" "$fake_payload" \
    --run-id test-order-fail \
    --marker "STAGE_ONE" \
    --marker "STAGE_TWO" \
    --timeout 5 2>&1) || rc=$?
if (( rc != 0 )); then
    ok "out-of-order markers are rejected (exit $rc)"
else
    bad "out-of-order markers are rejected" "expected non-zero exit, got 0"
fi
case "$out" in
    *"marker order assertion failed"*|*"appeared only before"*)
        ok "out-of-order failure names the ordering violation"
        ;;
    *)
        bad "out-of-order failure names the ordering violation" "$out"
        ;;
esac

# Missing marker must fail
rc=0
out=$(QEMU_OP6="$fake_ooo_qemu" run_harness "$run_sh" "$fake_payload" \
    --run-id test-missing-fail \
    --marker "STAGE_TWO" \
    --marker "STAGE_NEVER_PRINTED" \
    --timeout 5 2>&1) || rc=$?
if (( rc != 0 )); then
    ok "missing marker is rejected (exit $rc)"
else
    bad "missing marker is rejected" "expected non-zero exit, got 0"
fi

# ==============================================================================
# 2. Timeout kills stubborn QEMU (ignores SIGTERM)
# ==============================================================================
printf '\n2. timeout enforcement with kill-after\n'

fake_hang_qemu="$scratch/fake-qemu-hang.sh"
make_fake_qemu "$fake_hang_qemu" '
trap "" TERM
printf "U-Boot starting but hanging before prompt...\r\n"
while :; do sleep 0.2; done
'

pid_rec_timeout="$scratch/pid-timeout"
rm -f "$pid_rec_timeout"
rc=0
out=$(FAKE_PID_RECORD="$pid_rec_timeout" QEMU_OP6="$fake_hang_qemu" \
    run_harness "$run_sh" "$fake_payload" \
    --run-id test-timeout-kill \
    --marker "=> " \
    --timeout 2 2>&1) || rc=$?
if (( rc != 0 )); then
    ok "timeout exits non-zero (exit $rc)"
else
    bad "timeout exits non-zero" "expected non-zero exit, got 0"
fi

timeout_qemu_pid=$(cat "$pid_rec_timeout" 2>/dev/null || true)
if [[ -z "$timeout_qemu_pid" ]]; then
    bad "fake QEMU recorded its PID on timeout run" "no PID found"
elif kill -0 "$timeout_qemu_pid" 2>/dev/null; then
    kill -KILL "$timeout_qemu_pid" 2>/dev/null || true
    bad "timeout killed stubborn fake QEMU (ignoring SIGTERM)" "PID $timeout_qemu_pid still alive"
else
    ok "timeout killed stubborn fake QEMU (PID $timeout_qemu_pid reaped)"
fi

# ==============================================================================
# 3. No orphan QEMU on normal exit or signal interruption
# ==============================================================================
printf '\n3. no orphan qemu on exit paths (trap)\n'

# 3a. Fake QEMU prints required markers then ignores SIGTERM and loops forever.
fake_linger_qemu="$scratch/fake-qemu-linger.sh"
make_fake_qemu "$fake_linger_qemu" '
trap "" TERM
printf "BOOT_BANNER_OK\r\nPAYLOAD_DONE_OK\r\n"
while :; do sleep 0.2; done
'

pid_rec_linger="$scratch/pid-linger"
rm -f "$pid_rec_linger"
rc=0
out=$(FAKE_PID_RECORD="$pid_rec_linger" QEMU_OP6="$fake_linger_qemu" \
    run_harness "$run_sh" "$fake_payload" \
    --run-id test-no-orphan-normal \
    --marker "BOOT_BANNER_OK" \
    --marker "PAYLOAD_DONE_OK" \
    --timeout 15 2>&1) || rc=$?
expect_rc "marker-matched run with lingering fake QEMU exits 0" 0 "$rc" "$out"

linger_pid=$(cat "$pid_rec_linger" 2>/dev/null || true)
if [[ -z "$linger_pid" ]]; then
    bad "lingering fake QEMU recorded its PID" "no PID found"
elif kill -0 "$linger_pid" 2>/dev/null; then
    kill -KILL "$linger_pid" 2>/dev/null || true
    bad "no orphan QEMU after normal exit" "PID $linger_pid still alive"
else
    ok "no orphan QEMU after normal exit (PID $linger_pid reaped by EXIT trap)"
fi

# 3b. External SIGTERM sent to run.sh mid-flight reaps stubborn fake QEMU.
pid_rec_sig="$scratch/pid-sig"
rm -f "$pid_rec_sig"
env \
    FAKE_PID_RECORD="$pid_rec_sig" \
    QEMU_OP6="$fake_hang_qemu" \
    EMU_OP6_UBOOT_GZ_DTB="$fake_uboot_gz_dtb" \
    EMU_OP6_UBOOT_SHA256="$fake_uboot_sha" \
    EMU_OP6_ESP_BASE="$fake_esp" \
    EMU_OP6_OUT_ROOT="$out_root" \
    EMU_OP6_KILL_AFTER=1 \
    "$run_sh" "$fake_payload" \
    --run-id test-no-orphan-sigterm \
    --marker "NEVER_REACHED" \
    --timeout 30 >"$scratch/sigterm.log" 2>&1 &
harness_bg_pid=$!

for _ in $(seq 1 50); do
    [[ -s "$pid_rec_sig" ]] && break
    sleep 0.05
done
sig_qemu_pid=$(cat "$pid_rec_sig" 2>/dev/null || true)
kill -TERM "$harness_bg_pid" 2>/dev/null || true
wait "$harness_bg_pid" 2>/dev/null || true

if [[ -z "$sig_qemu_pid" ]]; then
    bad "interrupted fake QEMU recorded its PID" "no PID found"
elif kill -0 "$sig_qemu_pid" 2>/dev/null; then
    kill -KILL "$sig_qemu_pid" 2>/dev/null || true
    bad "no orphan QEMU after SIGTERM to run.sh" "PID $sig_qemu_pid still alive"
else
    ok "no orphan QEMU after SIGTERM to run.sh (PID $sig_qemu_pid reaped by trap)"
fi

# ==============================================================================
# 4. Receipt written and ESP image copy verified
# ==============================================================================
printf '\n4. receipt.tsv and ESP copy integrity\n'

receipt_file="$out_root/test-order-pass/receipt.tsv"
esp_copied="$out_root/test-order-pass/esp.img"
serial_captured="$out_root/test-order-pass/serial.log"

if [[ -s "$receipt_file" ]]; then
    ok "receipt.tsv written for passing run"
else
    bad "receipt.tsv written for passing run" "missing $receipt_file"
fi

if [[ -s "$out_root/test-timeout-kill/receipt.tsv" ]]; then
    ok "receipt.tsv also written on timed-out run"
else
    bad "receipt.tsv also written on timed-out run" "missing timeout receipt.tsv"
fi

expected_emu_sha=$(sha256sum "$fake_uboot_qemu" | awk '{print $1}')
expected_img_sha=$(sha256sum "$esp_copied" | awk '{print $1}')
expected_serial_sha=$(sha256sum "$serial_captured" | awk '{print $1}')

rec_emu_sha=$(awk -F'\t' '$1 == "emulator" {print $2}' "$receipt_file")
rec_pay_sha=$(awk -F'\t' '$1 == "payload" {print $2}' "$receipt_file")
rec_img_sha=$(awk -F'\t' '$1 == "image" {print $2}' "$receipt_file")
rec_ser_sha=$(awk -F'\t' '$1 == "serial_log" {print $2}' "$receipt_file")

expect_rc "receipt.tsv records exact emulator SHA-256" "$expected_emu_sha" "$rec_emu_sha" "$receipt_file"
expect_rc "receipt.tsv records exact payload SHA-256" "$fake_payload_sha" "$rec_pay_sha" "$receipt_file"
expect_rc "receipt.tsv records exact ESP copy SHA-256" "$expected_img_sha" "$rec_img_sha" "$receipt_file"
expect_rc "receipt.tsv records exact serial.log SHA-256" "$expected_serial_sha" "$rec_ser_sha" "$receipt_file"

# Verify payload was actually placed onto the ESP copy at /EFI/BOOT/BOOTAA64.EFI
extracted_efi="$scratch/extracted-bootaa64.efi"
"$mcopy_bin" -o -i "${esp_copied}@@1048576" ::/EFI/BOOT/BOOTAA64.EFI "$extracted_efi"
extracted_sha=$(sha256sum "$extracted_efi" | awk '{print $1}')
expect_rc "ESP copy contains payload at /EFI/BOOT/BOOTAA64.EFI" "$fake_payload_sha" "$extracted_sha"

# Verify base ESP image was not modified
fake_esp_after_sha=$(sha256sum "$fake_esp" | awk '{print $1}')
expect_rc "base ESP image remains unmodified" "$fake_esp_base_sha" "$fake_esp_after_sha"

# ==============================================================================
# 5. --qemu-arg passthrough
# ==============================================================================
printf '\n5. --qemu-arg passthrough\n'

fake_args_qemu="$scratch/fake-qemu-args.sh"
make_fake_qemu "$fake_args_qemu" '
for arg in "$@"; do
    printf "ARG<%s>\r\n" "$arg"
done
printf "ARGS-DONE\r\n"
'

rc=0
out=$(QEMU_OP6="$fake_args_qemu" run_harness "$run_sh" "$fake_payload" \
    --run-id test-qemu-arg \
    --qemu-arg -qmp --qemu-arg "unix:/tmp/fake qmp.sock,server=on,wait=off" \
    --marker "ARG<-d>" \
    --marker "ARG<-qmp>" \
    --marker "ARG<unix:/tmp/fake qmp.sock,server=on,wait=off>" \
    --marker "ARGS-DONE" \
    --timeout 5 2>&1) || rc=$?
expect_rc "extra QEMU arguments are appended in order, spaces intact" 0 "$rc" "$out"

rc=0
out=$(QEMU_OP6="$fake_args_qemu" run_harness "$run_sh" "$fake_payload" \
    --run-id test-qemu-arg-none \
    --marker "ARGS-DONE" \
    --timeout 5 2>&1) || rc=$?
if (( rc == 0 )) && ! grep -aq "ARG<-qmp>" "$out_root/test-qemu-arg-none/serial.log"; then
    ok "no extra QEMU arguments without --qemu-arg"
else
    bad "no extra QEMU arguments without --qemu-arg" "rc=$rc $out"
fi

printf '\nSummary: %d passed, %d failed\n' "$pass" "$fail"
(( fail == 0 ))
