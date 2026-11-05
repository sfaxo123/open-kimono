#!/usr/bin/env bash
# E1-STAGING-CHECK: host-only, NO-BOOT, 2-arm disposable-copy staging experiment
# (Codex-approved plan 4:24 AM PT + Codex in-flight script review hardening).
# ARM A = mount/unmount only; ARM B = mount + insert usr/bin/net-probe only.
# One attempt per arm, no retries; baseline walk failure stops the WHOLE experiment.
# Original ISO and the old bad image are read-only inputs; all writes land in a
# fresh out/e1-staging-check/<run>/ dir. No global pkill — only owned PIDs die.
set -uo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
forks=${REDOX_FORKS:-$HOME/Developer/forks/redox}
redox="$forks/redox"
orig_iso="$redox/build/aarch64/desktop-minimal/e1-live.iso"
ORIG_ISO_SHA=c02012eaa182800cc7ed937e0e3ebf7b436f2af3e9dacb5c57bb00c89df361bc
binary="$repo/out/e1net-native/redox/aarch64-unknown-redox/release/net-probe"
BINARY_SHA=9ec92a9e0e14819bb828a92b1e79724820c1fb121cf40a2c330bd3a230afd7f7
bad_iso="$repo/out/e1net-native/e1net-probe.iso"          # preserve untouched
BAD_ISO_SHA=87ae0aa883724a60b7247ca0f5fb570684fc92610caa31d13b7f81e2767b942f
fstool="$redox/build/fstools/bin/redoxfs"
clone="$redox/build/fstools/bin/redoxfs-clone"

# A fixed, approved phase ID is a one-time claim, not a fresh retry timestamp.
phase=${1:-}
[[ "$phase" =~ ^[a-zA-Z0-9][a-zA-Z0-9_-]{0,63}$ ]] || { echo "usage: $0 <approved-phase-id>" >&2; exit 2; }
mkdir -p "$repo/out/e1-staging-check"
run="$repo/out/e1-staging-check/$phase"
mkdir "$run" 2>/dev/null || { echo "phase already claimed; refusing another pair of copies" >&2; exit 4; }
res="$run/results.tsv"
printf 'key\tvalue\n' >"$res"
rec() { printf '%s\t%s\n' "$1" "${2:-}" >>"$res"; printf '%s: %s\n' "$1" "${2:-}"; }
sha() { sha256sum "$1" 2>/dev/null | cut -d ' ' -f1; }

# ---- owned-process cleanup: per-arm setsid PGID + every mount we opened -----
RFS_PGID=""; MNT=""; declare -a MNTS=()
reap() {  # $1=pgid; TERM -> KILL -> bounded wait; never global
    local pg=$1 i
    [[ -z "$pg" ]] && return 0
    kill -TERM -- "-$pg" 2>/dev/null || true
    for i in 1 2 3; do kill -0 -- "-$pg" 2>/dev/null || return 0; sleep 1; done
    kill -KILL -- "-$pg" 2>/dev/null || true
    for i in 1 2 3; do kill -0 -- "-$pg" 2>/dev/null || return 0; sleep 1; done
}
cleanup() {  # works even on the mount-not-ready path; retries unmount for
             # any mount we still own (leftover from hang/failed umount)
    local m
    for m in "${MNTS[@]}"; do
        mountpoint -q "$m" 2>/dev/null && timeout --kill-after=2 5 fusermount3 -u "$m" 2>/dev/null || true
    done
    reap "$RFS_PGID"; RFS_PGID=""
}
trap cleanup EXIT
trap 'cleanup; exit 130' INT
trap 'cleanup; exit 143' TERM

# ---- fresh read-only tree walk: SOURCE first, disposable OUTPUT second -----
# (the clone tool's own banner is misleading — this order is the guarded one)
walk() {  # $1=src image, $2=log path -> reader exit code (9 = guard trip)
    local src=$1 log=$2 rc=0 scratch
    scratch="$log.img"
    # explicit guard: scratch distinct resolved path AND distinct dev:inode
    # vs the walk source, the original ISO, and the old bad image
    local sp sd so sb sid soid sbid scid
    sp=$(realpath -m "$scratch"); sd=$(realpath "$src")
    so=$(realpath "$orig_iso"); sb=$(realpath "$bad_iso")
    [[ "$sp" != "$sd" && "$sp" != "$so" && "$sp" != "$sb" ]] || { rec "walk.scratch_path_alias" "$sp"; return 9; }
    (set -o noclobber; : > "$scratch") 2>/dev/null || return 9
    truncate -s 320M "$scratch" || return 9
    sid=$(stat -c '%d:%i' "$src"); soid=$(stat -c '%d:%i' "$orig_iso")
    sbid=$(stat -c '%d:%i' "$bad_iso"); scid=$(stat -c '%d:%i' "$scratch")
    [[ "$scid" != "$sid" && "$scid" != "$soid" && "$scid" != "$sbid" ]] || { rec "walk.scratch_inode_alias" "$scid"; return 9; }
    # source hash must be unchanged across the walk (per-walk, not just global)
    local s0 s1; s0=$(sha "$src")
    timeout --kill-after=5 40 "$clone" "$src" "$scratch" >"$log" 2>&1 || rc=$?
    rm -f -- "$scratch"
    s1=$(sha "$src")
    if [[ "$s0" != "$s1" ]]; then
        rec "walk.src_changed" "$src: $s0 -> $s1"
        [[ $rc -eq 0 ]] && rc=9
    fi
    return $rc
}
hash_err() {  # actual hash-error text (block numbers etc.) from a clone log
    grep -iE 'hash|block|corrupt|error' "$1" 2>/dev/null | head -5 | tr '\n' ' | '
}

# arm return codes: 0 pass | 1 BASELINE fail (STOP WHOLE EXPERIMENT) | 2 other stop/fail
arm() {  # $1=A|B
    local name=$1 img="$run/arm$1.iso" mnt="$run/mnt-$1" rc writer_rc=0 cp_rc=0 chmod_rc=0 cmp_rc=0 um_rc=0
    rec "arm$name.copy_source" "$orig_iso"
    cp -a --reflink=auto "$orig_iso" "$img" || { rec "arm$name.verdict" "COPY_FAIL"; return 2; }
    # byte-identical 256MiB copy guard: never resized; no path/inode aliasing
    local sz; sz=$(stat -c %s "$img")
    [[ "$sz" == 268435456 ]] || { rec "arm$name.copy_size" "$sz"; rec "arm$name.verdict" "COPY_SIZE_GUARD"; return 2; }
    local i0 i1; i0=$(stat -c %i "$orig_iso"); i1=$(stat -c %i "$img")
    [[ "$i0" != "$i1" ]] || { rec "arm$name.verdict" "COPY_INODE_ALIAS_GUARD"; return 2; }
    rec "arm$name.copy_size" "$sz"; rec "arm$name.copy_inodes_distinct" "yes"
    rec "arm$name.copy_sha_pre" "$(sha "$img")"
    [[ "$(sha "$img")" == "$ORIG_ISO_SHA" ]] || { rec "arm$name.verdict" "COPY_NOT_BYTE_IDENTICAL"; return 2; }

    # 1. baseline fresh read-only walk — must PASS before any mount
    walk "$img" "$run/arm${name}-baseline-clone.log"; rc=$?
    rec "arm$name.baseline_reader_rc" "$rc"
    [[ $rc -ne 0 ]] && rec "arm$name.baseline_err_text" "$(hash_err "$run/arm${name}-baseline-clone.log")"
    if [[ $rc -ne 0 ]]; then rec "arm$name.verdict" "BASELINE_FAIL_STOPPED"; return 1; fi

    # 2. RW mount, own setsid PGID, own mount point
    mkdir -p "$mnt"; MNT="$mnt"; MNTS+=("$mnt")
    setsid "$fstool" --no-daemon "$img" "$mnt" >"$run/arm${name}-redoxfs.log" 2>&1 & RFS_PGID=$!
    local i mounted=0
    for i in $(seq 1 20); do mountpoint -q "$mnt" && { mounted=1; break; }; sleep 1; done
    if [[ $mounted -ne 1 ]]; then
        rec "arm$name.verdict" "MOUNT_FAIL_STOPPED"   # MNT retained: trap retries unmount+reap
        return 2
    fi

    # 3. ARM B only: verify binary sha BEFORE copy, insert ONLY usr/bin/net-probe,
    #    chmod, then verify mounted-copy hash + cmp readback
    if [[ "$name" == B ]]; then
        local bsha; bsha=$(sha "$binary")
        [[ "$bsha" == "$BINARY_SHA" ]] || { rec "arm$name.binary_sha_guard" "$bsha"; rec "arm$name.verdict" "BINARY_SHA_GUARD"; return 2; }
        rec "arm$name.binary_sha_pre_copy" "$bsha"
        cp "$binary" "$mnt/usr/bin/net-probe"; cp_rc=$?
        chmod 755 "$mnt/usr/bin/net-probe"; chmod_rc=$?
        cmp "$binary" "$mnt/usr/bin/net-probe"; cmp_rc=$?
        local msha; msha=$(sha "$mnt/usr/bin/net-probe")
        rec "arm$name.cp_rc" "$cp_rc"; rec "arm$name.chmod_rc" "$chmod_rc"
        rec "arm$name.cmp_readback_rc" "$cmp_rc"
        rec "arm$name.mounted_copy_sha" "$msha"
        [[ "$msha" == "$BINARY_SHA" ]] || rec "arm$name.mounted_copy_sha_guard" "MISMATCH"
    fi

    # 4. clean unmount (retry once on failure; MNT retained for trap either way),
    #    then WAIT for the writer process to exit
    timeout --kill-after=2 5 fusermount3 -u "$mnt"; um_rc=$?
    if [[ $um_rc -ne 0 ]]; then
        rec "arm$name.umount_rc" "$um_rc (retrying)"
        timeout --kill-after=2 5 fusermount3 -u "$mnt"; um_rc=$?
        rec "arm$name.umount_retry_rc" "$um_rc"
    else
        rec "arm$name.umount_rc" 0
    fi
    for i in $(seq 1 10); do kill -0 "$RFS_PGID" 2>/dev/null || break; sleep 1; done
    if kill -0 "$RFS_PGID" 2>/dev/null; then
        rec "arm$name.writer_exit" "STILL_RUNNING_AFTER_UNMOUNT"
        reap "$RFS_PGID"; RFS_PGID=""
        mountpoint -q "$mnt" && rec "arm$name.leftover_mount" "$mnt" || rec "arm$name.leftover_mount" "none"
        rec "arm$name.verdict" "WRITER_HANG_STOPPED"; return 2   # MNT retained: trap retries unmount
    fi
    wait "$RFS_PGID"; writer_rc=$?; RFS_PGID=""
    rec "arm$name.writer_rc" "$writer_rc"
    if mountpoint -q "$mnt"; then
        rec "arm$name.leftover_mount" "$mnt"
        rec "arm$name.verdict" "LEFTOVER_MOUNT_FAIL"; return 2   # MNT retained: trap retries unmount
    fi
    MNT=""

    # 5. fresh read-only walk after writer exit
    walk "$img" "$run/arm${name}-post-clone.log"; rc=$?
    rec "arm$name.post_reader_rc" "$rc"
    [[ $rc -ne 0 ]] && rec "arm$name.post_err_text" "$(hash_err "$run/arm${name}-post-clone.log")"
    rec "arm$name.copy_sha_post" "$(sha "$img")"
    # PASS requires EVERY step clean: walks, writer, umount, AND cp/chmod/cmp all 0
    if [[ $rc -eq 0 && $writer_rc -eq 0 && $um_rc -eq 0 && $cp_rc -eq 0 && $chmod_rc -eq 0 && $cmp_rc -eq 0 ]]; then
        rec "arm$name.verdict" "PASS_IMAGE_CONSISTENT_ONLY"; return 0
    fi
    rec "arm$name.verdict" "FAIL"; return 2
}

# ---- preconditions + BEFORE hashes -----------------------------------------
test -f "$orig_iso" && test -f "$bad_iso" && test -x "$fstool" && test -x "$clone" && test -f "$binary" \
    || { echo "BLOCKER: missing input (iso/fstools/binary)" >&2; exit 3; }
# explicit disk budget: 2 ARM copies (256MiB each) + distinct 320MiB clone scratch
avail_kb=$(df --output=avail -k "$run" | tail -1 | tr -d ' ')
(( avail_kb >= 1300000 )) || { echo "BLOCKER: disk budget $avail_kb KB < 1.3 GiB" >&2; exit 3; }
rec "disk_avail_kb" "$avail_kb"
iso_before=$(sha "$orig_iso")
rec "orig_iso_sha_before" "$iso_before"
[[ "$iso_before" == "$ORIG_ISO_SHA" ]] || { echo "BLOCKER: original ISO hash mismatch" >&2; exit 1; }
bad_before=$(sha "$bad_iso")
rec "bad_image_sha_before" "$bad_before"
[[ "$bad_before" == "$BAD_ISO_SHA" ]] || { echo "BLOCKER: old bad image differs from pinned evidence" >&2; exit 1; }
rec "binary_sha" "$(sha "$binary")"

arm A; a_rc=$?
b_rc=255
if [[ $a_rc -ne 0 ]]; then
    rec "experiment" "STOPPED_ON_ARM_A_FAILURE — arm B NOT attempted"
else
    arm B; b_rc=$?
fi

# ---- AFTER hashes: original must be unchanged; bad image must be untouched --
iso_after=$(sha "$orig_iso")
rec "orig_iso_sha_after" "$iso_after"
[[ "$iso_after" == "$iso_before" ]] || { rec "experiment" "ORIGINAL_ISO_CHANGED"; echo "BLOCKER: ORIGINAL ISO CHANGED" >&2; exit 1; }
bad_after=$(sha "$bad_iso")
rec "bad_image_sha_after" "$bad_after"
[[ "$bad_after" == "$bad_before" ]] || { rec "experiment" "OLD_BAD_IMAGE_CHANGED"; echo "BLOCKER: OLD BAD IMAGE CHANGED" >&2; exit 1; }
rec "run_dir" "$run"
echo "run complete: A_rc=$a_rc B_rc=$b_rc — receipts saved in $run"
# nonzero exit if any arm failed; receipts already saved above
[[ $a_rc -eq 0 && $b_rc -eq 0 ]] || exit 1
