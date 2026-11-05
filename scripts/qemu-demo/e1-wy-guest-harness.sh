#!/usr/bin/env bash
# E1-WY-GUEST-HARNESS — dedicated Wyoming guest harness (REAL integration fix).
# Contracts (WY-REAL-INTEGRATION-FIX-20261025T1603, Cursor review §1-2):
#  1. Guest CLI is POSITIONAL: /usr/bin/wyoming-probe <ip:port> <nonce> <deadline_secs>
#     (apps/wyoming-probe/src/main.rs has NO flag parsing; a cmd containing
#     "--" is refused here and would exit 2 in the real binary).
#  2. QEMU argv is the PROVEN e1-net-native-from-image.sh:128-138 recipe verbatim
#     (virt/gic3/max/QEMU_EFI.fd/virtio-blk readonly=on/ramfb/dtb/user-net/
#     rtl8139/filter-dump). The accepted staged image 6dffa1c4..b71d is a
#     READ-ONLY input: no staging wrapper, no copies, no mounts, no restaging.
#  3. FIFO opened O_RDWR (exec 3<>) BEFORE launch; timeout+QEMU share ONE owned
#     process group (single setsid, NFI pattern); traps installed before start;
#     reap = kill -TERM -- -PGID, bounded KILL, wait. No nested setsid.
#  4. Host side = e1-wy-guest-harness-fixture.py: two REAL sockets (answering
#     Wyoming fixture on the pinned reference read_event/write_event/Pong API +
#     silent peer), OS-assigned ports, JSON readiness line + ss LISTEN proof.
#  5. Offset race fix WITHOUT touching the HS lib: HS is armed with `echo WYARM`,
#     offset taken after HANDSHAKE_OK, wait_marker re-anchors at the post-echo
#     prompt, THEN send_line the real probe; scan only anchored PROBE lines.
#     90 s per VM total; each leg has its own sub-claim; leg2 timing measured
#     from probe-send to the exact FAIL line.
#  6. QEMU + fixture are stopped and reaped BEFORE the pcap is parsed;
#     e1-wy-guest-harness-pcap.py verifies the exact TCP tuple ports AND the
#     nonce inside ping/pong frames both directions; fail closed on anything
#     unsupported.
#  7. Run mode: production pins/paths are FIXED (WG_* ignored). WG_FAKE_TEST=1
#     may redirect tools ONLY inside WG_FAKE_ROOT. --approve + WG_CODEX_PHASE is
#     admission bookkeeping, NOT a security token. Before/after hashes + owned
#     PGIDs recorded; protected-file disappearance fails.
# Subcommands: dry-run | run.  Exit: 0 ok | 1 guard/fail-closed | 2 bad args
# | 3 missing dep/input | 4 phase claimed | 5 leg/verdict failure.
set -uo pipefail
repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)

# ---- production pins (fixed; seams only inside WG_FAKE_ROOT) -----------------
seam() {  # $1=VAR $2=default — WG_* honored ONLY in fake-test mode under root
    local cur=${!1:-}
    if [[ "${WG_FAKE_TEST:-0}" == 1 && -n "${WG_FAKE_ROOT:-}" && "$cur" == "$WG_FAKE_ROOT"/* ]]; then
        printf '%s' "$cur"; else printf '%s' "$2"; fi
}
seam_val() {  # non-path seam (sha pins, bounds): fake-test mode only
    if [[ "${WG_FAKE_TEST:-0}" == 1 ]]; then printf '%s' "${1:-$2}"; else printf '%s' "$2"; fi
}
OUT=$(seam WG_OUT "$repo/out/e1-wy-guest-harness")
STAGED_DEF=$repo/out/e1-wyoming-stage-prep/wy-1793005221-kern23/wy-stage.iso
image_pin=$(seam WG_IMAGE "$STAGED_DEF")
STAGED_SHA=$(seam_val "${WG_STAGED_ISO_SHA:-}" 6dffa1c4e6817b033141d2878852b94d59a1e96c1597b10f32f16bc55cbab71d)
WY_GUEST_SHA=d1576d6a0d6973af87d5d4d3221746633cbddb1f94f328531021b1ebcced703b
# Python wyoming reference checkout HEAD (verified via git rev-parse, NOT the
# import path or dist-info version; mismatch or dirty tree fails before claim).
WY_REF_HEAD=$(seam_val "${WG_REF_HEAD:-}" 08a0c258df83fea6e1643bcfe67e86243f01cf31)
ORIG_ISO=$(seam WG_ORIG_ISO "$HOME/Developer/forks/redox/redox/build/aarch64/desktop-minimal/e1-live.iso")
ORIG_ISO_SHA=$(seam_val "${WG_ORIG_ISO_SHA:-}" c02012eaa182800cc7ed937e0e3ebf7b436f2af3e9dacb5c57bb00c89df361bc)
BAD_ISO=$(seam WG_BAD_ISO "$repo/out/e1net-native/e1net-probe.iso")
BAD_ISO_SHA=$(seam_val "${WG_BAD_ISO_SHA:-}" 87ae0aa883724a60b7247ca0f5fb570684fc92610caa31d13b7f81e2767b942f)
ARM_B=$(seam WG_ARM_B_ISO "$repo/out/e1-staging-check/20261025T113022Z/armB.iso")
ARM_B_SHA=$(seam_val "${WG_ARM_B_ISO_SHA:-}" c91ab5338659fa6eef6a553c612ecac998a0046473292272328351e5b2f11e7c)
FIRMWARE=$(seam WG_FIRMWARE /usr/share/qemu-efi-aarch64/QEMU_EFI.fd)
DTB=$(seam WG_DTB "$repo/out/qemu-demo/simplefb-virt.dtb")
QEMU_BIN=$(seam WG_QEMU_BIN qemu-system-aarch64)
PYREF=$HOME/Developer/forks/op6-refs/wyoming
PYREF_META=$repo/crates/wyoming/tests/pyref_meta
fixture_py=$repo/scripts/qemu-demo/e1-wy-guest-harness-fixture.py
pcap_py=$repo/scripts/qemu-demo/e1-wy-guest-harness-pcap.py
hs_lib=$repo/scripts/qemu-demo/e1-net-serial-handshake.sh
host_ip=10.0.2.2
vm_bound=$(seam_val "${WG_VM_BOUND:-}" 90); probe_deadline=$(seam_val "${WG_PROBE_DEADLINE:-}" 10)
grace=$(seam_val "${WG_GRACE:-}" 10)

usage() { printf 'usage: %s <dry-run|run> --phase wy-<UTCstamp>[-tag] --image <staged.iso> [--approve] [--boot-cmd <cmd>]\n' "$0" >&2; }

# ---- args + bounds BEFORE any side effect (exit 2) ---------------------------
sub=${1:-}; shift || true
phase=""; image=""; approve=0; boot_cmd=""
while (($#)); do
    case $1 in
        --phase)  [[ $# -ge 2 ]] || { usage; exit 2; }; phase=$2; shift 2 ;;
        --image)  [[ $# -ge 2 ]] || { usage; exit 2; }; image=$2; shift 2 ;;
        --approve) approve=1; shift ;;
        --boot-cmd) [[ $# -ge 2 ]] || { usage; exit 2; }; boot_cmd=$2; shift 2 ;;
        *) usage; exit 2 ;;
    esac
done
case $sub in dry-run|run) ;; *) usage; exit 2 ;; esac
[[ "$phase" =~ ^wy-[0-9]{10}(-[a-z0-9]{1,12})?$ ]] || { usage; exit 2; }
[[ -n "$image" ]] || { usage; exit 2; }
[[ "$vm_bound" =~ ^[0-9]+$ ]] && (( vm_bound >= 40 && vm_bound <= 90 )) || { echo "REFUSED: vm_bound 40..90 (hard VM ceiling 90s)" >&2; exit 2; }
[[ "$probe_deadline" =~ ^[0-9]+$ ]] && (( probe_deadline >= 3 && probe_deadline <= 10 )) || { usage; exit 2; }
[[ "$grace" =~ ^[0-9]+$ ]] && (( grace >= 1 && grace <= 30 )) || { usage; exit 2; }
if [[ -n "$boot_cmd" ]]; then
    echo "REFUSED: GENERIC_BOOT_CMD_REJECTED — this harness never executes --boot-cmd." >&2; exit 2
fi
if [[ $sub == run ]]; then
    [[ $approve -eq 1 ]] || { echo "REFUSED: run requires explicit --approve" >&2; exit 2; }
    [[ "${WG_CODEX_PHASE:-}" == "$phase" ]] || { echo "REFUSED: NOT_AUTHORIZED — WG_CODEX_PHASE does not match phase $phase (admission bookkeeping, not a security token)" >&2; exit 2; }
fi
if [[ "${WG_FAKE_TEST:-0}" == 1 ]]; then
    [[ -d "${WG_FAKE_ROOT:-}" ]] || { echo "REFUSED: WG_FAKE_TEST requires an existing WG_FAKE_ROOT" >&2; exit 2; }
    [[ "$image" == "$WG_FAKE_ROOT"/* ]] || { echo "REFUSED: fake mode image must live under WG_FAKE_ROOT" >&2; exit 2; }
else
    [[ "$image" == "$image_pin" ]] || { echo "REFUSED: IMAGE_PIN_OVERRIDE — run mode uses the accepted staged image only ($image_pin)" >&2; exit 2; }
fi

sha() { sha256sum "$1" 2>/dev/null | cut -d ' ' -f1; }

# ---- preflight: every dependency resolved BEFORE the claim (exit 3) ---------
preflight_rc=0
have_cmd() { [[ "$1" == */* ]] && { [[ -x "$1" ]] && return 0 || return 1; }; command -v "$1" >/dev/null; }
need()   { [[ -e "$2" ]] || { echo "PREFLIGHT MISSING: $1: $2" >&2; preflight_rc=1; }; }
have_cmd "$QEMU_BIN" || { echo "PREFLIGHT MISSING: qemu binary: $QEMU_BIN" >&2; preflight_rc=1; }
need firmware "$FIRMWARE"; need dtb "$DTB"; need fixture "$fixture_py"; need pcapverifier "$pcap_py"
need hs_lib "$hs_lib"; need pyref "$PYREF/wyoming/__init__.py"; need pyref_meta "$PYREF_META"
command -v python3 >/dev/null || { echo "PREFLIGHT MISSING: python3" >&2; preflight_rc=1; }
[[ $preflight_rc -eq 0 ]] || exit 3
ref_head=$(git -C "$PYREF" rev-parse HEAD 2>/dev/null || true)
ref_dirty=$(git -C "$PYREF" status --porcelain 2>/dev/null); ref_rc=$?
[[ "$ref_head" == "$WY_REF_HEAD" && $ref_rc -eq 0 && -z "$ref_dirty" ]] || {
    echo "FAIL-CLOSED: PYREF_PIN_OR_TREE_MISMATCH" >&2; exit 1;
}
[[ ! -L "$image" ]] || { echo "FAIL-CLOSED: symlink image refused" >&2; exit 1; }
test -f "$image" || { echo "BLOCKER: staged image missing: $image" >&2; exit 3; }
img_got=$(sha "$image")
[[ "$img_got" == "$STAGED_SHA" ]] || { echo "FAIL-CLOSED: image $img_got != pinned staged sha $STAGED_SHA" >&2; exit 1; }

# ---- phase claim (mkdir, once) ----------------------------------------------
run=$OUT/$phase
mkdir -p "$OUT"
mkdir "$run" 2>/dev/null || { echo "REFUSED: phase $phase already claimed; no retries" >&2; exit 4; }
res=$run/results.tsv
printf 'key\tvalue\n' >"$res"
rec() { printf '%s\t%s\n' "$1" "${2:-}" >>"$res"; printf '%s: %s\n' "$1" "${2:-}"; }
n1=wy-guest-$phase-$RANDOM; n2=$n1-silent
rec phase "$phase"; rec sub "$sub"; rec image "$image"; rec image_sha "$img_got"
rec staged_pin_sha "$STAGED_SHA"; rec guest_elf_pin "$WY_GUEST_SHA"
rec vm_bound_s "$vm_bound"; rec probe_deadline_s "$probe_deadline"; rec grace_s "$grace"
rec nonce "$n1"; rec nonce_silent "$n2"; rec phase_budget "2-probe-1-VM"

# ---- protected images before-hashes ------------------------------------------
prot_rc=0; declare -A PROT_PRESENT=()
prot() {  # $1=name $2=path $3=pin
    if [[ ! -e "$2" ]]; then PROT_PRESENT[$1]=0; rec "protected.$1" absent; return 0; fi
    PROT_PRESENT[$1]=1; local s; s=$(sha "$2")
    [[ "$s" == "$3" ]] && rec "protected.$1.sha" ok || { rec "protected.$1.sha" "MISMATCH:$s"; return 1; }
}
protafter() {
    if [[ ! -e "$2" ]]; then
        [[ ${PROT_PRESENT[$1]:-0} -eq 1 ]] && { rec "protected.$1.after" DISAPPEARED; return 1; }
        rec "protected.$1.after" absent; return 0
    fi
    local s; s=$(sha "$2")
    [[ "$s" == "$3" ]] && rec "protected.$1.after.sha" ok || { rec "protected.$1.after.sha" "MISMATCH:$s"; return 1; }
}
prot orig_iso "$ORIG_ISO" "$ORIG_ISO_SHA" || prot_rc=1
prot bad_image "$BAD_ISO" "$BAD_ISO_SHA" || prot_rc=1
prot armB "$ARM_B" "$ARM_B_SHA" || prot_rc=1
[[ $prot_rc -eq 0 ]] || { rec verdict PROTECTED_IMAGE_MISMATCH; exit 1; }

# ---- ownership: traps BEFORE any launch; PGIDs recorded ----------------------
declare -a PGIDS=(); CLEANUP_DONE=0; FD3_OPEN=0; REAP_RC=0
reap_group() {  # bounded TERM -> KILL on the PGID, then wait; never global
    local pg=$1 i
    [[ -z "$pg" ]] && return 0
    kill -TERM -- "-$pg" 2>/dev/null || true
    for i in 1 2 3 4; do kill -0 -- "-$pg" 2>/dev/null || { wait "$pg" 2>/dev/null || true; return 0; }; sleep 1; done
    kill -KILL -- "-$pg" 2>/dev/null || true
    for i in 1 2 3 4; do kill -0 -- "-$pg" 2>/dev/null || { wait "$pg" 2>/dev/null || true; return 0; }; sleep 1; done
    rec "cleanup.reap_failed_pg" "$pg"; return 1
}
AFTER_DONE=0; PROT_AFTER_RC=0
after_hashes() {
    (( AFTER_DONE )) && return 0; AFTER_DONE=1
    local ia; ia=$(sha "$image")
    [[ "$ia" == "$img_got" ]] && rec input_image.after ok \
        || { rec input_image.after "MUTATED:$img_got->$ia"; PROT_AFTER_RC=1; }
    protafter orig_iso "$ORIG_ISO" "$ORIG_ISO_SHA" || PROT_AFTER_RC=1
    protafter bad_image "$BAD_ISO" "$BAD_ISO_SHA" || PROT_AFTER_RC=1
    protafter armB "$ARM_B" "$ARM_B_SHA" || PROT_AFTER_RC=1
    (( PROT_AFTER_RC )) && rec protected.after_fail 1
}
cleanup() {
    (( CLEANUP_DONE )) && return 0; CLEANUP_DONE=1
    local pg
    for pg in "${PGIDS[@]}"; do reap_group "$pg" || REAP_RC=1; done
    PGIDS=()
    if (( FD3_OPEN )); then exec 3>&-; fi
    after_hashes
}
# Protected mutation/disappearance detected during failure cleanup still
# forces a nonzero exit — a failing path may never mask integrity damage.
trap 'rc=$?; cleanup; (( PROT_AFTER_RC || REAP_RC )) && (( rc == 0 )) && rc=1; exit $rc' EXIT
trap 'cleanup; exit 130' INT
trap 'cleanup; exit 143' TERM

plan() {
    printf 'PLAN (%s): fixture(2 real sockets, ref Pong) -> boot ONCE (NFI argv, readonly staged image) -> WYARM arm -> offset -> probe1 %s -> probe2 %s -> reap BEFORE pcap -> pcap exact-tuple nonce ping/pong -> hashes+PGIDs\n' "$sub" "$n1" "$n2"
}
if [[ $sub == dry-run ]]; then plan | tee "$run/plan.txt"; rec verdict DRY_RUN_OK_NO_VM; exit 0; fi

# ---- host fixture: two REAL sockets, readiness before anything boots --------
fready=$run/fixture-ready.json; freceipt=$run/fixture-receipt.json
PYTHONPATH="$PYREF:$PYREF_META" setsid python3 "$fixture_py" \
    --receipt "$freceipt" --deadline $(( $(date +%s) + vm_bound + 2 * probe_deadline + grace + 15 )) \
    >"$fready" 2>"$run/fixture.err" & FPG=$!; PGIDS+=("$FPG")
ap=""; sp=""
for _ in $(seq 1 40); do
    ap=$(python3 - "$fready" <<'PJ' 2>/dev/null
import json, sys
try:
    d = json.loads(open(sys.argv[1]).read().splitlines()[0])
    print(d["answer_port"], d["silent_port"])
except Exception: pass
PJ
)
    read -r ap sp <<<"$ap"
    [[ -n "$ap" && -n "$sp" ]] && break
    kill -0 "$FPG" 2>/dev/null || break
    sleep 0.25
done
if [[ -z "$ap" || -z "$sp" ]] || ! grep -q "$PYREF" "$fready"; then
    rec verdict FIXTURE_NOT_READY; rec fixture.err "$(tail -c 200 "$run/fixture.err" 2>/dev/null)"
    exit 1
fi
for p in "$ap" "$sp"; do
    ss -ltnH "sport = :$p" 2>/dev/null | grep -q LISTEN || { rec verdict "FIXTURE_NOT_LISTENING:$p"; exit 1; }
done
rec fixture.answer_port "$ap"; rec fixture.silent_port "$sp"
rec fixture.ref "$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["ref"])' "$fready" 2>/dev/null)"
rec pyref.head ok

# ---- ONE boot: FIFO O_RDWR first, then NFI argv in ONE owned group ----------
fifo=$run/serial-in.fifo; serial=$run/serial.log
mkfifo -- "$fifo" || { rec verdict FIFO_FAIL; exit 1; }
exec 3<>"$fifo" || { rec verdict FIFO_OPEN_FAIL; exit 1; }
FD3_OPEN=1
t0=$(date +%s)
setsid timeout --signal=TERM --kill-after=5 "$vm_bound" "$QEMU_BIN" \
    -machine virt,acpi=off,gic-version=3 -cpu max -smp 1 -m 2048 \
    -bios "$FIRMWARE" \
    -drive "if=none,id=e1disk,file=$image,format=raw,readonly=on" \
    -device virtio-blk-pci,drive=e1disk,bootindex=2 \
    -device ramfb -dtb "$DTB" \
    -netdev "user,id=e1net" \
    -object "filter-dump,id=e1dump,netdev=e1net,file=$run/pcap.pcap" \
    -device rtl8139,netdev=e1net,bootindex=1 \
    -display none -serial stdio -monitor none \
    <"$fifo" >"$serial" 2>"$run/qemu.log" & QPID=$!; PGIDS+=("$QPID")
rec qemu.pg "$QPID"; rec fixture.pg "$FPG"

# ---- handshake (proven HS lib, untouched) armed with a harmless marker ------
# shellcheck disable=SC1090
source "$hs_lib" lib || { rec verdict HS_LIB_MISSING; exit 1; }
set +e  # the HS lib sets -eu for its own use; this harness manages rc explicitly
boot_deadline=$(( t0 + vm_bound ))
hs_deadline=$(( boot_deadline - 8 - 2 * probe_deadline ))
# Scan floor is taken BEFORE the handshake's arm-command send, but every arm /
# anchor wait below is LINE-ANCHORED: the command echo ("echo WYARM") can never
# satisfy a ^WYARM line wait, no handshake text contains one, and a response
# that lands before any snapshot can never be missed. The real probe commands
# are sent only after a FRESH shell prompt that follows the arm result line.
pre_off=$(wc -c <"$serial")
st=$(run_handshake "$serial" 3 "$hs_deadline" "echo WYARM")
rec handshake "$st"
if [[ "$st" != HANDSHAKE_OK ]]; then rec verdict "HANDSHAKE_FAIL:$st"; exit 5; fi
off=$pre_off

scan_probe() {  # $1=off $2=deadline $3=leg-floor-off -> first anchored PROBE line
    local cur=$1 dl=$2 floor=$3 size out
    while :; do
        size=$(wc -c <"$serial" 2>/dev/null || echo 0)
        if (( size > cur )); then
            out=$(python3 - "$serial" "$cur" "$size" <<'PSCAN'
from pathlib import Path
import re, sys
f, start, end = sys.argv[1:]
with Path(f).open('rb') as s:
    s.seek(int(start)); data = s.read(int(end) - int(start))
data = re.sub(rb'\x1b\][^\x07]*(?:\x07|\x1b\\)', b'', data)
data = re.sub(rb'\x1b\[[0-?]*[ -/]*[@-~]', b'', data)
nl = data.rfind(b'\n')
if nl < 0: sys.exit(0)          # no COMPLETE line yet: a fragmented PROBE line
data = data[:nl]                # must never be matched as if it were finished
for line in re.split(rb'[\r\n]+', data):
    line = line.strip(b'\n').strip()
    if re.match(rb'^PROBE (PASS|FAIL)( |$)', line):
        sys.stdout.write(line.decode('utf-8', 'replace')); break
PSCAN
)
            [[ -n "$out" ]] && { printf '%s' "$out"; return; }
            cur=$(( size - 200 )); (( cur < floor )) && cur=$floor
        fi
        (( $(date +%s) >= dl )) && { printf 'SCAN_TIMEOUT'; return; }
        sleep "${POLL_STEP:-0.25}"
    done
}

# wait_line <offvar> <python-line-regex> <deadline> — anchored at line start,
# requires the line to be TERMINATED (\r or \n present), and advances the offset
# to the END of the match (not EOF size), so a prompt that was already written
# past a fast reply is still found by the next wait. Echo lines (prefixed by
# the typed command) can never match a ^-anchored result pattern.
wait_line() {
    local offvar=$1 pat=$2 dl=$3 cur size pos
    cur=${!offvar}
    while :; do
        size=$(wc -c <"$serial" 2>/dev/null || echo 0)
        if (( size > cur )); then
            pos=$(python3 - "$serial" "$cur" "$size" "$pat" <<'PWLINE'
from pathlib import Path
import re, sys
f, start, end, pat = sys.argv[1:]
with Path(f).open('rb') as s:
    s.seek(int(start)); data = s.read(int(end) - int(start))
# Match normalized text but return offsets in ORIGINAL serial bytes. Stripping
# CSI/OSC first changes byte counts and can move the next scan into an old prompt.
for line in re.finditer(rb'[^\r\n]*(?:\r\n|\r|\n)', data):
    clean = re.sub(rb'\x1b\][^\x07]*(?:\x07|\x1b\\)', b'', line.group())
    clean = re.sub(rb'\x1b\[[0-?]*[ -/]*[@-~]', b'', clean)
    if re.search(pat.encode(), clean, re.M):
        sys.stdout.write(str(int(start) + line.end()))
        break
PWLINE
)
            [[ -n "$pos" ]] && { printf -v "$offvar" '%s' "$pos"; return 0; }
        fi
        (( $(date +%s) >= dl )) && return 1
        sleep "${POLL_STEP:-0.25}"
    done
}

# leg1: SUCCESS leg. Two-stage arm: the ACTUAL WYARM output line (line-anchored:
# the command echo can never match), then a FRESH shell prompt — the probe is
# only sent once the shell is actually idle at a prompt that follows the arm
# command's result. floor is anchored before the send so
# the scan can never miss a fast PROBE line.
mkdir "$run/leg1-claim" 2>/dev/null || { rec verdict LEG1_CLAIM_CONSUMED; exit 4; }
wait_line off '^WYARM(?=[\r\n])' "$hs_deadline" || { rec verdict LEG1_ARM_WAIT:ARM_LINE_TIMEOUT; exit 5; }
wait_marker "$serial" off "$M_SHELL" "$hs_deadline" "$AB_SHELL" || { rec verdict "LEG1_PROMPT_WAIT:$AB_SHELL"; exit 5; }
floor=$off
probe1="/usr/bin/wyoming-probe $host_ip:$ap $n1 $probe_deadline"
[[ "$probe1" != *--* ]] || { rec verdict PROBE_CMD_FLAG_REJECTED; exit 1; }
t_send1=$(date +%s)
send_line 3 "$probe1"
line1=$(scan_probe "$floor" $(( t_send1 + probe_deadline + 5 )) "$floor")
el1=$(( $(date +%s) - t_send1 ))
rec leg1.line "$line1"; rec leg1.elapsed_s "$el1"

# Anchor leg2's floor on leg1's exact verdict line — unique, cannot be the echo —
# then require the fresh prompt that follows it; the scan floor is set only
# after that prompt, so leg1 output or any stale marker can never be leg2's.
mkdir "$run/leg2-claim" 2>/dev/null || { rec verdict LEG2_CLAIM_CONSUMED; exit 4; }
wait_line off '^PROBE PASS addr=' "$hs_deadline" || { rec verdict LEG2_ANCHOR_WAIT:ANCHOR_LINE_TIMEOUT; exit 5; }
wait_marker "$serial" off "$M_SHELL" "$hs_deadline" "$AB_SHELL" || { rec verdict "LEG2_PROMPT_WAIT:$AB_SHELL"; exit 5; }
floor=$off
probe2="/usr/bin/wyoming-probe $host_ip:$sp $n2 $probe_deadline"
t_send2=$(date +%s)
send_line 3 "$probe2"
line2=$(scan_probe "$floor" $(( t_send2 + probe_deadline + grace )) "$floor")
el2=$(( $(date +%s) - t_send2 ))
rec leg2.line "$line2"; rec leg2.elapsed_s "$el2"

# ---- STOP FIRST: reap QEMU + fixture, THEN parse the pcap (contract 6) ------
exec 3>&-; FD3_OPEN=0
reap_group "$QPID" || REAP_RC=1
reap_group "$FPG" || REAP_RC=1
for _ in $(seq 1 20); do [[ -f "$freceipt" ]] && break; sleep 0.25; done
pcap=$run/pcap.pcap
pcout=$(python3 "$pcap_py" "$pcap" "$ap" "$n1" --host-ip "$host_ip" 2>&1); prc=$?
while IFS=$'\t' read -r k v; do [[ -n "$k" ]] && rec "$k" "$v"; done <<<"$pcout"
rec pcap.verify_rc "$prc"

# ---- fixture receipt evidence (real sockets, reference API) -----------------
fev=$(PYTHONPATH="$PYREF:$PYREF_META" python3 - "$freceipt" "$n1" <<'PR'
import json, sys
try:
    d = json.load(open(sys.argv[1]))
except Exception as e:
    print("fixture.receipt\tUNREADABLE"); sys.exit(0)
c = d.get("answer_counts", {})
print("fixture.describe\t%s" % ("yes" if c.get("describe", 0) >= 1 else "no"))
print("fixture.info\t%s" % ("yes" if c.get("info", 0) >= 1 else "no"))
print("fixture.ping\t%s" % ("yes" if c.get("ping", 0) >= 1 else "no"))
print("fixture.writes\t%s" % d.get("answer_writes", 0))
print("fixture.nonce_seen\t%s" % ("yes" if d.get("nonce_seen") == sys.argv[2] else "no"))
print("fixture.eof\t%s" % ("yes" if d.get("answer_eof") else "no"))
print("fixture.silent_accepts\t%d" % len(d.get("silent_accepts", [])))
print("fixture.silent_writes\t%d" % d.get("silent_writes", 0))
ev = d.get("silent_events", [])
print("fixture.silent_first_event\t%s" % (ev[0]["type"] if ev else "none"))
PR
)
while IFS=$'\t' read -r k v; do [[ -n "$k" ]] && rec "$k" "$v"; done <<<"$fev"

for f in serial.log pcap.pcap fixture-receipt.json fixture-ready.json; do
    [[ -f "$run/$f" ]] && rec "hash.$f" "$(sha "$run/$f")" || rec "hash.$f" MISSING
done

# ---- verdict (fail closed; every key is measured, not assumed) --------------
leg1_ok=0
# Production: exact addr=10.0.2.2:<answer port>. Fake mode: the stub guest has
# no slirp, so its probe legitimately dials the local relay — same nonce check.
if [[ "${WG_FAKE_TEST:-0}" == 1 ]]; then want1="PROBE PASS addr=127.0.0.1:"; else want1="PROBE PASS addr=$host_ip:$ap"; fi
[[ "$line1" == "$want1"* && "$line1" == *" nonce=$n1" ]] && leg1_ok=1
leg2_ok=0
[[ "$line2" == "PROBE FAIL describe: socket read timed out (silent peer)" ]] && leg2_ok=1
# Measured silent time bounded on BOTH sides: too fast = an arbitrary PROBE
# FAIL cannot pass; too slow = the scan deadline was blown.
[[ $leg2_ok -eq 1 && ( $el2 -lt $(( probe_deadline - 1 )) || $el2 -gt $(( probe_deadline + grace )) ) ]] && leg2_ok=0
if [[ $leg2_ok -eq 1 ]]; then
    sa=$(sed -n 's/^fixture\.silent_accepts\t//p' <<<"$fev")
    [[ "$sa" =~ ^[0-9]+$ && "$sa" -ge 1 ]] || leg2_ok=0
    grep -q 'fixture.silent_first_event	describe' <<<"$fev" || leg2_ok=0
    grep -q 'fixture.silent_writes	0' <<<"$fev" || leg2_ok=0
fi
orphans=0
for pg in "$QPID" "$FPG"; do
    # The whole OWNED process group must be gone: a surviving child with a dead
    # leader still keeps the PGID alive, so check -PGID, never the leader alone.
    kill -0 -- "-$pg" 2>/dev/null && orphans=$(( orphans + 1 ))
done
rec orphan_pgids "$orphans"
after_hashes
img_after=$(sha "$image"); input_mutated=0
[[ "$img_after" == "$img_got" ]] || { rec input_image.after "MUTATED:$img_got->$img_after"; input_mutated=1; }
CLEANUP_DONE=1  # traps must not re-run after-hashes

if [[ $leg1_ok -eq 1 && $leg2_ok -eq 1 && $prc -eq 0 && $prot_rc -eq 0 \
      && $PROT_AFTER_RC -eq 0 && $REAP_RC -eq 0 \
      && $input_mutated -eq 0 && $orphans -eq 0 \
      && "$(grep -c 'fixture.nonce_seen	yes' <<<"$fev")" -eq 1 \
      && "$(grep -c 'fixture.describe	yes' <<<"$fev")" -eq 1 \
      && "$(grep -c 'fixture.info	yes' <<<"$fev")" -eq 1 \
      && "$(grep -c 'fixture.ping	yes' <<<"$fev")" -eq 1 \
      && "$(grep -c 'fixture.eof	yes' <<<"$fev")" -eq 1 \
      && "$(grep -c 'fixture.silent_writes	0' <<<"$fev")" -eq 1 ]]; then
    rec verdict GUEST_HARNESS_PASS; exit 0
fi
rec verdict GUEST_HARNESS_FAIL
exit 5
