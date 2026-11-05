#!/usr/bin/env bash
# E1-NET-NATIVE-PROBE driver (host-repair revision 2, Codex-reviewed).
# Stage the native net-probe binary into an OWNED copy of e1-live.iso, boot it
# (max 2 boots x <=90s, EXPLICIT `boot` argument only — no default boot) with
# the RTL8139/INTx recipe, and require this run's nonce at the host peer + in
# the directional guest->host TCP payload + WRITE_OK from the native app.
# Owned paths only; forks and the original ISO are read-only inputs.
# Tests live in e1-net-native-probe-tests.sh and call the SAME evaluate_run.
set -euo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
out="$repo/out/e1net-native"
forks=${REDOX_FORKS:-$HOME/Developer/forks/redox}
redox="$forks/redox"
orig_iso="$redox/build/aarch64/desktop-minimal/e1-live.iso"
ORIG_ISO_SHA=c02012eaa182800cc7ed937e0e3ebf7b436f2af3e9dacb5c57bb00c89df361bc
dtb="$repo/out/qemu-demo/simplefb-virt.dtb"   # existing input, read-only
binary="$out/redox/aarch64-unknown-redox/release/net-probe"
fstool="$redox/build/fstools/bin/redoxfs"
pcap_tool="$repo/scripts/qemu-demo/e1-net-native-probe-pcap.py"

run_timeout=${RUN_TIMEOUT:-90}
port=${PORT:-4766}
nonce="e1np-$(date +%s)-$RANDOM"
guest_ip=10.0.2.15; host_ip=10.0.2.2
guest_endpoint="$host_ip:$port"          # slirp host alias; NEVER 127.0.0.1 in guest
mkdir -p "$out"

PEER_PGID=""; qpid=""; rfs_pid=""
reap_group() {  # bounded TERM -> KILL -> reap; never waits forever
    local pg=$1 i
    [[ -z "$pg" ]] && return 0
    kill -TERM -- "-$pg" 2>/dev/null || true
    for i in 1 2 3; do kill -0 -- "-$pg" 2>/dev/null || { wait "$pg" 2>/dev/null || true; return 0; }; sleep 1; done
    kill -KILL -- "-$pg" 2>/dev/null || true
    for i in 1 2 3; do kill -0 -- "-$pg" 2>/dev/null || break; sleep 1; done
    wait "$pg" 2>/dev/null || true
}
cleanup() {   # reap writer even when its mount never became ready
    reap_group "$PEER_PGID"; PEER_PGID=""
    reap_group "$qpid"; qpid=""
    if mountpoint -q "$out/iso-mount" 2>/dev/null; then
        timeout --kill-after=2 5 fusermount3 -u "$out/iso-mount" 2>/dev/null || true
    fi
    reap_group "$rfs_pid"; rfs_pid=""
}
trap cleanup EXIT
# INT/TERM must NOT fall through to the next attempt: cleanup then hard exit.
trap 'cleanup; exit 130' INT
trap 'cleanup; exit 143' TERM

validate_env() {
    [[ "$run_timeout" =~ ^[0-9]+$ ]] || { printf 'RUN_TIMEOUT must be numeric, got %s\n' "$run_timeout" >&2; return 1; }
    # lower bound keeps the in-loop sleeps valid: 25 s boot wait + (run_timeout-35) s drain
    (( run_timeout >= 40 && run_timeout <= 90 )) || { printf 'RUN_TIMEOUT must be 40..90, got %s\n' "$run_timeout" >&2; return 1; }
    [[ "$port" =~ ^[0-9]+$ ]] && (( port >= 1024 && port <= 65535 )) || { printf 'PORT must be 1024..65535, got %s\n' "$port" >&2; return 1; }
}

# Host peer: TWO addresses only (the old `-` made three and socat refused).
# Records the first line, then sends ACK-<nonce> back. Own process group.
start_peer() {  # $1=run_dir $2=nonce $3=listen_port
    setsid timeout $((run_timeout + 5)) socat \
        "TCP-LISTEN:$3,bind=127.0.0.1,reuseaddr" \
        SYSTEM:"head -n 1 > '$1/host-recv.txt'; printf 'ACK-$2\n'" \
        >"$1/peer.log" 2>&1 &
    PEER_PGID=$!
}

extract_pcap() {  # $1=pcap $2=out_file $3=guest_ip $4=host_ip $5=port
    python3 "$pcap_tool" "$1" "$2" "$3" "$4" "$5"
}

# A signature is a fact about the capture, never a conclusion about cause.
classify() {  # $1=serial -> one signature word
    local s=$1
    if grep -qE 'SETUP PANIC|READ_BLOCK: INCORRECT HASH' "$s" 2>/dev/null; then echo image_integrity; return; fi
    # Boot services may fault before login; that is not evidence the probe ran.
    # Attribute a fault to the probe window only after its own start marker (or
    # an explicitly probe-prefixed diagnostic in a fixture).
    if grep -qE '^net-probe:.*(exception|page fault|EL0|kernel panic)' "$s" 2>/dev/null ||
       { grep -q 'net-probe: start' "$s" 2>/dev/null &&
         sed -n '/net-probe: start/,$p' "$s" | grep -qE 'FATAL:|exception|page fault|kernel panic'; }; then
        echo crash; return
    fi
    if grep -q 'net-probe: READ_ACK' "$s" 2>/dev/null; then echo read_ack; return; fi
    if grep -q 'net-probe: WRITE_OK' "$s" 2>/dev/null; then echo write_ok; return; fi
    if grep -q 'net-probe: CONNECT_ERROR' "$s" 2>/dev/null; then echo connect_error; return; fi
    if grep -q 'net-probe: CONNECT_OK' "$s" 2>/dev/null; then echo handshake_only; return; fi
    if grep -q 'redox login:' "$s" 2>/dev/null; then echo login_only; return; fi
    if grep -q 'Redox Bootloader Environment Editor' "$s" 2>/dev/null; then echo bootloader_wait; return; fi
    echo unknown
}
# FULL = this run's nonce as the EXACT peer protocol line AND as the EXACT
# payload line in the directional capture (nonce inside ONE real TCP payload;
# a nonce split across packets is conservatively REFUSED — no reassembly is
# claimed) AND the app's WRITE_OK. prefix<nonce>suffix never matches (-Fx).
accept() {  # $1=nonce $2=recv $3=pcap-payload $4=signature -> verdict word
    local nonce=$1 recv=$2 pay=$3 sig=$4
    [[ -n "$nonce" && "$nonce" != 'e1np-missing' ]] || { echo REJECT_EMPTY_NONCE; return; }
    case "$sig" in
        image_integrity) echo REJECT_IMAGE_INTEGRITY; return;;
        crash) echo REJECT_APP_CRASH; return;;
    esac
    local peer=0 pcap=0
    [[ -s "$recv" ]] && grep -qFx -- "$nonce" "$recv" && peer=1
    [[ -s "$pay" ]] && grep -qFx -- "$nonce" "$pay" && pcap=1
    if (( peer && pcap )) && [[ "$sig" == write_ok || "$sig" == read_ack ]]; then echo FULL; return; fi
    if (( peer == 0 && pcap == 0 )) && [[ "$sig" == handshake_only ]]; then echo PARTIAL; return; fi
    echo REJECT
}
# THE boot-result evaluator: extraction FIRST, then classify, then accept.
# The boot loop and the tests both call this — the ordering cannot drift.
evaluate_run() {  # $1=run_dir $2=nonce $3=port -> verdict on stdout
    local rd=$1 n=$2 p=$3
    extract_pcap "$rd/pcap.pcap" "$rd/pcap-payload.txt" "$guest_ip" "$host_ip" "$p"
    local sig; sig=$(classify "$rd/serial.log")
    # READ_ACK is additional evidence, never a substitute for the promised
    # native WRITE_OK marker. Keep this check in the real boot evaluator.
    if [[ "$sig" == read_ack ]] && ! grep -q '^net-probe: WRITE_OK ' "$rd/serial.log"; then
        echo REJECT_MISSING_WRITE_OK; return
    fi
    accept "$n" "$rd/host-recv.txt" "$rd/pcap-payload.txt" "$sig"
}

# Fresh-process tree walk after the FUSE writer has exited: cmp through the
# same live mount cannot detect corruption introduced at unmount/commit.
verify_image() {  # read-only input, disposable owned destination
    local image=$1 scratch rc=0
    scratch=$(mktemp "$out/integrity.XXXXXX.img")
    truncate -s 320M "$scratch"
    timeout --kill-after=5 40 "$redox/build/fstools/bin/redoxfs-clone" \
        "$image" "$scratch" >"$scratch.log" 2>&1 || rc=$?
    rm -f -- "$scratch"
    if (( rc != 0 )); then
        printf 'BLOCKER: fresh image tree walk failed (%s); log %s.log; refusing boot\n' "$rc" "$scratch" >&2
        return 1
    fi
}

if [[ "${1:-}" == lib ]]; then return 0 2>/dev/null || exit 0; fi

case "${1:-}" in
    boot) ;;
    *) printf 'usage: %s boot   (explicit only; tests: e1-net-native-probe-tests.sh)\n' "$0" >&2; exit 2 ;;
esac

# ------------------------------------------------------------------- boot ---
validate_env || exit 2

# 0. Original ISO integrity: verify before and after the whole phase.
iso_sha=$(sha256sum "$orig_iso" | cut -d ' ' -f1)
[[ "$iso_sha" == "$ORIG_ISO_SHA" ]] || { echo "ORIGINAL ISO hash mismatch: $iso_sha" >&2; exit 1; }
printf 'original iso sha256 before: %s (matches ledger)\n' "$iso_sha"

# 1. Owned copy + RedoxFS staging of net-probe into usr/bin (owned copy only).
# SKIP_STAGE=1 is a host-only test hook (used by the interruption regression);
# it skips image writes but never weakens verdicts.
if [[ "${SKIP_STAGE:-0}" != 1 ]]; then
myiso="$out/e1net-probe.iso"
cp -a --reflink=auto "$orig_iso" "$myiso"
test -x "$fstool" || { echo "BLOCKER: no existing RedoxFS tool at $fstool" >&2; exit 3; }
mkdir -p "$out/iso-mount"
setsid "$fstool" --no-daemon "$myiso" "$out/iso-mount" >/dev/null 2>&1 & rfs_pid=$!
for _ in $(seq 1 20); do mountpoint -q "$out/iso-mount" && break; sleep 1; done
mountpoint -q "$out/iso-mount" || { echo "BLOCKER: redoxfs did not mount owned ISO copy" >&2; exit 3; }
cp "$binary" "$out/iso-mount/usr/bin/net-probe"
chmod 755 "$out/iso-mount/usr/bin/net-probe"
cmp "$binary" "$out/iso-mount/usr/bin/net-probe"
printf 'staged usr/bin/net-probe into owned copy; binary sha256 %s\n' "$(sha256sum "$binary" | cut -d ' ' -f1)"
fusermount3 -u "$out/iso-mount"
for _ in $(seq 1 10); do kill -0 "$rfs_pid" 2>/dev/null || break; sleep 1; done
if kill -0 "$rfs_pid" 2>/dev/null; then
    echo 'BLOCKER: image writer did not exit after unmount; refusing boot' >&2
    exit 3
fi
wait "$rfs_pid" || { echo 'BLOCKER: image writer failed' >&2; exit 3; }
rfs_pid=""
verify_image "$myiso" || exit 3
myiso_sha=$(sha256sum "$myiso" | cut -d ' ' -f1)
printf 'owned iso sha256: %s\n' "$myiso_sha"
else
myiso="$out/e1net-probe.iso"; myiso_sha=skip-stage
printf 'SKIP_STAGE=1: host-only test mode, no image writes\n' >&2
fi

# 2. Boot loop: max 2 attempts x run_timeout, stop on first FULL or on an
#    identical failure signature twice. Immutable per-run dirs, nonce-scoped.
ledger="$out/e1net-native-ledger.tsv"
[[ -f "$ledger" ]] || printf 'attempt\tstarted_utc\tverdict\tserial_sha256\tiso_orig\tiso_owned\tpcap_sha256\tpeer_sha256\tsignature\tnonce\n' >"$ledger"
serial_in="$out/serial-in.fifo"; [[ -p "$serial_in" ]] || mkfifo "$serial_in"
exec 3<>"$serial_in"

start_attempt=${START_ATTEMPT:-1}
prev_sig=""; final_verdict="none"
for attempt in $(seq "$start_attempt" $((start_attempt + 1))); do
    rd="$out/run-$attempt-$nonce"
    [[ -e "$rd" ]] && { echo "run dir $rd exists — artifacts are immutable, pick a new nonce" >&2; exit 4; }
    mkdir -p "$rd"
    start_peer "$rd" "$nonce" "$port"
    setsid timeout --signal=TERM --kill-after=5 "$run_timeout" ${QEMU:-qemu-system-aarch64} \
        -machine virt,acpi=off,gic-version=3 -cpu max -smp 1 -m 2048 \
        -bios /usr/share/qemu-efi-aarch64/QEMU_EFI.fd \
        -drive "if=none,id=e1disk,file=$myiso,format=raw,readonly=on" \
        -device virtio-blk-pci,drive=e1disk,bootindex=2 \
        -device ramfb -dtb "$dtb" \
        -netdev "user,id=e1net" \
        -object "filter-dump,id=e1dump,netdev=e1net,file=$rd/pcap.pcap" \
        -device rtl8139,netdev=e1net,bootindex=1 \
        -display none -serial stdio -monitor none \
        <"$serial_in" >"$rd/serial.log" 2>"$rd/qemu.log" & qpid=$!
    sleep 25   # boot to shell
    printf '/usr/bin/net-probe %s %s\r\n' "$nonce" "$guest_endpoint" >&3
    sleep $((run_timeout - 35))
    reap_group "$qpid"; qpid=""
    reap_group "$PEER_PGID"; PEER_PGID=""

    verdict=$(evaluate_run "$rd" "$nonce" "$port")   # extract -> classify -> accept
    final_verdict="$verdict"
    sig=$(classify "$rd/serial.log")
    printf 'attempt %s: signature=%s verdict=%s\n' "$attempt" "$sig" "$verdict"
    serial_sha=$(sha256sum "$rd/serial.log" | cut -d ' ' -f1)
    printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "$attempt" "$(date -u +%FT%TZ)" \
        "$verdict" "$serial_sha" "$iso_sha" "$myiso_sha" \
        "$(sha256sum "$rd/pcap.pcap" 2>/dev/null | cut -d ' ' -f1 || echo none)" \
        "$(sha256sum "$rd/host-recv.txt" 2>/dev/null | cut -d ' ' -f1 || echo none)" "$sig" "$nonce" >>"$ledger"
    [[ "$sig" == "$prev_sig" && "$sig" != "unknown" ]] && break
    prev_sig="$sig"
    [[ "$verdict" == FULL ]] && break
done

iso_sha_after=$(sha256sum "$orig_iso" | cut -d ' ' -f1)
[[ "$iso_sha" == "$iso_sha_after" ]] || { echo "ORIGINAL ISO CHANGED — investigate" >&2; exit 1; }
printf 'original iso unchanged after phase\n'
tail -3 "$ledger"
[[ "$final_verdict" == FULL ]] || { printf 'final verdict %s — NOT FULL, exiting nonzero\n' "$final_verdict" >&2; exit 1; }
