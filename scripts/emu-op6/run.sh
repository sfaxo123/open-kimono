#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
#
# MIT test harness that invokes the external GPL qemu-oneplus6 binary (no QEMU
# code copied), boots the phone U-Boot on a per-run copy of the ESP image with
# the payload staged at /EFI/BOOT/BOOTAA64.EFI, drives the U-Boot serial prompt
# to bootefi the payload, captures serial output to out/emu-op6/<run-id>/serial.log,
# asserts markers in order, enforces a timeout with kill-after and exit-trap
# process reaping, and writes out/emu-op6/<run-id>/receipt.tsv.
set -euo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)

usage() {
    cat >&2 <<'EOF'
Usage: scripts/emu-op6/run.sh <payload> [--marker STR ...] [--qmp-script FILE] [--timer-ppi-fix] [--timeout S] [--run-id ID] [--qemu-arg ARG ...]

  --qemu-arg ARG        Append one extra argument to the QEMU command line (repeatable),
                        for example a QMP socket for input injection.

Environment overrides:
  QEMU_OP6              Path to qemu-system-aarch64 (default: ~/Developer/forks/qemu-oneplus6/build/qemu-system-aarch64)
  EMU_OP6_UBOOT_GZ_DTB  Path to u-boot-nodtb.bin.gz-dtb
  EMU_OP6_UBOOT_SHA256  Expected SHA-256 of u-boot-nodtb.bin.gz-dtb
  EMU_OP6_ESP_BASE      Path to base ESP image (op6-boot-esp.img)
  EMU_OP6_OUT_ROOT      Base output directory (default: <repo>/out/emu-op6)
  EMU_OP6_RUN_ID        Explicit run identifier
  EMU_OP6_KILL_AFTER    Seconds between SIGTERM and SIGKILL on timeout/reap (default: 5)
EOF
}

payload=""
markers=()
timeout_s=45
run_id="${EMU_OP6_RUN_ID:-}"
qmp_script=""
timer_ppi_fix=0
qemu_extra=()

while [[ $# -gt 0 ]]; do
    case "$1" in
        --marker)
            [[ $# -ge 2 ]] || { printf 'error: --marker requires an argument\n' >&2; usage; exit 2; }
            markers+=("$2")
            shift 2
            ;;
        --marker=*)
            markers+=("${1#--marker=}")
            shift
            ;;
        --qmp-script)
            [[ $# -ge 2 ]] || { printf 'error: --qmp-script requires a file\n' >&2; exit 2; }
            qmp_script="$2"
            shift 2
            ;;
        --qmp-script=*)
            qmp_script="${1#--qmp-script=}"
            shift
            ;;
        --timer-ppi-fix)
            timer_ppi_fix=1
            shift
            ;;
        --timeout)
            [[ $# -ge 2 ]] || { printf 'error: --timeout requires an argument\n' >&2; usage; exit 2; }
            timeout_s="$2"
            shift 2
            ;;
        --timeout=*)
            timeout_s="${1#--timeout=}"
            shift
            ;;
        --run-id)
            [[ $# -ge 2 ]] || { printf 'error: --run-id requires an argument\n' >&2; usage; exit 2; }
            run_id="$2"
            shift 2
            ;;
        --run-id=*)
            run_id="${1#--run-id=}"
            shift
            ;;
        --qemu-arg)
            [[ $# -ge 2 ]] || { printf 'error: --qemu-arg requires an argument\n' >&2; usage; exit 2; }
            qemu_extra+=("$2")
            shift 2
            ;;
        --qemu-arg=*)
            qemu_extra+=("${1#--qemu-arg=}")
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        --)
            shift
            break
            ;;
        -*)
            printf 'error: unknown option: %s\n' "$1" >&2
            usage
            exit 2
            ;;
        *)
            if [[ -z "$payload" ]]; then
                payload="$1"
                shift
            else
                printf 'error: unexpected positional argument: %s\n' "$1" >&2
                usage
                exit 2
            fi
            ;;
    esac
done

if [[ -z "$payload" && $# -gt 0 ]]; then
    payload="$1"
    shift
fi

[[ -n "$payload" ]] || { printf 'error: missing <payload> argument\n' >&2; usage; exit 2; }
[[ -f "$payload" ]] || { printf 'error: payload not found: %s\n' "$payload" >&2; exit 1; }
if [[ -n "$qmp_script" ]]; then
    [[ -f "$qmp_script" ]] || { printf 'error: QMP script not found: %s\n' "$qmp_script" >&2; exit 1; }
    qmp_script=$(realpath "$qmp_script")
fi
[[ "$timeout_s" =~ ^[0-9]+$ ]] && (( timeout_s > 0 )) || {
    printf 'error: --timeout must be a positive integer, got: %s\n' "$timeout_s" >&2
    exit 2
}

qemu_op6="${QEMU_OP6:-$HOME/Developer/forks/qemu-oneplus6/build/qemu-system-aarch64}"
[[ -x "$qemu_op6" ]] || { printf 'error: emulator binary not executable: %s\n' "$qemu_op6" >&2; exit 1; }

phone_boot_dir="${EMU_OP6_PHONE_BOOT:-$HOME/artifacts/handoff/oneplus-redox-20261024/phone-boot}"
uboot_gz_dtb="${EMU_OP6_UBOOT_GZ_DTB:-$phone_boot_dir/u-boot-nodtb.bin.gz-dtb}"
uboot_expected_sha="${EMU_OP6_UBOOT_SHA256:-c50fec59e1c6bedaa5077b324615bf3eb001315144501da850d007b0a7619662}"
esp_base="${EMU_OP6_ESP_BASE:-$phone_boot_dir/op6-boot-esp.img}"
kill_after="${EMU_OP6_KILL_AFTER:-5}"

[[ -f "$uboot_gz_dtb" ]] || { printf 'error: U-Boot payload not found: %s\n' "$uboot_gz_dtb" >&2; exit 1; }
[[ -f "$esp_base" ]] || { printf 'error: base ESP image not found: %s\n' "$esp_base" >&2; exit 1; }

if [[ -z "$run_id" ]]; then
    run_id="$(date -u +%Y%m%dT%H%M%SZ)-$$"
fi

out_root="${EMU_OP6_OUT_ROOT:-$repo/out/emu-op6}"
run_dir="$out_root/$run_id"
mkdir -p "$run_dir"

payload_abs=$(cd "$(dirname "$payload")" && pwd)/$(basename "$payload")
qemu_abs=$(cd "$(dirname "$qemu_op6")" && pwd)/$(basename "$qemu_op6")
esp_copy="$run_dir/esp.img"
uboot_dtb="$run_dir/oneplus6-u-boot-appended.dtb"
uboot_ram_img="$run_dir/oneplus6-u-boot-with-ram.bin"
serial_log="$run_dir/serial.log"
unimp_log="$run_dir/unimp.log"
qemu_err="$run_dir/qemu.stderr.log"
pidfile="$run_dir/qemu.pid"
serial_in="$run_dir/serial.in"
receipt="$run_dir/receipt.tsv"
socket_dir=$(realpath --relative-to="$PWD" "$run_dir")
qmp_sock="$socket_dir/qmp.sock"
qmp_log="$run_dir/qmp.log"
qtest_sock="$socket_dir/qtest.sock"

# 1. Unpack phone U-Boot and patch /memory@80000000/reg to 4 GiB as run-m7-uboot.py does.
python3 - "$uboot_gz_dtb" "$uboot_expected_sha" "$uboot_dtb" "$uboot_ram_img" "$timer_ppi_fix" <<'PY'
import hashlib
import pathlib
import subprocess
import sys
import zlib

src = pathlib.Path(sys.argv[1])
expected_sha = sys.argv[2]
dtb_path = pathlib.Path(sys.argv[3])
out_img = pathlib.Path(sys.argv[4])
timer_ppi_fix = sys.argv[5] == "1"

data = src.read_bytes()
actual_sha = hashlib.sha256(data).hexdigest()
if expected_sha and actual_sha != expected_sha:
    raise SystemExit(f"phone U-Boot payload SHA-256 mismatch: expected {expected_sha}, got {actual_sha}")

z = zlib.decompressobj(31)
binary = z.decompress(data) + z.flush()
if not z.unused_data:
    raise SystemExit("phone U-Boot payload has no appended DTB")
dtb_path.write_bytes(z.unused_data)
subprocess.run(
    ["fdtput", "-t", "x", str(dtb_path), "/memory@80000000", "reg", "0", "0x80000000", "1", "0"],
    check=True,
)
# M10's emulator-only ABL stand-in: this U-Boot has no SDM845 RPMh power
# domains, so GCC probe must not depend on one in the disposable DTB copy.
gcc = "/soc@0/clock-controller@100000"
if subprocess.run(["fdtget", str(dtb_path), gcc, "power-domains"],
                  stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode == 0:
    subprocess.run(["fdtput", "-d", str(dtb_path), gcc, "power-domains"], check=True)
if timer_ppi_fix:
    original = subprocess.check_output(
        ["fdtget", "-t", "x", str(dtb_path), "/timer", "interrupts"], text=True
    ).strip()
    if original != "1 1 8 1 2 8 1 3 8 1 0 8":
        raise SystemExit(f"unexpected phone timer PPIs: {original}")
    # QEMU oneplus6.c wires architectural timer INTIDs 29/30/27/26.
    # Only this disposable emulator DTB receives the matching PPI numbers.
    subprocess.run(
        ["fdtput", "-t", "x", str(dtb_path), "/timer", "interrupts",
         "1", "d", "8", "1", "e", "8", "1", "b", "8", "1", "a", "8"],
        check=True,
    )
out_img.write_bytes(binary + dtb_path.read_bytes())
PY

# 2. Copy the ESP image and stage <payload> onto the copy at /EFI/BOOT/BOOTAA64.EFI.
cp -f "$esp_base" "$esp_copy"

mcopy_bin=""
if command -v mcopy >/dev/null 2>&1; then
    mcopy_bin="$(command -v mcopy)"
elif [[ -x "$phone_boot_dir/host-tools/usr/bin/mcopy" ]]; then
    mcopy_bin="$phone_boot_dir/host-tools/usr/bin/mcopy"
fi

esp_offset="${EMU_OP6_ESP_OFFSET:-1048576}"
if [[ -n "$mcopy_bin" ]]; then
    "$mcopy_bin" -m -o -i "${esp_copy}@@${esp_offset}" "$payload_abs" ::/EFI/BOOT/BOOTAA64.EFI
else
    sudo -n true || { printf 'error: neither mcopy nor passwordless sudo available to stage ESP\n' >&2; exit 1; }
    mnt="$run_dir/esp-mnt"
    mkdir -p "$mnt"
    loop_dev=$(sudo -n losetup -P -f --show "$esp_copy")
    sudo -n mount "${loop_dev}p1" "$mnt"
    sudo -n mkdir -p "$mnt/EFI/BOOT"
    sudo -n cp -f "$payload_abs" "$mnt/EFI/BOOT/BOOTAA64.EFI"
    sync
    sudo -n umount "$mnt"
    sudo -n losetup -d "$loop_dev"
    rmdir "$mnt"
fi

# 3. Compute artifact SHA-256 hashes and write receipt.tsv (updated again on exit).
emu_sha=$(sha256sum "$qemu_abs" | awk '{print $1}')
payload_sha=$(sha256sum "$payload_abs" | awk '{print $1}')
image_sha=$(sha256sum "$esp_copy" | awk '{print $1}')
uboot_sha=$(sha256sum "$uboot_ram_img" | awk '{print $1}')
uboot_src_sha=$(sha256sum "$uboot_gz_dtb" | awk '{print $1}')

write_receipt() {
    local serial_sha="none"
    if [[ -f "$serial_log" ]]; then
        serial_sha=$(sha256sum "$serial_log" | awk '{print $1}')
    fi
    cat >"$receipt" <<EOF
artifact	sha256	path
emulator	$emu_sha	$qemu_abs
payload	$payload_sha	$payload_abs
image	$image_sha	$esp_copy
uboot_image	$uboot_sha	$uboot_ram_img
uboot_source	$uboot_src_sha	$uboot_gz_dtb
serial_log	$serial_sha	$serial_log
EOF
}

: >"$serial_log"
write_receipt

# 4. Bounded QEMU execution with exit trap that reaps both wrapper and QEMU process.
wrapper_pid=""
driver_pid=""
cleanup() {
    local rc=$?
    if [[ -n "$driver_pid" ]] && kill -0 "$driver_pid" 2>/dev/null; then
        kill -TERM "$driver_pid" 2>/dev/null || true
        wait "$driver_pid" 2>/dev/null || true
    fi
    driver_pid=""
    exec 3>&- 2>/dev/null || true
    rm -f "$serial_in"

    local qpid=""
    if [[ -f "$pidfile" ]]; then
        qpid=$(cat "$pidfile" 2>/dev/null || true)
    fi

    local children=()
    if [[ -n "$wrapper_pid" ]]; then
        while IFS= read -r cpid; do
            [[ -n "$cpid" ]] && children+=("$cpid")
        done < <(pgrep -P "$wrapper_pid" 2>/dev/null || true)
    fi

    if [[ -n "$wrapper_pid" ]] && kill -0 "$wrapper_pid" 2>/dev/null; then
        kill -TERM "$wrapper_pid" 2>/dev/null || true
    fi
    for cpid in "${children[@]}"; do
        kill -TERM "$cpid" 2>/dev/null || true
    done
    if [[ -n "$qpid" ]] && kill -0 "$qpid" 2>/dev/null; then
        kill -TERM "$qpid" 2>/dev/null || true
    fi

    local ticks=$(( kill_after * 10 ))
    (( ticks < 1 )) && ticks=1
    for (( i = 0; i < ticks; i++ )); do
        local alive=0
        if [[ -n "$wrapper_pid" ]] && kill -0 "$wrapper_pid" 2>/dev/null; then alive=1; fi
        if [[ -n "$qpid" ]] && kill -0 "$qpid" 2>/dev/null; then alive=1; fi
        for cpid in "${children[@]}"; do
            if kill -0 "$cpid" 2>/dev/null; then alive=1; fi
        done
        (( alive == 0 )) && break
        sleep 0.1
    done

    if [[ -n "$wrapper_pid" ]] && kill -0 "$wrapper_pid" 2>/dev/null; then
        kill -KILL "$wrapper_pid" 2>/dev/null || true
    fi
    for cpid in "${children[@]}"; do
        if kill -0 "$cpid" 2>/dev/null; then
            kill -KILL "$cpid" 2>/dev/null || true
        fi
    done
    if [[ -n "$qpid" ]] && kill -0 "$qpid" 2>/dev/null; then
        kill -KILL "$qpid" 2>/dev/null || true
    fi

    if [[ -n "$wrapper_pid" ]]; then
        wait "$wrapper_pid" 2>/dev/null || true
        wrapper_pid=""
    fi

    write_receipt
    return "$rc"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

rm -f "$pidfile" "$unimp_log" "$qemu_err" "$serial_in"
rm -f "$qmp_sock" "$qmp_log" "$qtest_sock"
mkfifo "$serial_in"
exec 3<>"$serial_in"
qmp_args=()
if [[ -n "$qmp_script" ]]; then
    qmp_args=(-qmp "unix:$qmp_sock,server=on,wait=off"
              -qtest "unix:$qtest_sock,server=on,wait=off")
fi

timeout --foreground --signal=TERM --kill-after="$kill_after" "$timeout_s" \
    "$qemu_abs" -M oneplus6 -m 4G \
    -display none -monitor none -serial stdio -no-reboot \
    "${qmp_args[@]}" \
    -pidfile "$pidfile" \
    -device "loader,file=$uboot_ram_img,addr=0x80008000,cpu-num=0,force-raw=on" \
    -drive "file=$esp_copy,if=none,id=ufs0,format=raw" \
    -d unimp -D "$unimp_log" \
    "${qemu_extra[@]}" \
    <"$serial_in" >"$serial_log" 2>"$qemu_err" &
wrapper_pid=$!

# 5. Drive U-Boot serial prompt and monitor ordered markers in serial.log.
# Run the serial monitor in the background and `wait` on it so bash's INT/TERM
# traps fire immediately if run.sh is signalled while waiting.
driver_rc=0
python3 - "$serial_log" "$wrapper_pid" "$timeout_s" "$qmp_script" "$qmp_sock" "$qmp_log" "$qtest_sock" "${markers[@]}" >&3 <<'PY' &
import json
import os
import socket
import sys
import time

serial_path = sys.argv[1]
wrapper_pid = int(sys.argv[2])
timeout_s = float(sys.argv[3])
script_path, qmp_socket, qmp_log, qtest_socket = sys.argv[4:8]
markers = [m.encode("utf-8", errors="surrogateescape") for m in sys.argv[8:]]
script = None
if script_path:
    with open(script_path, encoding="utf-8") as f:
        script = json.load(f)
    if not isinstance(script, dict) or not isinstance(script.get("marker"), str) or not isinstance(script.get("commands"), list):
        raise SystemExit("error: QMP script needs marker string and commands array")
    for command in script["commands"]:
        if not isinstance(command, dict) or not (isinstance(command.get("execute"), str) or "qtest_set_irq_in" in command):
            raise SystemExit("error: each script command needs execute or qtest_set_irq_in")

def run_qmp(commands):
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as sock, open(qmp_log, "w", encoding="utf-8") as log:
        sock.settimeout(5)
        sock.connect(qmp_socket)
        stream = sock.makefile("rwb", buffering=0)
        greeting = json.loads(stream.readline())
        if "QMP" not in greeting:
            raise RuntimeError(f"missing QMP greeting: {greeting}")
        for command in [{"execute": "qmp_capabilities"}, *commands]:
            delay = command.get("delay_ms", 0)
            if not isinstance(delay, (int, float)) or delay < 0:
                raise RuntimeError("delay_ms must be nonnegative")
            time.sleep(delay / 1000)
            if "qtest_set_irq_in" in command:
                item = command["qtest_set_irq_in"]
                if not isinstance(item, dict) or item.get("line") not in ("vol-down", "vol-up", "pwrkey") or item.get("level") not in (0, 1):
                    raise RuntimeError("qtest_set_irq_in needs a named button line and level 0 or 1")
                with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as test_sock:
                    test_sock.settimeout(5)
                    test_sock.connect(qtest_socket)
                    test_sock.sendall(f"set_irq_in /machine/spmi {item['line']} 0 {item['level']}\n".encode())
                    reply = test_sock.makefile("rb").readline().decode().strip()
                log.write(json.dumps({"qtest_set_irq_in": item, "response": reply}) + "\n")
                log.flush()
                if reply != "OK":
                    raise RuntimeError(f"QTest set_irq_in failed: {reply}")
                continue
            request = {k: v for k, v in command.items() if k != "delay_ms"}
            stream.write((json.dumps(request) + "\n").encode())
            while True:
                response = json.loads(stream.readline())
                if "event" not in response:
                    break
            log.write(json.dumps({"request": request, "response": response}) + "\n")
            log.flush()
            if "error" in response:
                raise RuntimeError(f"QMP {request['execute']} failed: {response['error']}")

bootefi_cmd = (
    b"scsi scan; load scsi 0:1 0xa0000000 /EFI/BOOT/BOOTAA64.EFI; "
    b"bootefi 0xa0000000 ${fdtcontroladdr}\r"
)

def send_paced(data: bytes, chunk_size: int = 16, delay_s: float = 0.005) -> None:
    for i in range(0, len(data), chunk_size):
        sys.stdout.buffer.write(data[i : i + chunk_size])
        sys.stdout.buffer.flush()
        if i + chunk_size < len(data):
            time.sleep(delay_s)

def pid_alive(pid: int) -> bool:
    try:
        os.kill(pid, 0)
        return True
    except ProcessLookupError:
        return False
    except PermissionError:
        return True

def check_markers(buf: bytes, markers_list: list[bytes]):
    """Return (all_matched, out_of_order_msg, missing_idx)."""
    pos = 0
    for idx, m in enumerate(markers_list):
        found = buf.find(m, pos)
        if found < 0:
            earlier = buf.find(m, 0)
            if earlier >= 0 and idx > 0:
                prev_m = markers_list[idx - 1].decode("utf-8", errors="replace")
                cur_m = m.decode("utf-8", errors="replace")
                return (
                    False,
                    f"marker #{idx + 1} ({cur_m!r}) appeared only before marker #{idx} ({prev_m!r})",
                    idx,
                )
            return (False, None, idx)
        pos = found + len(m)
    return (True, None, -1)

continued = False
selected_shell = False
sent_bootefi = False
qmp_done = script is None
bootefi_sent_at = 0.0
deadline = time.monotonic() + timeout_s + 2.0

while time.monotonic() < deadline:
    try:
        with open(serial_path, "rb") as f:
            buf = f.read()
    except FileNotFoundError:
        buf = b""

    if not continued and b"Press any key to continue..." in buf:
        sys.stdout.buffer.write(b"\r")
        sys.stdout.buffer.flush()
        continued = True

    if continued and not selected_shell and b"*** U-Boot Boot Menu ***" in buf:
        sys.stdout.buffer.write(b"5")
        sys.stdout.buffer.flush()
        selected_shell = True

    if not sent_bootefi and b"=> " in buf:
        send_paced(bootefi_cmd)
        sent_bootefi = True
        bootefi_sent_at = time.monotonic()

    if not qmp_done and script["marker"].encode() in buf:
        try:
            run_qmp(script["commands"])
        except (OSError, ValueError, RuntimeError) as error:
            sys.stderr.write(f"error: {error}\n")
            sys.exit(1)
        qmp_done = True

    matched, ooo_msg, _ = check_markers(buf, markers)
    if matched and qmp_done:
        # If U-Boot prompt was reached, allow the driven bootefi command to echo and execute.
        if not sent_bootefi or (
            b"bootefi 0xa0000000" in buf and (
                buf.count(b"=> ") >= 2 or (time.monotonic() - bootefi_sent_at) >= 0.4
            )
        ) or (time.monotonic() - bootefi_sent_at) >= 1.0:
            sys.exit(0)

    if not pid_alive(wrapper_pid):
        break

    time.sleep(0.05)

try:
    with open(serial_path, "rb") as f:
        buf = f.read()
except FileNotFoundError:
    buf = b""

matched, ooo_msg, missing_idx = check_markers(buf, markers)
if matched and qmp_done:
    sys.exit(0)

if ooo_msg:
    sys.stderr.write(f"error: marker order assertion failed: {ooo_msg}\n")
    sys.exit(1)

if missing_idx >= 0:
    missing_str = markers[missing_idx].decode("utf-8", errors="replace")
    sys.stderr.write(
        f"error: marker #{missing_idx + 1} ({missing_str!r}) not found in order in {serial_path}\n"
    )
    sys.exit(1)

sys.exit(1)
PY
driver_pid=$!
wait "$driver_pid" || driver_rc=$?
driver_pid=""

if (( driver_rc != 0 )); then
    # Check whether the timeout wrapper exited with 124/137 (timeout).
    if [[ -n "$wrapper_pid" ]] && ! kill -0 "$wrapper_pid" 2>/dev/null; then
        wrc=0
        wait "$wrapper_pid" 2>/dev/null || wrc=$?
        wrapper_pid=""
        if (( wrc == 124 || wrc == 137 )); then
            printf 'error: QEMU timed out after %ss (serial: %s)\n' "$timeout_s" "$serial_log" >&2
        fi
    fi
    exit "$driver_rc"
fi

printf 'run_id=%s\nserial=%s\nreceipt=%s\n' "$run_id" "$serial_log" "$receipt"
