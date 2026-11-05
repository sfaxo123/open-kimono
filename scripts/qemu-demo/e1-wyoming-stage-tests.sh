#!/usr/bin/env bash
# FAKE-ONLY host tests for e1-wyoming-stage.sh: stub binaries + WS_* test seams.
# NO qemu, NO real images, NO mounts, NO network. Asserts exit codes + side effects:
# missing/bad phase-ID refused; consumed claim refused; hash mismatch fails closed;
# stage/boot refuse without --approve; dry-run creates claim but NO arm image;
# baseline-walk failure stops before mount; trap reaps stub children; no orphans.
set -uo pipefail
repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
S=$repo/scripts/qemu-demo/e1-wyoming-stage.sh
T=$(mktemp -d "${TMPDIR:-/tmp}/wy-stage-tests.XXXXXX")
trap 'rm -rf "$T"' EXIT
ORPHAN_SLEEP=$((900 + RANDOM))   # unique marker for the stub's orphan child
pass=0; fail=0
ok()  { printf 'PASS: %s\n' "$1"; pass=$((pass+1)); }
bad() { printf 'FAIL: %s — expected rc %s got %s\n' "$1" "$2" "$3"; fail=$((fail+1)); }
chk() { [[ "$3" == "$2" ]] && ok "$1" || bad "$1" "$2" "$3"; }
sha() { sha256sum "$1" | cut -d ' ' -f1; }
NOW=$(date +%s)

# ---- fakes: tiny image + binary, stub fstools, overridden protected images ----
IMG=$T/input.iso; head -c 4096 /dev/urandom >"$IMG"
BIN=$T/wyoming-probe; printf 'fake-wyoming-probe-bytes' >"$BIN"; chmod +x "$BIN"
IMG_SHA=$(sha "$IMG"); BIN_SHA=$(sha "$BIN")
printf 'image\t%s\t%s\nbinary\t%s\t%s\n' "$IMG" "$IMG_SHA" "$BIN" "$BIN_SHA" >"$T/manifest.tsv"
printf 'image\t%s\t%s\nbinary\t%s\t%s\n' "$IMG" "0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0" "$BIN" "$BIN_SHA" >"$T/manifest-bad.tsv"
cat >"$T/redoxfs" <<'EOF'
#!/usr/bin/env bash
# fake fstool: mark dir "mounted", spawn a child in our new session, exit 0
# (the EXIT trap must still reap that child after the writer exits)
touch "$3/.fake_mounted"
sleep "${WS_ORPHAN_SLEEP}" &
exit 0
EOF
cat >"$T/clone" <<'EOF'
#!/usr/bin/env bash
cp -- "$1" "$2" 2>/dev/null; exit 0
EOF
cat >"$T/clone-fail" <<'EOF'
#!/usr/bin/env bash
exit 9
EOF
printf '#!/usr/bin/env bash\n[[ -f "$2/.fake_mounted" ]]\n' >"$T/mountpoint"  # stateful fake
printf '#!/usr/bin/env bash\nrm -f -- "$2/.fake_mounted"\n' >"$T/umount"      # clears marker
chmod +x "$T/redoxfs" "$T/clone" "$T/clone-fail" "$T/mountpoint" "$T/umount"
# protected images: WS_* redirects PATHS only; pins stay enforced, so fakes
# point at ABSENT paths (recorded "absent") unless a test creates a mismatch.
cat >"$T/redoxfs-evil" <<'EOF'
#!/usr/bin/env bash
# fake fstool whose "image content" plants an escaping symlink at the guest path
touch "$3/.fake_mounted"
mkdir -p "$3/usr/bin"
ln -s "$WS_EVIL_TARGET" "$3/usr/bin/wyoming-probe"
exit 0
EOF
cat >"$T/umount-tamper" <<'EOF'
#!/usr/bin/env bash
# fake umount that mutates the INPUT image (stub-writer mutation negative)
rm -f -- "$2/.fake_mounted"
[[ -n "${WS_FAKE_TAMPER:-}" ]] && printf 'x' >>"$WS_FAKE_TAMPER"
exit 0
EOF
chmod +x "$T/redoxfs-evil" "$T/umount-tamper"

run() {  # every invocation runs with fake seams only; real forks path absent.
         # ${WS_*:-default} lets a caller subshell override one seam for a negative.
    WS_OUT=$T/out WS_FSTOOL=${WS_FSTOOL:-$T/redoxfs} WS_CLONE=${WS_CLONE:-$T/clone} \
    WS_MOUNTPOINT=$T/mountpoint WS_UMOUNT=${WS_UMOUNT:-$T/umount} \
    WS_WY_BINARY_SHA=${WS_WY_BINARY_SHA:-$BIN_SHA} WS_CLONE_TIMEOUT=5 \
    WS_ORIG_ISO=$T/absent-orig.iso WS_BAD_ISO=$T/absent-bad.iso \
    WS_ARM_B_ISO=${WS_ARM_B_ISO:-$T/absent-armB.iso} \
    WS_ORPHAN_SLEEP=$ORPHAN_SLEEP \
    REDOX_FORKS=$T/noforks bash "$S" "$@" --manifest "${MANIFEST:-$T/manifest.tsv}" --binary "$BIN"
}

# T1: missing phase-ID refused (exit 2, no side effects)
run dry-run --image "$IMG" >/dev/null 2>&1; chk "T1 missing phase-ID -> 2" 2 $?
# T2: malformed phase-ID (wrong namespace) refused
run dry-run --phase "net-probe-123" --image "$IMG" >/dev/null 2>&1; chk "T2 bad phase-ID ns -> 2" 2 $?
# T3: stage without --approve refused; claim NOT consumed
run stage --phase "wy-$NOW-t3" --image "$IMG" >/dev/null 2>&1; chk "T3 stage no --approve -> 2" 2 $?
[[ ! -e $T/out/wy-$NOW-t3 ]] && ok "T3b no claim consumed by refusal" || { printf 'FAIL: T3b claim created\n'; fail=$((fail+1)); }
# T4: boot refused even WITH --approve (boot disabled this phase)
run boot --phase "wy-$NOW-t4" --image "$IMG" --boot-cmd true >/dev/null 2>&1; chk "T4 boot no --approve -> 2" 2 $?
# T5: boot with --approve AND arbitrary --boot-cmd: BOOT_NOT_IMPLEMENTED before
# any side effect; sentinel proves the command NEVER ran; no claim, no copy.
printf 'sentinel' >"$T/boot-sentinel"
run boot --phase "wy-$NOW-t5" --image "$IMG" --approve --boot-cmd "rm -f $T/boot-sentinel" >"$T/t5.out" 2>&1
chk "T5 boot --approve + --boot-cmd -> 2" 2 $?
grep -q 'BOOT_NOT_IMPLEMENTED' "$T/t5.out" \
    && ok "T5b BOOT_NOT_IMPLEMENTED disclosed" || { printf 'FAIL: T5b no BOOT_NOT_IMPLEMENTED\n'; fail=$((fail+1)); }
[[ -f $T/boot-sentinel ]] && ok "T5c --boot-cmd never executed" || { printf 'FAIL: T5c boot-cmd ran\n'; fail=$((fail+1)); }
[[ ! -e $T/out/wy-$NOW-t5 ]] && ok "T5d no claim/copy on boot refusal" || { printf 'FAIL: T5d side effects\n'; fail=$((fail+1)); }
# T6: manifest hash mismatch fails closed; claim NOT consumed
MANIFEST=$T/manifest-bad.tsv run dry-run --phase "wy-$NOW-t6" --image "$IMG" >/dev/null 2>&1
chk "T6 hash mismatch -> 1" 1 $?
[[ ! -e $T/out/wy-$NOW-t6 ]] && ok "T6b no claim on fail-closed" || { printf 'FAIL: T6b claim created\n'; fail=$((fail+1)); }
# T7: dry-run passes, creates claim, writes NO arm image
run dry-run --phase "wy-$NOW-t7" --image "$IMG" >/dev/null 2>&1; chk "T7 dry-run -> 0" 0 $?
[[ -d $T/out/wy-$NOW-t7 ]] && ok "T7b claim created" || { printf 'FAIL: T7b no claim dir\n'; fail=$((fail+1)); }
n=$(find "$T/out/wy-$NOW-t7" -name '*.iso' | wc -l)
chk "T7c dry-run creates NO arm image" 0 "$n"
grep -q 'DRY_RUN_OK_NO_WRITES_BEYOND_OUT' "$T/out/wy-$NOW-t7/results.tsv" \
    && ok "T7d dry-run verdict recorded" || { printf 'FAIL: T7d verdict missing\n'; fail=$((fail+1)); }
# T8: consumed claim refused on retry (exit 4)
run dry-run --phase "wy-$NOW-t7" --image "$IMG" >/dev/null 2>&1; chk "T8 consumed claim -> 4" 4 $?
# T9: baseline walk failure stops BEFORE mount (no mnt dir, no insert)
WS_CLONE=$T/clone-fail WS_OUT=$T/out WS_FSTOOL=$T/redoxfs \
WS_MOUNTPOINT=$T/mountpoint WS_UMOUNT=$T/umount WS_WY_BINARY_SHA=$BIN_SHA WS_CLONE_TIMEOUT=5 \
WS_ORIG_ISO=$T/absent-orig.iso WS_BAD_ISO=$T/absent-bad.iso WS_ARM_B_ISO=$T/absent-armB.iso \
WS_ORPHAN_SLEEP=$ORPHAN_SLEEP REDOX_FORKS=$T/noforks \
bash "$S" stage --phase "wy-$NOW-t9" --image "$IMG" --manifest "$T/manifest.tsv" --binary "$BIN" --approve >/dev/null 2>&1
chk "T9 baseline fail -> 1" 1 $?
[[ ! -e $T/out/wy-$NOW-t9/mnt ]] && ok "T9b stopped before mount (no mnt dir)" || { printf 'FAIL: T9b mnt created\n'; fail=$((fail+1)); }
grep -q 'BASELINE_FAIL_STOPPED_BEFORE_MOUNT' "$T/out/wy-$NOW-t9/results.tsv" \
    && ok "T9c stop-before-mount verdict" || { printf 'FAIL: T9c verdict missing\n'; fail=$((fail+1)); }
grep -q 'protected.armB.after' "$T/out/wy-$NOW-t9/results.tsv" \
    && ok "T9d cleanup receipt discloses after-hashes on failure" || { printf 'FAIL: T9d after-hashes skipped\n'; fail=$((fail+1)); }
# T10: stage happy path with stubs: STAGE_PASS, guest binary inserted, input untouched
img_before=$(sha "$IMG")
run stage --phase "wy-$NOW-t10" --image "$IMG" --manifest "$T/manifest.tsv" --binary "$BIN" --approve >/dev/null 2>&1
chk "T10 stage --approve (stubs) -> 0" 0 $?
grep -q 'STAGE_PASS' "$T/out/wy-$NOW-t10/results.tsv" \
    && ok "T10b STAGE_PASS verdict" || { printf 'FAIL: T10b verdict missing\n'; fail=$((fail+1)); }
[[ -f $T/out/wy-$NOW-t10/mnt/usr/bin/wyoming-probe ]] && chk "T10c guest path inserted w/ sha" "$BIN_SHA" "$(sha "$T/out/wy-$NOW-t10/mnt/usr/bin/wyoming-probe")" \
    || { printf 'FAIL: T10c usr/bin/wyoming-probe missing\n'; fail=$((fail+1)); }
chk "T10d input image untouched" "$img_before" "$(sha "$IMG")"
# T11: trap reaped the stub's orphan child (bounded wait, then check)
reaped=0
for i in 1 2 3 4 5; do pgrep -f "sleep $ORPHAN_SLEEP" >/dev/null || { reaped=1; break; }; sleep 1; done
chk "T11 trap reaped stub child" 1 "$reaped"
# T12: final orphan sweep — nothing of ours left running
left=$(pgrep -cf "sleep $ORPHAN_SLEEP" || true)
chk "T12 no orphans at end" 0 "$left"
# T13: PRODUCTION pin contradiction despite correct manifest+file hashes.
# NO WS_WY_BINARY_SHA override here — the fake seam must not bypass the pin.
WS_OUT=$T/out WS_CLONE=$T/clone WS_MOUNTPOINT=$T/mountpoint WS_UMOUNT=$T/umount \
WS_FSTOOL=$T/redoxfs WS_CLONE_TIMEOUT=5 WS_ORIG_ISO=$T/absent-orig.iso WS_BAD_ISO=$T/absent-bad.iso \
WS_ARM_B_ISO=$T/absent-armB.iso REDOX_FORKS=$T/noforks \
bash "$S" dry-run --phase "wy-$NOW-t13" --image "$IMG" --manifest "$T/manifest.tsv" --binary "$BIN" >"$T/t13.out" 2>&1
chk "T13 manifest vs production pin -> 1" 1 $?
grep -q 'contradicts pinned evidence' "$T/t13.out" \
    && ok "T13b pin contradiction disclosed" || { printf 'FAIL: T13b no disclosure\n'; fail=$((fail+1)); }
[[ ! -e $T/out/wy-$NOW-t13 ]] && ok "T13c no claim on pin contradiction" || { printf 'FAIL: T13c claim\n'; fail=$((fail+1)); }
# T14: protected armB.iso present with wrong content -> fail closed
head -c 512 /dev/urandom >"$T/armB.iso"
run dry-run --phase "wy-$NOW-t14" --image "$IMG" >/dev/null 2>&1
chk "T14 default run (armB absent) -> 0" 0 $?
( WS_ARM_B_ISO=$T/armB.iso run dry-run --phase "wy-$NOW-t14b" --image "$IMG" ) >/dev/null 2>&1
chk "T14b armB mismatch -> 1" 1 $?
grep -q 'PROTECTED_IMAGE_MISMATCH' "$T/out/wy-$NOW-t14b/results.tsv" \
    && ok "T14c armB mismatch verdict" || { printf 'FAIL: T14c verdict missing\n'; fail=$((fail+1)); }
# T15: stub writer mutates the INPUT image -> after-hash discloses, STAGE_FAIL
( WS_UMOUNT=$T/umount-tamper WS_FAKE_TAMPER=$IMG \
  run stage --phase "wy-$NOW-t15" --image "$IMG" --approve ) >/dev/null 2>&1
chk "T15 input-mutation stage -> 1" 1 $?
grep -q 'input_image.after.*MUTATED' "$T/out/wy-$NOW-t15/results.tsv" \
    && ok "T15b input after-hash MUTATED disclosed" || { printf 'FAIL: T15b no mutation receipt\n'; fail=$((fail+1)); }
grep -q 'STAGE_FAIL' "$T/out/wy-$NOW-t15/results.tsv" \
    && ok "T15c STAGE_FAIL verdict" || { printf 'FAIL: T15c verdict missing\n'; fail=$((fail+1)); }
truncate -s 4096 "$IMG"   # undo the fake append; manifest sha must match again
# T16: escaping guest-path symlink planted by fake image -> refuse BEFORE copy;
# host sentinel untouched
printf 'host-sentinel-untouched' >"$T/host-sentinel"
( WS_FSTOOL=$T/redoxfs-evil WS_EVIL_TARGET=$T/host-sentinel \
  run stage --phase "wy-$NOW-t16" --image "$IMG" --approve ) >/dev/null 2>&1
chk "T16 guest-path symlink escape -> 1" 1 $?
grep -q 'INSERT_PATH_ESCAPE' "$T/out/wy-$NOW-t16/results.tsv" \
    && ok "T16b INSERT_PATH_ESCAPE verdict" || { printf 'FAIL: T16b verdict missing\n'; fail=$((fail+1)); }
chk "T16c host sentinel untouched" 'host-sentinel-untouched' "$(cat "$T/host-sentinel")"
[[ ! -e $T/out/wy-$NOW-t16/mnt/usr/bin/wyoming-probe || -L $T/out/wy-$NOW-t16/mnt/usr/bin/wyoming-probe ]] \
    && ok "T16d no bytes copied through the escape" || { printf 'FAIL: T16d copy through symlink\n'; fail=$((fail+1)); }

# T17: cp -a must never preserve a manifest-listed input symlink.
ln -s "$IMG" "$T/input-link.iso"
run stage --phase "wy-$NOW-t17" --image "$T/input-link.iso" --approve >"$T/t17.out" 2>&1
chk "T17 input symlink refused" 1 $?
[[ ! -e $T/out/wy-$NOW-t17 ]] && ok "T17b no copy/claim for symlink" || bad "T17b claim" 0 1
# T18: a protected file present initially cannot disappear and still pass.
# Fake sha for ONLY this tiny protected fixture; other files use real sha256sum.
mkdir -p "$T/hashbin"
printf 'protected-fixture' >"$T/protected-armB.iso"
cat >"$T/hashbin/sha256sum" <<'EOF'
#!/usr/bin/env bash
if [[ "$1" == "$WS_FAKE_PROTECTED" && -f "$1" ]]; then
 printf '%s  %s\n' c91ab5338659fa6eef6a553c612ecac998a0046473292272328351e5b2f11e7c "$1"
else /usr/bin/sha256sum "$@"; fi
EOF
cat >"$T/umount-remove" <<'EOF'
#!/usr/bin/env bash
rm -f -- "$2/.fake_mounted" "$WS_FAKE_PROTECTED"
EOF
chmod +x "$T/hashbin/sha256sum" "$T/umount-remove"
( PATH="$T/hashbin:$PATH" WS_FAKE_PROTECTED="$T/protected-armB.iso" \
 WS_ARM_B_ISO="$T/protected-armB.iso" WS_UMOUNT="$T/umount-remove" \
 run stage --phase "wy-$NOW-t18" --image "$IMG" --approve ) >"$T/t18.out" 2>&1
chk "T18 protected disappearance fails" 1 $?
grep -q 'protected.armB.after.*DISAPPEARED' "$T/out/wy-$NOW-t18/results.tsv" \
 && ok "T18b disappeared receipt" || bad "T18b missing receipt" 1 0
grep -q 'STAGE_FAIL' "$T/out/wy-$NOW-t18/results.tsv" \
 && ok "T18c no pass after disappearance" || bad "T18c missing failure" 1 0


# ---- kernel-candidate pair (K*). Kernel pins have NO WS_* seam: a test-only
# sha256sum shim maps ONLY bytes identical to the fake refs onto the production
# pins, so the copies inside the fake mount hash to the pins too.
KP=b8533efd2beea718ab9879f9800c0fa5bf2ecaba8ff8964ad894763628d246c5
KAP=baede849e769f70c4983a61a4f444922a13e9d7c12b5172cc11954d950dbaba6
mkdir -p "$T/kin" "$T/kref" "$T/kbin"
printf 'fake-kernel-bytes' >"$T/kref/k"; printf 'fake-kernel-all-bytes' >"$T/kref/ka"
KERN=$T/kin/kernel; KALL=$T/kin/kernel.all
cp "$T/kref/k" "$KERN"; cp "$T/kref/ka" "$KALL"
cat >"$T/kbin/sha256sum" <<'EOF'
#!/usr/bin/env bash
if [[ -f "$1" ]] && cmp -s -- "$1" "$TK_REF"; then printf '%s  %s\n' "$TK_PIN" "$1"
elif [[ -f "$1" ]] && cmp -s -- "$1" "$TKA_REF"; then printf '%s  %s\n' "$TKA_PIN" "$1"
else exec /usr/bin/sha256sum "$@"; fi
EOF
cat >"$T/redoxfs-kern" <<'EOF'
#!/usr/bin/env bash
# fake fstool: image layout already carries the original kernel pair
touch "$3/.fake_mounted"; mkdir -p "$3/usr/lib/boot"
printf 'orig-kernel' >"$3/usr/lib/boot/kernel"; printf 'orig-kernel-all' >"$3/usr/lib/boot/kernel.all"
sleep "${WS_ORPHAN_SLEEP}" &
exit 0
EOF
cat >"$T/redoxfs-kevil" <<'EOF'
#!/usr/bin/env bash
touch "$3/.fake_mounted"; mkdir -p "$3/usr/lib/boot"
printf 'orig-kernel-all' >"$3/usr/lib/boot/kernel.all"
ln -s "$WS_EVIL_TARGET" "$3/usr/lib/boot/kernel"
exit 0
EOF
chmod +x "$T/kbin/sha256sum" "$T/redoxfs-kern" "$T/redoxfs-kevil"
kman() {  # $1=out $2=kernel-role-line-path $3=kernel-sha $4=kall-path $5=kall-sha
    printf 'image\t%s\t%s\nbinary\t%s\t%s\nkernel\t%s\t%s\nkernel-all\t%s\t%s\n' \
        "$IMG" "$IMG_SHA" "$BIN" "$BIN_SHA" "$2" "$3" "$4" "$5" >"$1"
}
kman "$T/manifest-k.tsv" "$KERN" "$KP" "$KALL" "$KAP"
krun() {  # shimmed sha + kernel layout fake; caller may override MANIFEST/WS_FSTOOL
    PATH="$T/kbin:$PATH" TK_REF=$T/kref/k TKA_REF=$T/kref/ka TK_PIN=$KP TKA_PIN=$KAP \
    WS_FSTOOL=${WS_FSTOOL:-$T/redoxfs-kern} MANIFEST=${MANIFEST:-$T/manifest-k.tsv} run "$@"
}
noclaim() { [[ ! -e $T/out/$2 ]] && ok "$1 no claim/copy" || bad "$1 claim created" 0 1; }
KS="wy-$NOW-kern"

# K1/K2: unpaired inputs refused before claim
( krun stage --phase "${KS}1" --image "$IMG" --kernel "$KERN" --approve ) >/dev/null 2>&1
chk "K1 --kernel without --kernel-all -> 2" 2 $?; noclaim K1b "${KS}1"
( krun stage --phase "${KS}2" --image "$IMG" --kernel-all "$KALL" --approve ) >/dev/null 2>&1
chk "K2 --kernel-all without --kernel -> 2" 2 $?; noclaim K2b "${KS}2"
# K3: kernel pair outside a wy-<stamp>-kern* phase refused
( krun stage --phase "wy-$NOW-k3" --image "$IMG" --kernel "$KERN" --kernel-all "$KALL" --approve ) >/dev/null 2>&1
chk "K3 kernel pair needs kern phase -> 2" 2 $?; noclaim K3b "wy-$NOW-k3"
# K4: duplicate --kernel refused
( krun stage --phase "${KS}4" --image "$IMG" --kernel "$KERN" --kernel "$KERN" --kernel-all "$KALL" --approve ) >/dev/null 2>&1
chk "K4 duplicate --kernel -> 2" 2 $?; noclaim K4b "${KS}4"
# K5: kernel stage without --approve; boot with pair still BOOT_NOT_IMPLEMENTED
( krun stage --phase "${KS}5" --image "$IMG" --kernel "$KERN" --kernel-all "$KALL" ) >/dev/null 2>&1
chk "K5 kernel stage no --approve -> 2" 2 $?; noclaim K5b "${KS}5"
( krun boot --phase "${KS}5c" --image "$IMG" --kernel "$KERN" --kernel-all "$KALL" --approve --boot-cmd "rm -f $T/boot-sentinel" ) >"$T/k5.out" 2>&1
chk "K5c kernel boot -> 2" 2 $?
grep -q BOOT_NOT_IMPLEMENTED "$T/k5.out" && [[ -f $T/boot-sentinel ]] \
    && ok "K5d boot fails closed, --boot-cmd never ran" || bad "K5d boot" 0 1
noclaim K5e "${KS}5c"
# K6: kernel bytes != manifest sha -> 1 before claim
kman "$T/manifest-k6.tsv" "$KERN" "$KAP" "$KALL" "$KAP"
( MANIFEST=$T/manifest-k6.tsv krun stage --phase "${KS}6" --image "$IMG" --kernel "$KERN" --kernel-all "$KALL" --approve ) >/dev/null 2>&1
chk "K6 kernel hash != manifest -> 1" 1 $?; noclaim K6b "${KS}6"
# K7: manifest agrees with REAL file sha but not the pin (no shim) -> 1
kman "$T/manifest-k7.tsv" "$KERN" "$(sha "$KERN")" "$KALL" "$(sha "$KALL")"
( MANIFEST=$T/manifest-k7.tsv run stage --phase "${KS}7" --image "$IMG" --kernel "$KERN" --kernel-all "$KALL" --approve ) >"$T/k7.out" 2>&1
chk "K7 manifest vs kernel pin -> 1" 1 $?
grep -q 'contradicts pinned candidate' "$T/k7.out" && ok "K7b pin contradiction disclosed" || bad "K7b" 0 1
noclaim K7c "${KS}7"
# K8: swapped roles: files swapped on argv -> not in manifest under that role
( krun stage --phase "${KS}8" --image "$IMG" --kernel "$KALL" --kernel-all "$KERN" --approve ) >/dev/null 2>&1
chk "K8 swapped argv roles -> 3" 3 $?; noclaim K8b "${KS}8"
# K9: swapped roles in manifest with consistent shas -> pin contradiction
kman "$T/manifest-k9.tsv" "$KALL" "$KAP" "$KERN" "$KP"
( MANIFEST=$T/manifest-k9.tsv krun stage --phase "${KS}9" --image "$IMG" --kernel "$KALL" --kernel-all "$KERN" --approve ) >/dev/null 2>&1
chk "K9 swapped manifest roles -> 1" 1 $?; noclaim K9b "${KS}9"
# K10: missing kernel-all manifest role -> 3
grep -v '^kernel-all' "$T/manifest-k.tsv" >"$T/manifest-k10.tsv"
( MANIFEST=$T/manifest-k10.tsv krun stage --phase "${KS}10" --image "$IMG" --kernel "$KERN" --kernel-all "$KALL" --approve ) >/dev/null 2>&1
chk "K10 missing kernel-all role -> 3" 3 $?; noclaim K10b "${KS}10"
# K11: missing kernel file -> 3
( krun stage --phase "${KS}11" --image "$IMG" --kernel "$T/kin/nope" --kernel-all "$KALL" --approve ) >/dev/null 2>&1
chk "K11 missing kernel file -> 3" 3 $?; noclaim K11b "${KS}11"
# K12: symlinked kernel input (listed in manifest) -> 1
ln -s "$KERN" "$T/kernel-link"
kman "$T/manifest-k12.tsv" "$T/kernel-link" "$KP" "$KALL" "$KAP"
( MANIFEST=$T/manifest-k12.tsv krun stage --phase "${KS}12" --image "$IMG" --kernel "$T/kernel-link" --kernel-all "$KALL" --approve ) >/dev/null 2>&1
chk "K12 symlink kernel -> 1" 1 $?; noclaim K12b "${KS}12"
# K13: path alias through a symlinked parent directory -> 1
ln -s "$T/kin" "$T/kin-alias"
kman "$T/manifest-k13.tsv" "$KERN" "$KP" "$T/kin-alias/kernel.all" "$KAP"
( MANIFEST=$T/manifest-k13.tsv krun stage --phase "${KS}13" --image "$IMG" --kernel "$KERN" --kernel-all "$T/kin-alias/kernel.all" --approve ) >"$T/k13.out" 2>&1
chk "K13 symlinked-parent alias -> 1" 1 $?
grep -q 'path alias' "$T/k13.out" && ok "K13b alias disclosed" || bad "K13b" 0 1
noclaim K13c "${KS}13"
# K14: hard-linked kernel input -> 1
mkdir -p "$T/khl"; cp "$T/kref/k" "$T/khl/kernel"; ln "$T/khl/kernel" "$T/khl/kernel-twin"
kman "$T/manifest-k14.tsv" "$T/khl/kernel" "$KP" "$KALL" "$KAP"
( MANIFEST=$T/manifest-k14.tsv krun stage --phase "${KS}14" --image "$IMG" --kernel "$T/khl/kernel" --kernel-all "$KALL" --approve ) >"$T/k14.out" 2>&1
chk "K14 hard-linked kernel -> 1" 1 $?
grep -q 'hard-linked' "$T/k14.out" && ok "K14b hardlink disclosed" || bad "K14b" 0 1
noclaim K14c "${KS}14"
# K15: dry-run pair: plan names both fixed guest paths, no image copy
( krun dry-run --phase "${KS}15" --image "$IMG" --kernel "$KERN" --kernel-all "$KALL" ) >/dev/null 2>&1
chk "K15 kernel dry-run -> 0" 0 $?
grep -q 'usr/lib/boot/kernel from' "$T/out/${KS}15/plan.txt" && grep -q 'usr/lib/boot/kernel.all from' "$T/out/${KS}15/plan.txt" \
    && ok "K15b plan names fixed guest paths" || bad "K15b plan" 0 1
chk "K15c dry-run creates NO image" 0 "$(find "$T/out/${KS}15" -name '*.iso' | wc -l)"
# K16: paired valid stage: fixed guest paths only, cmp + pin receipts
img_before=$(sha "$IMG")
( krun stage --phase "${KS}16" --image "$IMG" --kernel "$KERN" --kernel-all "$KALL" --approve ) >"$T/k16.out" 2>&1
chk "K16 paired kernel stage -> 0" 0 $?
R=$T/out/${KS}16/results.tsv; M=$T/out/${KS}16/mnt
grep -q 'STAGE_PASS' "$R" && ok "K16b STAGE_PASS" || bad "K16b verdict" 0 1
cmp -s "$KERN" "$M/usr/lib/boot/kernel" && cmp -s "$KALL" "$M/usr/lib/boot/kernel.all" && cmp -s "$BIN" "$M/usr/bin/wyoming-probe" \
    && ok "K16c mounted copies byte-identical" || bad "K16c cmp" 0 1
chk "K16d only the three fixed guest files" "usr/bin/wyoming-probe usr/lib/boot/kernel usr/lib/boot/kernel.all" \
    "$(cd "$M" && find . -type f | sed 's|^\./||' | sort | tr '\n' ' ' | sed 's/ $//')"
for k in kernel kernel-all; do
    grep -qP "^insert\.$k\.cmp_rc\t0$" "$R" && ok "K16e $k cmp receipt" || bad "K16e $k cmp" 0 1
done
grep -qP "^insert\.kernel\.mounted_sha\t$KP$" "$R" && grep -qP "^insert\.kernel-all\.mounted_sha\t$KAP$" "$R" \
    && ok "K16f mounted pin receipts" || bad "K16f pins" 0 1
grep -q 'insert.usr/lib/boot/kernel.prior_sha' "$R" && grep -q 'kernel.source_identity.*metadata only' "$R" \
    && ok "K16g prior-sha + metadata-only identity receipts" || bad "K16g receipts" 0 1
grep -qP '^input_kernel\.after_sha\t' "$R" && ! grep -q 'MUTATED' "$R" \
    && ok "K16h kernel input after-hash receipts" || bad "K16h" 0 1
chk "K16i input image untouched" "$img_before" "$(sha "$IMG")"
# K17: consumed kern phase refused
( krun stage --phase "${KS}16" --image "$IMG" --kernel "$KERN" --kernel-all "$KALL" --approve ) >/dev/null 2>&1
chk "K17 consumed kern phase -> 4" 4 $?
# K18: escaping symlink at usr/lib/boot/kernel -> refuse before ANY copy
printf 'host-sentinel-untouched' >"$T/host-sentinel-k"
( WS_FSTOOL=$T/redoxfs-kevil WS_EVIL_TARGET=$T/host-sentinel-k \
  krun stage --phase "${KS}18" --image "$IMG" --kernel "$KERN" --kernel-all "$KALL" --approve ) >/dev/null 2>&1
chk "K18 kernel guest-path escape -> 1" 1 $?
grep -q 'INSERT_PATH_ESCAPE' "$T/out/${KS}18/results.tsv" && ok "K18b escape verdict" || bad "K18b" 0 1
chk "K18c host sentinel untouched" 'host-sentinel-untouched' "$(cat "$T/host-sentinel-k")"
[[ ! -e $T/out/${KS}18/mnt/usr/bin/wyoming-probe ]] && ok "K18d nothing inserted (guards precede copies)" || bad "K18d" 0 1
# K19: image lacking the kernel pair (app-layout fake) -> refuse before ANY copy
( WS_FSTOOL=$T/redoxfs krun stage --phase "${KS}19" --image "$IMG" --kernel "$KERN" --kernel-all "$KALL" --approve ) >/dev/null 2>&1
chk "K19 absent guest kernel -> 1" 1 $?
grep -q 'KERNEL_GUEST_PATH_ABSENT' "$T/out/${KS}19/results.tsv" && ok "K19b absent verdict" || bad "K19b" 0 1
[[ ! -e $T/out/${KS}19/mnt/usr/bin/wyoming-probe && ! -e $T/out/${KS}19/mnt/usr/lib/boot/kernel ]] \
    && ok "K19c nothing inserted" || bad "K19c" 0 1
grep -q 'protected.armB.after' "$T/out/${KS}19/results.tsv" && grep -q 'input_kernel.after' "$T/out/${KS}19/results.tsv" \
    && ok "K19d cleanup receipt discloses after-hashes incl. kernel" || bad "K19d" 0 1
# K20: stub writer mutates the kernel INPUT -> after-hash discloses, STAGE_FAIL
( WS_UMOUNT=$T/umount-tamper WS_FAKE_TAMPER=$KERN \
  krun stage --phase "${KS}20" --image "$IMG" --kernel "$KERN" --kernel-all "$KALL" --approve ) >/dev/null 2>&1
chk "K20 kernel input mutation -> 1" 1 $?
grep -q 'input_kernel.after.*MUTATED' "$T/out/${KS}20/results.tsv" && grep -q 'STAGE_FAIL' "$T/out/${KS}20/results.tsv" \
    && ok "K20b mutation disclosed, STAGE_FAIL" || bad "K20b" 0 1
cp "$T/kref/k" "$KERN"
# K21: app-only run unaffected by extra kernel roles in the manifest
( MANIFEST=$T/manifest-k.tsv run stage --phase "wy-$NOW-k21" --image "$IMG" --approve ) >/dev/null 2>&1
chk "K21 app-only stage with kernel-role manifest -> 0" 0 $?
grep -q 'stage_mode' "$T/out/wy-$NOW-k21/results.tsv" && bad "K21b app-only got kernel receipts" 0 1 || ok "K21b app-only receipts unchanged"
# K22: kernel-stage fake children reaped; no orphans
reaped=0
for i in 1 2 3 4 5; do pgrep -f "sleep $ORPHAN_SLEEP" >/dev/null || { reaped=1; break; }; sleep 1; done
chk "K22 no orphans after kernel stages" 1 "$reaped"

printf '\nsuite: %d passed, %d failed\n' "$pass" "$fail"
[[ $fail -eq 0 ]] || exit 1
