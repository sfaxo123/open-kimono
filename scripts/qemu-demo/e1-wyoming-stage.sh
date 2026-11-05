#!/usr/bin/env bash
# E1-WYOMING-STAGE: wyoming-specific staging/boot wrapper for the guest artifact
# usr/bin/wyoming-probe. Host-only authoring (Codex-authorized prep, 6:28 AM PT);
# stage/boot paths are GATED and NOT authorized to run until Codex budget.
# Guard lineage: e1-staging-check.sh (main, 8a824e5) — fixed approved phase-ID
# claimed ONCE (retry refuses), stop-before-mount on baseline failure, old-evidence
# mismatch fails closed, scratch alias checks with exclusive create.
# Boot lineage: 594dbda prompt-driven serial handshake (bootloader -> login ->
# root:~# -> shell). The fixed-25s-sleep wrapper was REJECTED; no fixed-sleep
# boot pattern exists in this file. boot is DISABLED here: it fails closed with
# BOOT_NOT_IMPLEMENTED BEFORE any side effect; no --boot-cmd is ever executed.
# Subcommands: dry-run | stage | boot.
#   dry-run: zero writes outside out/e1-wyoming-stage-prep/ (claim + receipts).
#   stage:   requires --approve + fresh phase claim; disposable copy ONLY — the
#            input image is NEVER written; redoxfs-clone hash-recomputing walks
#            verify the copy before and after the mount.
#   boot:    fails closed with BOOT_NOT_IMPLEMENTED BEFORE any side effect;
#            requires a separately reviewed dedicated Wyoming QEMU harness
#            (594dbda prompt-driven lineage). --boot-cmd is parsed so tests
#            can prove it NEVER runs, but is never executed here.
# Kernel-candidate phase (optional, PAIRED --kernel + --kernel-all, all-or-none,
#   wy-<stamp>-kern* phase only): additionally replaces ONLY usr/lib/boot/kernel
#   and usr/lib/boot/kernel.all. Both hard-pinned (no WS_* seam), manifest roles
#   kernel/kernel-all, validated BEFORE claim/copy. Pins are not run authority.
# Exit codes: 0 ok | 1 guard/fail-closed | 2 bad args | 3 missing input | 4 phase claimed.
# Host-test seams (WS_*): FAKE-ONLY path/command redirects for the stub suite.
# They redirect PATHS only — pinned shas stay enforced against whatever file
# exists there — and are NOT an admission mechanism; never set in a real run.
set -uo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
forks=${REDOX_FORKS:-$HOME/Developer/forks/redox}
redox="$forks/redox"
OUT=${WS_OUT:-$repo/out/e1-wyoming-stage-prep}

# Pinned evidence for the guest artifact (docs/workbench/E1-WYOMING-PROBE.md:236,
# docs/HANDOFF.md:22). A manifest that contradicts this fails closed.
WY_BINARY_SHA=${WS_WY_BINARY_SHA:-d1576d6a0d6973af87d5d4d3221746633cbddb1f94f328531021b1ebcced703b}
GUEST_BIN=usr/bin/wyoming-probe
DEF_BINARY=$repo/out/wyoming-probe/redox/aarch64-unknown-redox/release/wyoming-probe
# Kernel-candidate pins: deliberately NO WS_* override.
KERNEL_SHA=b8533efd2beea718ab9879f9800c0fa5bf2ecaba8ff8964ad894763628d246c5
KERNEL_ALL_SHA=baede849e769f70c4983a61a4f444922a13e9d7c12b5172cc11954d950dbaba6
KERNEL_SRC_ID=cb1dfb1388a33674ccfa14bd20806f038ba42819  # receipt metadata only
GUEST_KERNEL=usr/lib/boot/kernel
GUEST_KERNEL_ALL=usr/lib/boot/kernel.all
# Protected images (never written; verified before/after when present).
ORIG_ISO_DEF=$redox/build/aarch64/desktop-minimal/e1-live.iso
ORIG_ISO_SHA=c02012eaa182800cc7ed937e0e3ebf7b436f2af3e9dacb5c57bb00c89df361bc
BAD_ISO_DEF=$repo/out/e1net-native/e1net-probe.iso
BAD_ISO_SHA=87ae0aa883724a60b7247ca0f5fb570684fc92610caa31d13b7f81e2767b942f
# armB.iso pinned from e1-net-native-from-image.sh:49-50 (accepted lineage)
ARM_B_DEF=$repo/out/e1-staging-check/20261025T113022Z/armB.iso
ARM_B_SHA=c91ab5338659fa6eef6a553c612ecac998a0046473292272328351e5b2f11e7c
orig_iso=${WS_ORIG_ISO:-$ORIG_ISO_DEF}
bad_iso=${WS_BAD_ISO:-$BAD_ISO_DEF}
armB_iso=${WS_ARM_B_ISO:-$ARM_B_DEF}
fstool=${WS_FSTOOL:-$redox/build/fstools/bin/redoxfs}
clone=${WS_CLONE:-$redox/build/fstools/bin/redoxfs-clone}
mnt_cmd=${WS_MOUNTPOINT:-mountpoint}
um_cmd=${WS_UMOUNT:-fusermount3}
clone_timeout=${WS_CLONE_TIMEOUT:-40}
boot_timeout=${WS_BOOT_TIMEOUT:-120}

usage() {
    printf 'usage: %s <dry-run|stage|boot> --phase wy-<UTCstamp>[-tag] --image <path> [--manifest <tsv>] [--binary <path>] [--kernel <path> --kernel-all <path>] [--approve] [--boot-cmd <cmd>]\n' "$0" >&2
}

# ---- argument + numeric validation BEFORE any side effect (exit 2) ----------
sub=${1:-}; shift || true
phase=""; image=""; manifest=""; binary=$DEF_BINARY; approve=0; boot_cmd=""
kernel=""; kernel_all=""
while (($#)); do
    case $1 in
        --phase)    [[ $# -ge 2 ]] || { usage; exit 2; }; phase=$2; shift 2 ;;
        --image)    [[ $# -ge 2 ]] || { usage; exit 2; }; image=$2; shift 2 ;;
        --manifest) [[ $# -ge 2 ]] || { usage; exit 2; }; manifest=$2; shift 2 ;;
        --binary)   [[ $# -ge 2 ]] || { usage; exit 2; }; binary=$2; shift 2 ;;
        --kernel)     [[ $# -ge 2 && -n "$2" && -z "$kernel" ]] || { usage; exit 2; }; kernel=$2; shift 2 ;;
        --kernel-all) [[ $# -ge 2 && -n "$2" && -z "$kernel_all" ]] || { usage; exit 2; }; kernel_all=$2; shift 2 ;;
        --approve)  approve=1; shift ;;
        --boot-cmd) [[ $# -ge 2 ]] || { usage; exit 2; }; boot_cmd=$2; shift 2 ;;
        *) usage; exit 2 ;;
    esac
done
case $sub in dry-run|stage|boot) ;; *) usage; exit 2 ;; esac
# NEW wyoming phase-ID namespace: wy-<10-digit UTC stamp>[-tag]. No retries.
[[ "$phase" =~ ^wy-[0-9]{10}(-[a-z0-9]{1,12})?$ ]] || { usage; exit 2; }
[[ -n "$image" ]] || { usage; exit 2; }
kmode=0
if [[ -n "$kernel" || -n "$kernel_all" ]]; then
    [[ -n "$kernel" && -n "$kernel_all" ]] || { echo "REFUSED: --kernel and --kernel-all are all-or-none" >&2; usage; exit 2; }
    [[ "$phase" =~ ^wy-[0-9]{10}-kern[a-z0-9]{0,8}$ ]] || { echo "REFUSED: kernel-candidate stage requires a wy-<stamp>-kern* phase" >&2; usage; exit 2; }
    kmode=1
fi
[[ "$clone_timeout" =~ ^[0-9]+$ ]] && (( clone_timeout >= 5 && clone_timeout <= 300 )) || { usage; exit 2; }
[[ "$boot_timeout"  =~ ^[0-9]+$ ]] && (( boot_timeout  >= 10 && boot_timeout  <= 600 )) || { usage; exit 2; }
# stage/boot refuse without the explicit Codex-approval flag.
if [[ $sub != dry-run && $approve -ne 1 ]]; then
    echo "REFUSED: $sub requires explicit --approve (Codex budget not granted by this script)" >&2; exit 2
fi
if [[ $sub == boot ]]; then
    echo "REFUSED: BOOT_NOT_IMPLEMENTED — this wrapper is staging-only readiness; boot requires a separately reviewed dedicated Wyoming QEMU harness. --boot-cmd is never executed." >&2; exit 2
fi

sha() { sha256sum "$1" 2>/dev/null | cut -d ' ' -f1; }
man_entry() {  # $1=role $2=path -> sha from manifest TSV (role\tpath\tsha), realpath match
    local rp l r p s
    rp=$(realpath -m -- "$2")
    while IFS=$'\t' read -r r p s || [[ -n "${r:-}" ]]; do
        [[ "$r" == "$1" && -n "$p" && "$(realpath -m -- "$p")" == "$rp" ]] && { echo "$s"; return 0; }
    done < "$manifest"
    return 1
}

# ---- inputs + manifest hashes (fail closed: 3 missing, 1 mismatch) ----------
manifest=${manifest:-$OUT/manifest.tsv}
test -f "$manifest" || { echo "BLOCKER: manifest missing: $manifest" >&2; exit 3; }
[[ ! -L "$image" ]] || { echo "FAIL-CLOSED: symlink image input refused" >&2; exit 1; }
test -f "$image"    || { echo "BLOCKER: image missing: $image" >&2; exit 3; }
test -f "$binary"   || { echo "BLOCKER: binary missing: $binary (rebuild under authorization)" >&2; exit 3; }
test -x "$fstool"   || { echo "BLOCKER: fstool missing: $fstool" >&2; exit 3; }
test -x "$clone"    || { echo "BLOCKER: clone tool missing: $clone" >&2; exit 3; }
img_want=$(man_entry image "$image")   || { echo "BLOCKER: image not in manifest" >&2; exit 3; }
bin_want=$(man_entry binary "$binary") || { echo "BLOCKER: binary not in manifest" >&2; exit 3; }
img_got=$(sha "$image"); bin_got=$(sha "$binary")
[[ "$img_got" == "$img_want" ]] || { echo "FAIL-CLOSED: image hash $img_got != manifest $img_want" >&2; exit 1; }
[[ "$bin_got" == "$bin_want" ]] || { echo "FAIL-CLOSED: binary hash $bin_got != manifest $bin_want" >&2; exit 1; }
[[ "$bin_want" == "$WY_BINARY_SHA" ]] || { echo "FAIL-CLOSED: manifest binary sha contradicts pinned evidence $WY_BINARY_SHA" >&2; exit 1; }
# kernel pair: every check runs BEFORE the phase claim; never call in a subshell.
kin() {  # $1=role $2=path $3=pin $4=out-var
    local w g
    [[ ! -L "$2" ]] || { echo "FAIL-CLOSED: symlink $1 input refused" >&2; exit 1; }
    test -f "$2" || { echo "BLOCKER: $1 missing: $2" >&2; exit 3; }
    [[ "$(realpath -- "$2")" == "$(realpath -s -- "$2")" ]] || { echo "FAIL-CLOSED: $1 path alias (symlinked parent) refused" >&2; exit 1; }
    [[ "$(stat -c %h -- "$2")" == 1 ]] || { echo "FAIL-CLOSED: hard-linked $1 input refused" >&2; exit 1; }
    w=$(man_entry "$1" "$2") || { echo "BLOCKER: $1 not in manifest" >&2; exit 3; }
    g=$(sha "$2")
    [[ "$g" == "$w" ]] || { echo "FAIL-CLOSED: $1 hash $g != manifest $w" >&2; exit 1; }
    [[ "$w" == "$3" ]] || { echo "FAIL-CLOSED: manifest $1 sha contradicts pinned candidate $3" >&2; exit 1; }
    printf -v "$4" '%s' "$g"
}
k_got=""; ka_got=""
if [[ $kmode -eq 1 ]]; then
    kin kernel "$kernel" "$KERNEL_SHA" k_got
    kin kernel-all "$kernel_all" "$KERNEL_ALL_SHA" ka_got
    kid=$(stat -c '%d:%i' -- "$kernel"); kaid=$(stat -c '%d:%i' -- "$kernel_all")
    for f in "$image" "$binary" "$kernel_all" "$orig_iso" "$bad_iso" "$armB_iso"; do
        [[ -e "$f" ]] || continue
        fid=$(stat -L -c '%d:%i' -- "$f")
        [[ "$f" != "$kernel_all" && "$fid" == "$kaid" ]] && { echo "FAIL-CLOSED: kernel-all inode alias with $f" >&2; exit 1; }
        [[ "$fid" != "$kid" ]] || { echo "FAIL-CLOSED: kernel inode alias with $f" >&2; exit 1; }
    done
fi

# ---- phase-ID claim: atomic mkdir, claimed ONCE, retry refuses (exit 4) -----
run=$OUT/$phase
mkdir -p "$OUT"
mkdir "$run" 2>/dev/null || { echo "REFUSED: phase $phase already claimed; no retries" >&2; exit 4; }
res=$run/results.tsv
printf 'key\tvalue\n' >"$res"
rec() { printf '%s\t%s\n' "$1" "${2:-}" >>"$res"; printf '%s: %s\n' "$1" "${2:-}"; }
rec "phase" "$phase"; rec "sub" "$sub"; rec "image" "$image"; rec "image_sha" "$img_got"
rec "binary" "$binary"; rec "binary_sha" "$bin_got"; rec "guest_path" "$GUEST_BIN"
if [[ $kmode -eq 1 ]]; then
    rec "stage_mode" "app+kernel-candidate"
    rec "kernel" "$kernel"; rec "kernel_sha" "$k_got"; rec "kernel_guest_path" "$GUEST_KERNEL"
    rec "kernel_all" "$kernel_all"; rec "kernel_all_sha" "$ka_got"; rec "kernel_all_guest_path" "$GUEST_KERNEL_ALL"
    rec "kernel.source_identity" "$KERNEL_SRC_ID (pinned metadata only; not admission, not guest PASS)"
fi

# ---- protected images: pinned sha enforced whenever the file EXISTS; only a
# missing file skips (recorded). WS_* may redirect the PATH but never the pin.
prot_rc=0
declare -A PROT_PRESENT=()
prot() {  # $1=name $2=path $3=pinned-sha
    if [[ ! -e "$2" ]]; then PROT_PRESENT[$1]=0; rec "protected.$1" "absent"; return 0; fi
    PROT_PRESENT[$1]=1
    local s; s=$(sha "$2")
    if [[ "$s" == "$3" ]]; then rec "protected.$1.sha" "ok"; else rec "protected.$1.sha" "MISMATCH:$s"; return 1; fi
}
prot orig_iso "$orig_iso" "$ORIG_ISO_SHA" || prot_rc=1
prot bad_image "$bad_iso" "$BAD_ISO_SHA" || prot_rc=1
prot armB "$armB_iso" "$ARM_B_SHA" || prot_rc=1
[[ $prot_rc -eq 0 ]] || { rec "verdict" "PROTECTED_IMAGE_MISMATCH"; exit 1; }

# after-hash receipt keys: recorded on the stage post-check AND by the EXIT
# trap on every failure path, so receipts disclose after-hash state instead of
# silently skipping it when a stage aborts mid-flight.
POSTHASH_DONE=0
protafter() {  # $1=name $2=path $3=pinned-sha
    if [[ ! -e "$2" ]]; then
        if [[ ${PROT_PRESENT[$1]:-0} -eq 1 ]]; then rec "protected.$1.after" "DISAPPEARED"; return 1; fi
        rec "protected.$1.after" "absent"; return 0
    fi
    local s; s=$(sha "$2")
    if [[ "$s" == "$3" ]]; then rec "protected.$1.after.sha" "ok"; else rec "protected.$1.after.sha" "MISMATCH:$s"; return 1; fi
}

# ---- owned-process cleanup: setsid PGID, bounded TERM->KILL, never global ----
RFS_PGID=""; BOOT_PGID=""; MNT=""; declare -a MNTS=()
reap() {  # $1=pgid; TERM -> KILL -> bounded wait; only our own groups
    local pg=$1 i
    [[ -z "$pg" ]] && return 0
    kill -TERM -- "-$pg" 2>/dev/null || true
    for i in 1 2 3; do kill -0 -- "-$pg" 2>/dev/null || return 0; sleep 1; done
    kill -KILL -- "-$pg" 2>/dev/null || true
    for i in 1 2 3; do kill -0 -- "-$pg" 2>/dev/null || return 0; sleep 1; done
}
cleanup() {
    local m
    for m in "${MNTS[@]}"; do
        $mnt_cmd -q "$m" 2>/dev/null && timeout --kill-after=2 5 "$um_cmd" -u "$m" 2>/dev/null || true
    done
    reap "$BOOT_PGID"; BOOT_PGID=""
    reap "$RFS_PGID"; RFS_PGID=""
    # cleanup receipt: if we claimed a phase and the stage post-check never
    # ran, disclose protected after-hashes now — failure paths never skip them.
    if [[ -n "${res:-}" && -f "$res" && $POSTHASH_DONE -eq 0 ]]; then
        POSTHASH_DONE=1
        if [[ -n "${img_got:-}" ]]; then
            ia=$(sha "$image")
            [[ "$ia" == "$img_got" ]] && rec "input_image.after" "ok" || rec "input_image.after" "MUTATED:$img_got->$ia"
        fi
        if [[ -n "${k_got:-}" ]]; then
            ia=$(sha "$kernel")
            [[ "$ia" == "$k_got" ]] && rec "input_kernel.after" "ok" || rec "input_kernel.after" "MUTATED:$k_got->$ia"
            ia=$(sha "$kernel_all")
            [[ "$ia" == "$ka_got" ]] && rec "input_kernel_all.after" "ok" || rec "input_kernel_all.after" "MUTATED:$ka_got->$ia"
        fi
        protafter orig_iso "$orig_iso" "$ORIG_ISO_SHA" || true
        protafter bad_image "$bad_iso" "$BAD_ISO_SHA" || true
        protafter armB "$armB_iso" "$ARM_B_SHA" || true
    fi
}
trap cleanup EXIT
trap 'cleanup; exit 130' INT
trap 'cleanup; exit 143' TERM

# ---- read-only walk via the hash-recomputing clone tool; scratch alias guard
walk() {  # $1=src image, $2=log path -> reader exit code (9 = guard trip)
    local src=$1 log=$2 rc=0 scratch sp sd so sb sid soid sbid scid s0 s1
    scratch="$log.img"
    sp=$(realpath -m -- "$scratch"); sd=$(realpath -- "$src")
    so=$(realpath -m -- "$orig_iso"); sb=$(realpath -m -- "$bad_iso"); sa=$(realpath -m -- "$armB_iso")
    [[ "$sp" != "$sd" && "$sp" != "$so" && "$sp" != "$sb" && "$sp" != "$sa" ]] || { rec "walk.scratch_path_alias" "$sp"; return 9; }
    (set -o noclobber; : > "$scratch") 2>/dev/null || return 9   # exclusive create
    truncate -s 320M "$scratch" || return 9
    sid=$(stat -c '%d:%i' "$src"); soid=$(stat -c '%d:%i' "$orig_iso" 2>/dev/null || echo none)
    sbid=$(stat -c '%d:%i' "$bad_iso" 2>/dev/null || echo none); said=$(stat -c '%d:%i' "$armB_iso" 2>/dev/null || echo none)
    scid=$(stat -c '%d:%i' "$scratch")
    [[ "$scid" != "$sid" && "$scid" != "$soid" && "$scid" != "$sbid" && "$scid" != "$said" ]] || { rec "walk.scratch_inode_alias" "$scid"; return 9; }
    s0=$(sha "$src")
    timeout --kill-after=5 "$clone_timeout" "$clone" "$src" "$scratch" >"$log" 2>&1 || rc=$?
    rm -f -- "$scratch"
    s1=$(sha "$src")
    if [[ "$s0" != "$s1" ]]; then rec "walk.src_changed" "$src: $s0 -> $s1"; [[ $rc -eq 0 ]] && rc=9; fi
    return $rc
}

# ---- exact plan (printed by dry-run; executed only by stage/boot) -----------
plan() {
    printf 'PLAN (%s):\n' "$sub"
    printf '  1. cp -a --reflink=auto %s -> %s/wy-stage.iso   (input NEVER written)\n' "$image" "$run"
    printf '  2. copy guards: distinct inode + sha == manifest (%s)\n' "$img_want"
    printf '  3. baseline read-only walk: timeout %s %s <copy> <scratch.img> (exclusive scratch, path+inode alias guards)\n' "$clone_timeout" "$clone"
    printf '  4. mount RW (owned setsid PGID): setsid %s --no-daemon <copy> %s/mnt\n' "$fstool" "$run"
    printf '  5. insert ONLY %s from %s (sha-verified), chmod 755, cmp + hash readback\n' "$GUEST_BIN" "$binary"
    if [[ $kmode -eq 1 ]]; then
        printf '  5k. replace ONLY existing %s from %s (pin %s) and %s from %s (pin %s), chmod 644, cmp + pin readback\n' \
            "$GUEST_KERNEL" "$kernel" "$KERNEL_SHA" "$GUEST_KERNEL_ALL" "$kernel_all" "$KERNEL_ALL_SHA"
    fi
    printf '  6. unmount (timeout, retry once), wait writer exit, reap owned PGID only\n'
    printf '  7. post-walk read-only verify + input-image after-hash + protected-image after-hashes\n'
}

if [[ $sub == dry-run ]]; then
    plan | tee "$run/plan.txt"
    rec "verdict" "DRY_RUN_OK_NO_WRITES_BEYOND_OUT"
    exit 0
fi

# ---- stage: disposable copy ONLY; stop-before-mount on baseline failure -----
img=$run/wy-stage.iso
cp -a --reflink=auto "$image" "$img" || { rec "verdict" "COPY_FAIL"; exit 1; }
[[ -f "$img" && ! -L "$img" ]] || { rec "verdict" "COPY_NOT_REGULAR"; exit 1; }
i0=$(stat -c '%d:%i' "$image"); i1=$(stat -c '%d:%i' "$img")
[[ "$i0" != "$i1" ]] || { rec "verdict" "COPY_INODE_ALIAS_GUARD"; exit 1; }
[[ "$(sha "$img")" == "$img_want" ]] || { rec "verdict" "COPY_NOT_BYTE_IDENTICAL"; exit 1; }
rec "copy.inode_distinct" "yes"; rec "copy.sha" "$img_want"

walk "$img" "$run/baseline-clone.log"; rc=$?
rec "baseline_reader_rc" "$rc"
if [[ $rc -ne 0 ]]; then rec "verdict" "BASELINE_FAIL_STOPPED_BEFORE_MOUNT"; exit 1; fi

mnt=$run/mnt; mkdir -p "$mnt"; MNT=$mnt; MNTS+=("$mnt")
setsid "$fstool" --no-daemon "$img" "$mnt" >"$run/redoxfs.log" 2>&1 & RFS_PGID=$!
mounted=0
for i in $(seq 1 20); do $mnt_cmd -q "$mnt" && { mounted=1; break; }; sleep 1; done
if [[ $mounted -ne 1 ]]; then rec "verdict" "MOUNT_FAIL_STOPPED"; exit 1; fi

bsha=$(sha "$binary")
[[ "$bsha" == "$WY_BINARY_SHA" ]] || { rec "verdict" "BINARY_SHA_GUARD"; exit 1; }
# guest-path escape guard BEFORE any copy: the destination must resolve INSIDE
# the canonical mount. A symlink at the guest path (or a symlinked parent dir)
# pointing outside the mount — e.g. at a host file — refuses with no write.
canon_mnt=$(realpath -- "$mnt") || { rec "verdict" "MOUNT_PATH_UNRESOLVABLE"; exit 1; }
dest_real=$(realpath -m -- "$mnt/$GUEST_BIN" 2>/dev/null || echo UNRESOLVED)
if [[ -L "$mnt/$GUEST_BIN" || "$dest_real" != "$canon_mnt/"* ]]; then
    rec "insert.path_escape" "$mnt/$GUEST_BIN -> $dest_real"
    rec "verdict" "INSERT_PATH_ESCAPE"; exit 1
fi
if [[ $kmode -eq 1 ]]; then
    for g in "$GUEST_KERNEL" "$GUEST_KERNEL_ALL"; do
        dest_real=$(realpath -m -- "$mnt/$g" 2>/dev/null || echo UNRESOLVED)
        if [[ -L "$mnt/$g" || "$dest_real" != "$canon_mnt/"* ]]; then
            rec "insert.path_escape" "$mnt/$g -> $dest_real"
            rec "verdict" "INSERT_PATH_ESCAPE"; exit 1
        fi
        [[ -f "$mnt/$g" ]] || { rec "insert.$g.prior" "absent"; rec "verdict" "KERNEL_GUEST_PATH_ABSENT"; exit 1; }
        rec "insert.$g.prior_sha" "$(sha "$mnt/$g")"
    done
    [[ "$(sha "$kernel")" == "$KERNEL_SHA" && "$(sha "$kernel_all")" == "$KERNEL_ALL_SHA" ]] \
        || { rec "verdict" "KERNEL_SHA_GUARD"; exit 1; }
fi
mkdir -p "$mnt/usr/bin"
cp "$binary" "$mnt/$GUEST_BIN"; cp_rc=$?
chmod 755 "$mnt/$GUEST_BIN"; chmod_rc=$?
cmp "$binary" "$mnt/$GUEST_BIN"; cmp_rc=$?
rec "insert.cp_rc" "$cp_rc"; rec "insert.chmod_rc" "$chmod_rc"; rec "insert.cmp_rc" "$cmp_rc"
rec "insert.mounted_sha" "$(sha "$mnt/$GUEST_BIN")"
[[ "$cp_rc" -eq 0 && "$chmod_rc" -eq 0 && "$cmp_rc" -eq 0 ]] || { rec "verdict" "INSERT_FAIL"; exit 1; }
kins() {  # $1=key $2=src $3=guest $4=pin
    local c m h s
    cp -- "$2" "$mnt/$3"; c=$?
    chmod 644 "$mnt/$3"; m=$?
    cmp -- "$2" "$mnt/$3"; h=$?
    s=$(sha "$mnt/$3")
    rec "insert.$1.cp_rc" "$c"; rec "insert.$1.chmod_rc" "$m"; rec "insert.$1.cmp_rc" "$h"
    rec "insert.$1.mounted_sha" "$s"
    [[ $c -eq 0 && $m -eq 0 && $h -eq 0 && "$s" == "$4" ]]
}
if [[ $kmode -eq 1 ]]; then
    { kins kernel "$kernel" "$GUEST_KERNEL" "$KERNEL_SHA" \
        && kins kernel-all "$kernel_all" "$GUEST_KERNEL_ALL" "$KERNEL_ALL_SHA"; } \
        || { rec "verdict" "INSERT_FAIL"; exit 1; }
fi

timeout --kill-after=2 5 "$um_cmd" -u "$mnt"; um_rc=$?
if [[ $um_rc -ne 0 ]]; then timeout --kill-after=2 5 "$um_cmd" -u "$mnt"; um_rc=$?; fi
rec "umount_rc" "$um_rc"
for i in $(seq 1 10); do kill -0 "$RFS_PGID" 2>/dev/null || break; sleep 1; done
if kill -0 "$RFS_PGID" 2>/dev/null; then reap "$RFS_PGID"; RFS_PGID=""; rec "verdict" "WRITER_HANG_STOPPED"; exit 1; fi
# keep RFS_PGID set: the writer may have exited but left children in its
# session — the EXIT trap's reap() must still kill the whole owned group.
wait "$RFS_PGID"; writer_rc=$?
rec "writer_rc" "$writer_rc"
if $mnt_cmd -q "$mnt"; then rec "verdict" "LEFTOVER_MOUNT_FAIL"; exit 1; fi
MNT=""

walk "$img" "$run/post-clone.log"; rc=$?
rec "post_reader_rc" "$rc"
# input image hash AFTER stage — the copied-source walk alone is not enough:
# a stub writer that mutated the INPUT is disclosed, never silently skipped.
img_after=$(sha "$image")
rec "input_image.after_sha" "$img_after"
input_mutated=0
[[ "$img_after" == "$img_got" ]] || { rec "input_image.after" "MUTATED:$img_got->$img_after"; input_mutated=1; }
if [[ $kmode -eq 1 ]]; then
    ka=$(sha "$kernel"); rec "input_kernel.after_sha" "$ka"
    [[ "$ka" == "$k_got" ]] || { rec "input_kernel.after" "MUTATED:$k_got->$ka"; input_mutated=1; }
    ka=$(sha "$kernel_all"); rec "input_kernel_all.after_sha" "$ka"
    [[ "$ka" == "$ka_got" ]] || { rec "input_kernel_all.after" "MUTATED:$ka_got->$ka"; input_mutated=1; }
fi
POSTHASH_DONE=1
protafter orig_iso "$orig_iso" "$ORIG_ISO_SHA" || prot_rc=1
protafter bad_image "$bad_iso" "$BAD_ISO_SHA" || prot_rc=1
protafter armB "$armB_iso" "$ARM_B_SHA" || prot_rc=1
if [[ $rc -eq 0 && $writer_rc -eq 0 && $um_rc -eq 0 && $prot_rc -eq 0 && $input_mutated -eq 0 ]]; then
    rec "staged_image" "$img"; rec "verdict" "STAGE_PASS"
else
    rec "verdict" "STAGE_FAIL"; exit 1
fi
echo "run complete — receipts in $run"
