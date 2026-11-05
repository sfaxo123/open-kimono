#!/usr/bin/env bash
# E1-NET-NATIVE-FROM-IMAGE (Codex-approved 4:24/4:37 AM PT, 2026-10-25).
# EXACTLY ONE QEMU boot (<=90 s) of the ALREADY-VERIFIED existing image
#   out/e1-staging-check/20261025T113022Z/armB.iso
#   sha256 c91ab5338659fa6eef6a553c612ecac998a0046473292272328351e5b2f11e7c
# The image is a READ-ONLY input (QEMU readonly=on): no mounts, no copies, no
# restaging, no rebuild. The native-probe DEFAULT boot path is never run; only
# its shell functions are reused via `. e1-net-native-probe.sh lib` and `out`
# is overridden to this phase's claimed dir. One boot total is enforced by an
# atomic mkdir claim on a FIXED phase id; a retry refuses before any work.
# Verdict comes from the SAME evaluate_run as the merged tests:
#   FULL = native WRITE_OK on serial + exact nonce line at host peer (-Fx)
#          + exact nonce in directional guest->host TCP payload (pcap tool).
#   Handshake alone = PARTIAL, never success. REJECT_* preserved.
#
# STATUS: PARTIAL (Codex review 4:44 AM PT). Phase net-from-image-20261025T1137 boot budget
# The old1137 phase is spent and preserved. Codex approved ONE new readonly
# attempt after real-prompt host regressions pass; this script requires `boot`.
set -euo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
[[ "${1:-}" == boot ]] || { echo "usage: $0 boot (one approved phase only)" >&2; exit 2; }

# ---- fixed phase id, atomically claimed once (retry refuses) --------------
# Codex-fixed id (4:37 AM PT): a fresh timestamp must NEVER grant retry rights.
PHASE_ID="net-from-image-20261025T1223"
out_root="$repo/out/e1-net-from-image"
claim="$out_root/$PHASE_ID"
mkdir -p "$out_root"
if ! mkdir "$claim" 2>/dev/null; then
    printf 'BLOCKER: phase %s already claimed at %s — ONE boot total, refusing\n' \
        "$PHASE_ID" "$claim" >&2
    exit 5
fi

# ---- reuse native-probe functions (lib mode: definitions only) ------------
# shellcheck source=e1-net-native-probe.sh
. "$repo/scripts/qemu-demo/e1-net-native-probe.sh" lib
. "$repo/scripts/qemu-demo/e1-net-serial-handshake.sh" lib
out="$claim"                       # override: ALL scratch/artifacts are mine
# Any nonzero exit of a CLAIMED phase is recorded in the claim dir; the claim
# is never removed or re-dated — ask Codex before touching a claimed phase.
_note_exit() { local rc=$1; (( rc == 0 )) && return 0
    printf 'phase %s exited rc=%s at %s (claim kept; ask Codex before any re-claim)\n' \
        "$PHASE_ID" "$rc" "$(date -u +%FT%TZ)" >>"$out/abort-note.txt"; }
trap 'rc=$?; cleanup; _note_exit "$rc"' EXIT
trap 'cleanup; exit 130' INT
trap 'cleanup; exit 143' TERM
armB_iso="$repo/out/e1-staging-check/20261025T113022Z/armB.iso"
ARM_B_SHA=c91ab5338659fa6eef6a553c612ecac998a0046473292272328351e5b2f11e7c
oldbad="$repo/out/e1net-native/e1net-probe.iso"
OLDBAD_SHA=87ae0aa883724a60b7247ca0f5fb570684fc92610caa31d13b7f81e2767b942f
nonce="e1fi-$(date +%s)-$RANDOM"   # unique to this phase, overrides lib value
run_timeout=${RUN_TIMEOUT:-90}

# unique host peer port: first free in a band (checked before boot)
pick_port() {
    local p
    for p in $(seq 4781 4810); do
        if ! ss -ltnH "sport = :$p" 2>/dev/null | grep -q .; then echo "$p"; return; fi
    done
    return 1
}
port=${PORT:-$(pick_port)} || { echo 'BLOCKER: no free peer port in 4781..4810' >&2; exit 2; }
guest_endpoint="$host_ip:$port"    # slirp host alias; NEVER 127.0.0.1 in guest

# ---- hash guards: record + compare named images and ALL staging ARM images -
# Deduped filename list (sort -u): armB is BOTH a named image and a staging
# *.iso — without dedup check_named would see two rows and always fail.
hash_guard() {  # $1 = output tsv
    local f
    printf 'path\tsha256\n' >"$1"
    while IFS= read -r f; do
        printf '%s\t%s\n' "$f" "$(sha256sum "$f" | cut -d ' ' -f1)" >>"$1"
    done < <( { printf '%s\n' "$armB_iso" "$orig_iso" "$oldbad"
                find "$repo/out/e1-staging-check" -name '*.iso'; } | sort -u )
}
get_hash() {  # first match only; inconsistent duplicates -> empty (fail closed)
    awk -F'\t' -v p="$1" '$1==p{if(h==""){h=$2} else if(h!=$2){h=""; exit}} END{print h}' "$2"
}
check_named() {  # $1 = tsv to validate against the ledger hashes
    local armB_h orig_h bad_h
    armB_h=$(get_hash "$armB_iso" "$1")
    orig_h=$(get_hash "$orig_iso" "$1")
    bad_h=$(get_hash "$oldbad" "$1")
    [[ "$armB_h" == "$ARM_B_SHA" ]] || { printf 'BLOCKER: armB.iso hash %s != ledger %s\n' "$armB_h" "$ARM_B_SHA" >&2; return 1; }
    [[ "$orig_h" == "$ORIG_ISO_SHA" ]] || { printf 'BLOCKER: original ISO hash %s != ledger %s\n' "$orig_h" "$ORIG_ISO_SHA" >&2; return 1; }
    [[ "$bad_h" == "$OLDBAD_SHA" ]] || { printf 'BLOCKER: old-bad image hash %s != ledger %s\n' "$bad_h" "$OLDBAD_SHA" >&2; return 1; }
}

validate_env || exit 2
hash_guard "$out/hashes-before.tsv"
check_named "$out/hashes-before.tsv" || exit 1
printf 'hash guards BEFORE: armB/original/oldbad + all staging ARM images recorded and match ledger\n'

# ---- fresh read-only tree walk of armB.iso into disposable owned scratch --
verify_image "$armB_iso" || { echo 'BLOCKER: verify_image failed — aborting, NO boot performed' >&2; exit 3; }
printf 'verify_image: PASS (fresh redoxfs-clone tree walk, rc=0)\n'

# ---- one run dir, unique FIFO, host peer FIRST with listening proof -------
rd="$out/run-1-$nonce"
mkdir -p "$rd"
serial_in="$out/serial-in.fifo"; mkfifo "$serial_in"
exec 3<>"$serial_in"

start_peer "$rd" "$nonce" "$port"
proof="$out/peer-listening-proof.txt"
listening=0
for _ in 1 2 3 4 5 6 7 8 9 10; do
    if ss -ltnH "sport = :$port" 2>/dev/null | grep -q LISTEN; then listening=1; break; fi
    sleep 1
done
if (( ! listening )); then
    printf '%s peer NOT listening on port %s — aborting, NO boot performed\n' "$(date -u +%FT%TZ)" "$port" >"$proof"
    echo 'BLOCKER: host peer never listened — aborting, NO boot performed' >&2
    exit 6
fi
{ date -u +%FT%TZ; ss -ltnH "sport = :$port"; } >"$proof"
printf 'host peer verified LISTENING on 127.0.0.1:%s BEFORE boot (proof: %s)\n' "$port" "$proof"

# ---- THE one boot: same RTL8139/virt recipe, drive = armB.iso readonly=on -
# Guard: the drive path must be the staging-check armB image, never the
# e1net-native oldbad copy, and must be mounted readonly=on (literal below).
[[ "$armB_iso" == "$repo/out/e1-staging-check/"* && "$armB_iso" != *e1net-native* ]] \
    || { echo 'BLOCKER: drive path is not the staging-check armB image' >&2; exit 7; }
boot_rc=unknown
boot_deadline=$(( $(date +%s) + run_timeout ))
setsid timeout --signal=TERM --kill-after=5 "$run_timeout" ${QEMU:-qemu-system-aarch64} \
    -machine virt,acpi=off,gic-version=3 -cpu max -smp 1 -m 2048 \
    -bios /usr/share/qemu-efi-aarch64/QEMU_EFI.fd \
    -drive "if=none,id=e1disk,file=$armB_iso,format=raw,readonly=on" \
    -device virtio-blk-pci,drive=e1disk,bootindex=2 \
    -device ramfb -dtb "$dtb" \
    -netdev "user,id=e1net" \
    -object "filter-dump,id=e1dump,netdev=e1net,file=$rd/pcap.pcap" \
    -device rtl8139,netdev=e1net,bootindex=1 \
    -display none -serial stdio -monitor none \
    <"$serial_in" >"$rd/serial.log" 2>"$rd/qemu.log" & qpid=$!
# Reserve eight seconds for the probe's ACK/read and packet output.
handshake=$(run_handshake "$rd/serial.log" 3 "$((boot_deadline - 8))" \
    "/usr/bin/net-probe $nonce $guest_endpoint" 2>"$out/handshake.log")
printf '%s\n' "$handshake" >"$out/handshake-status.txt"
# Keep the single VM inside its original timeout; no re-launch or new image.
while kill -0 "$qpid" 2>/dev/null && (( $(date +%s) < boot_deadline )); do
    if [[ -s "$rd/host-recv.txt" ]] && grep -qFx "$nonce" "$rd/host-recv.txt" &&
       grep -q 'net-probe: READ_ACK' "$rd/serial.log"; then break; fi
    sleep 0.25
done
if kill -0 "$qpid" 2>/dev/null; then
    boot_rc=terminated_by_harness
else
    if wait "$qpid"; then boot_rc=0; else boot_rc=$?; fi
fi
reap_group "$qpid"; qpid=""
reap_group "$PEER_PGID"; PEER_PGID=""
# timeout(1) exits 124 when it had to terminate QEMU — expected, not a failure
[[ -s "$rd/qemu.log" ]] && printf 'qemu.log tail: %s\n' "$(tail -1 "$rd/qemu.log")"

# ---- verdict via the SAME evaluate_run as the merged tests ----------------
verdict=$(evaluate_run "$rd" "$nonce" "$port" 2>"$out/evaluate-stderr.txt") \
    || verdict=REJECT_EVAL_FAIL
sig=$(classify "$rd/serial.log")
printf '%s\n' "$verdict" >"$out/verdict.txt"
printf 'signature=%s verdict=%s\n' "$sig" "$verdict"

# ---- hash guards AFTER + artifact hashes -----------------------------------
hash_guard "$out/hashes-after.tsv"
check_named "$out/hashes-after.tsv" || { echo 'BLOCKER: guarded images CHANGED during phase' >&2; exit 1; }
diff "$out/hashes-before.tsv" "$out/hashes-after.tsv" \
    || { echo 'BLOCKER: any guarded image byte changed — investigate' >&2; exit 1; }
printf 'hash guards AFTER: all guarded images byte-identical before/after\n'

{
    printf 'phase_id\t%s\nnonce\t%s\nport\t%s\nrun_timeout\t%s\n' "$PHASE_ID" "$nonce" "$port" "$run_timeout"
    printf 'handshake\t%s\n' "$handshake"
    printf 'boot_rc\t%s\nsignature\t%s\nverdict\t%s\n' "$boot_rc" "$sig" "$verdict"
    printf 'armB_iso\t%s\n' "$armB_iso"
    printf 'serial\t%s\t%s\n' "$rd/serial.log" "$(sha256sum "$rd/serial.log" | cut -d ' ' -f1)"
    printf 'pcap\t%s\t%s\n' "$rd/pcap.pcap" "$(sha256sum "$rd/pcap.pcap" 2>/dev/null | cut -d ' ' -f1 || echo none)"
    printf 'pcap_payload\t%s\t%s\n' "$rd/pcap-payload.txt" "$(sha256sum "$rd/pcap-payload.txt" 2>/dev/null | cut -d ' ' -f1 || echo none)"
    printf 'host_recv\t%s\t%s\n' "$rd/host-recv.txt" "$(sha256sum "$rd/host-recv.txt" 2>/dev/null | cut -d ' ' -f1 || echo none)"
    printf 'peer_log\t%s\t%s\n' "$rd/peer.log" "$(sha256sum "$rd/peer.log" | cut -d ' ' -f1)"
} >"$out/receipt.tsv"

[[ "$verdict" == FULL ]] || { printf 'final verdict %s — NOT FULL (PARTIAL/REJECT preserved), exiting nonzero\n' "$verdict" >&2; exit 1; }
printf 'FULL: native payload proof (WRITE_OK + peer -Fx + pcap -Fx)\n'
