#!/usr/bin/env bash
# tests.sh — offline test suite for return-runbook.sh.
#
# NEVER touches a real phone, Mac, or fastboot: PATH is prefixed with fake
# ssh/fastboot/ioreg shims that log every call and return canned output.
# Proves: dry-run touches nothing, timeouts fire, and flash/erase strings
# are refused. Run: bash scripts/phone/tests.sh
set -u

HERE="$(cd "$(dirname "$0")" && pwd)"
RUNBOOK="$HERE/return-runbook.sh"
PASS=0; FAIL=0

ok()   { PASS=$((PASS+1)); echo "  ok: $1"; }
bad()  { FAIL=$((FAIL+1)); echo "  FAIL: $1"; }
check(){ if eval "$2"; then ok "$1"; else bad "$1"; fi; }
# status_of <receipt.tsv> <phase> <action> -> prints the status column
status_of(){ awk -F'\t' -v p="$2" -v a="$3" '$2==p && $3==a {s=$4} END{print s}' "$1"; }

# ---------------------------------------------------------------- shims ----
SHIM="$(mktemp -d)"
WORK="$(mktemp -d)"
trap 'rm -rf "$SHIM" "$WORK"' EXIT

cat > "$SHIM/ssh" <<'EOF'
#!/usr/bin/env bash
# Fake ssh. Logs every call. Host "macbook" runs the remote command locally
# (against the other fakes); "oneplus" fails per FAKE_SSH_ONEPLUS (0=up,
# 1=down) and sleeps FAKE_SSH_SLEEP to exercise hard timeouts; "oneplus-usb"
# succeeds per FAKE_PHONE_USB.
log(){ printf '%s\n' "$*" >> "${FAKE_LOG:-/dev/null}"; }
log "ssh $*"
host=""
while [ $# -gt 0 ]; do
  case "$1" in
    -o) shift 2 ;;
    -p) shift 2 ;;
    *) host="$1"; shift; break ;;
  esac
done
case "$host" in
  macbook)
    printf 'mac %s\n' "$*" >> "${FAKE_LOG:-/dev/null}"
    # ACM discovery: when FAKE_ACM_DEV is set, the ls of /dev/cu.usbmodem*
    # finds it (the real device only exists after U-Boot enables its gadget)
    case "$*" in
      *ls\ /dev/cu.usbmodem*)
        if [ -n "${FAKE_ACM_DEV:-}" ]; then printf '%s\n' "$FAKE_ACM_DEV"; exit 0; fi ;;
    esac
    bash -c "$*" ;;
  oneplus)
    printf 'phone %s\n' "$*" >> "${FAKE_LOG:-/dev/null}"
    [ -n "${FAKE_SSH_SLEEP:-}" ] && sleep "$FAKE_SSH_SLEEP"
    rc="${FAKE_SSH_ONEPLUS:-1}"
    [ "$rc" != 0 ] && echo "ssh: connect to host mesh port 22: Connection timed out" >&2
    exit "$rc" ;;
  oneplus-usb)
    printf 'phone-usb %s\n' "$*" >> "${FAKE_LOG:-/dev/null}"
    exit "${FAKE_PHONE_USB:-0}" ;;
  *) exit 255 ;;
esac
EOF

cat > "$SHIM/fastboot" <<'EOF'
#!/usr/bin/env bash
printf 'fastboot %s\n' "$*" >> "${FAKE_LOG:-/dev/null}"
printf '%s\n' "${FAKE_FASTBOOT_OUT:-}"
exit "${FAKE_FASTBOOT_RC:-0}"
EOF

cat > "$SHIM/ioreg" <<'EOF'
#!/usr/bin/env bash
printf 'ioreg %s\n' "$*" >> "${FAKE_LOG:-/dev/null}"
printf '%s\n' "${FAKE_IOREG_OUT:-}"
exit 0
EOF

cat > "$SHIM/scp" <<'EOF'
#!/usr/bin/env bash
# fake scp: log, then actually copy so remote sha verification is real
printf 'scp %s\n' "$*" >> "${FAKE_LOG:-/dev/null}"
src=""; dst=""
for a in "$@"; do
  case "$a" in -o|-1|-q) continue ;; esac
  case "$a" in "$FAKE_SSH_HOST":*) dst="$a" ;; *) [ -f "$a" ] && src="$a" ;; esac
done
[ -n "$src" ] && [ -n "$dst" ] || exit 1
d="${dst#"$FAKE_SSH_HOST":}"
d="${d/#\~/$HOME}"
mkdir -p "$d" && cp "$src" "$d/"
EOF

cat > "$SHIM/shasum" <<'EOF'
#!/usr/bin/env bash
printf 'shasum %s\n' "$*" >> "${FAKE_LOG:-/dev/null}"
while [ $# -gt 0 ]; do
  case "$1" in -a) shift 2 ;; *) break ;; esac
done
exec sha256sum "$@"
EOF

chmod +x "$SHIM"/*
# the runbook calls fastboot by its explicit Mac path; provide the shim there too
mkdir -p "$WORK/machome/tools/platform-tools"
cp "$SHIM/fastboot" "$WORK/machome/tools/platform-tools/fastboot"
export PATH="$SHIM:$PATH"
export FAKE_LOG="$WORK/fake.log"
export OUT_ROOT="$WORK/out"

# canned scenario defaults (each test overrides what it needs)
mkdir -p "$WORK/payload"
printf 'FAKE-UBOOT\n'  > "$WORK/payload/u-boot-enchilada-v0.img"
printf 'FAKE-EFIDIAG\n' > "$WORK/payload/efi-diag.efi"
printf 'FAKE-REDOX\n'  > "$WORK/payload/redox-ramboot.efi"
reset_env(){
  rm -f "$FAKE_LOG"
  export FAKE_SSH_ONEPLUS=1 FAKE_PHONE_USB=0 FAKE_SSH_SLEEP="" \
         FAKE_FASTBOOT_OUT="" FAKE_FASTBOOT_RC=0 FAKE_IOREG_OUT="" \
         SSH_CONNECT_TIMEOUT=10 CMD_TIMEOUT=20 P2_TIMEOUT=30 P3_TIMEOUT=30 P4_TIMEOUT=30 \
         ACM_TIMEOUT=10 FAKE_ACM_DEV="/dev/cu.usbmodem9999" \
         RUN_ID="t-$RANDOM$RANDOM" \
         FAKE_SSH_HOST=macbook MAC_HOME="$WORK/machome" HOME="$WORK/machome" \
         PAYLOAD_DIR="$WORK/payload" UBOOT_IMG="$WORK/payload/u-boot-enchilada-v0.img" \
         EFI_DIAG="$WORK/payload/efi-diag.efi" REDOX_IMG="$WORK/payload/redox-ramboot.efi"
}

# ------------------------------------------------------- test: guard -------
echo "== guard self-test (P0 rule: refuse flash/erase/persist/modem) =="
out="$(bash "$RUNBOOK" --selftest-guard 2>&1)"; rc=$?
check "guard self-test exits 0" "[ $rc -eq 0 ]"
check "guard self-test PASSes all cases" "printf '%s' '$out' | grep -q 'GUARD-SELFTEST: PASS'"

# ------------------------------------------------- test: dry-run default ---
echo "== dry-run default: prints commands, touches nothing =="
reset_env
out="$(bash "$RUNBOOK" 2>&1)"; rc=$?
R="$WORK/out/$(printf '%s\n' "$out" | grep -oE 'run-id=[^ ]+' | cut -d= -f2)/receipt.tsv"
check "dry-run exits 0" "[ $rc -eq 0 ]"
check "dry-run executed NO fake device command" "[ ! -f \"$FAKE_LOG\" ]"
check "receipt.tsv written" "[ -f \"$R\" ]"
check "receipt has only dryrun/skipped/noted/UNKNOWN statuses" \
  "! awk -F'\t' 'NR>1 && (\$4==\"ok\" || \$4==\"fail\" || \$4==\"timeout\" || \$4==\"refused\") {found=1} END{exit found?0:1}' \"$R\""
check "dry-run receipt contains no flash/erase verbs" "! grep -qiE 'flash|erase|format|oem unlock' \"$R\""
check "dry-run prints DRYRUN lines" "printf '%s' \"\$out\" | grep -q '^DRYRUN '"

# --------------------------------------- test: --live actually executes ----
echo "== --live is the explicit opt-in (executes via shims only) =="
reset_env
export FAKE_FASTBOOT_OUT="8009200c    fastboot"
out="$(bash "$RUNBOOK" --live 2>&1)"; rc=$?
R="$WORK/out/$(printf '%s\n' "$out" | grep -oE 'run-id=[^ ]+' | cut -d= -f2)/receipt.tsv"
check "--live exits 0" "[ $rc -eq 0 ]"
check "--live executed fake commands" "[ -s \"$FAKE_LOG\" ]"
check "P0 classified FASTBOOT (fastboot devices non-empty)" \
  "[ \"$(status_of "$R" P0 classify)\" = FASTBOOT ]"
check "P3 fastboot boot (RAM) issued through the Mac" \
  "grep -q 'fastboot boot' \"$FAKE_LOG\" && grep -q 'ssh -o ConnectTimeout=10 macbook' \"$FAKE_LOG\""
check "P3 bdinfo issued to console" "grep -q 'bdinfo' \"$FAKE_LOG\""
check "P4 Redox skipped without --redox" \
  "[ \"$(status_of "$R" P4 skip)\" = skipped ]"
check "no flash/erase/format/-w in anything executed" "! grep -qiE 'flash|erase|format|fastboot -w' \"$FAKE_LOG\""

# ------------------- test: macOS portability (review bug 2) ----------------
echo "== macOS portability: explicit fastboot path, no GNU timeout, stty -f =="
reset_env
export FAKE_FASTBOOT_OUT="8009200c    fastboot"
out="$(bash "$RUNBOOK" --live 2>&1)"; rc=$?
check "fastboot invoked via ~/tools/platform-tools/fastboot" \
  "grep -q 'platform-tools/fastboot' \"$FAKE_LOG\""
check "no Mac-side fastboot call without the explicit platform-tools path" \
  "[ -z \"\$(grep '^mac ' \"$FAKE_LOG\" | grep 'fastboot' | grep -v 'platform-tools/fastboot')\" ]"
check "no GNU 'timeout' used in any Mac-side command" \
  "! grep '^mac ' \"$FAKE_LOG\" | grep -qE '(^|[;&|] *)timeout [0-9]'"
check "portable perl-alarm timeout used on the Mac side" \
  "grep -q 'alarm shift' \"$FAKE_LOG\""
check "stty uses macOS -f, never GNU -F" \
  "grep -q 'stty -f' \"$FAKE_LOG\" && ! grep -q 'stty -F' \"$FAKE_LOG\""

# ------------------- test: P4 loads payloads via U-Boot fastboot gadget (review bug 3)
echo "== P4 stages payloads over USB via U-Boot fastboot gadget, not mmc =="
reset_env
export FAKE_FASTBOOT_OUT="8009200c    fastboot"
out="$(bash "$RUNBOOK" --live --redox 2>&1)"; rc=$?
check "U-Boot fastboot gadget started on the console (fastboot usb 0)" \
  "grep -q 'fastboot usb 0' \"$FAKE_LOG\""
check "payloads staged over USB from the Mac (fastboot stage)" \
  "grep -qE 'fastboot stage .*/efi-diag.efi' \"$FAKE_LOG\" && grep -qE 'fastboot stage .*/redox-ramboot.efi' \"$FAKE_LOG\""
check "U-Boot driven over the gadget, not via the dead ACM console" \
  "grep -q 'oem run:' \"$FAKE_LOG\""
check "bootefi boots from the pinned fastboot buffer address" \
  "grep -q 'bootefi 0xa0000000' \"$FAKE_LOG\""
check "no mmc rescan / fatload mmc anywhere (UFS phone, payloads not on-device)" \
  "! grep -qE 'mmc rescan|fatload mmc' \"$FAKE_LOG\""

# ------- test: ONE DWC3 gadget — no ACM write after the gadget switch (review bug 5)
echo "== one DWC3 gadget: gadget started once, U-Boot driven by oem run, no ACM write after switch =="
reset_env
export FAKE_FASTBOOT_OUT="8009200c    fastboot"
out="$(bash "$RUNBOOK" --live --redox 2>&1)"; rc=$?
check "fastboot gadget started exactly once (the ACM console dies with the switch)" \
  "[ \"$(grep -c '^mac printf .*fastboot usb 0' "$FAKE_LOG")\" -eq 1 ]"
check "U-Boot driven via fastboot oem run (CONFIG_FASTBOOT_OEM_RUN=y, run_ucmd)" \
  "grep -q \"oem run:'bootefi 0xa0000000\" \"$FAKE_LOG\""
check "exactly two stage + two oem run cycles (efi-diag, then redox)" \
  "[ \"$(grep -c '^mac .*fastboot stage' "$FAKE_LOG")\" -eq 2 ] && [ \"$(grep -c '^mac .*oem run:' "$FAKE_LOG")\" -eq 2 ]"
check "no ACM console write after the 'fastboot usb' switch line" \
  "awk '/^mac /{ if (/printf.*> \/dev\/cu\.usbmodem/ && seen) exit 1; if (/fastboot usb/) seen=1 }' \"$FAKE_LOG\""

# ------------------- test: console discovery — polled ACM device (review bug 4)
echo "== console discovery: ACM device polled, never hardcoded usbmodem0 =="
reset_env
export FAKE_FASTBOOT_OUT="8009200c    fastboot" FAKE_ACM_DEV="/dev/cu.usbmodem9999"
out="$(bash "$RUNBOOK" --live --redox 2>&1)"; rc=$?
R="$WORK/out/$(printf '%s\n' "$out" | grep -oE 'run-id=[^ ]+' | cut -d= -f2)/receipt.tsv"
check "console discovery polls for /dev/cu.usbmodem*" \
  "grep -q 'ls /dev/cu.usbmodem' \"$FAKE_LOG\""
check "discovered ACM device used for the console capture (stty)" \
  "grep -q 'stty -f /dev/cu.usbmodem9999' \"$FAKE_LOG\""
check "discovered device used for U-Boot input (printf > device)" \
  "grep -q '> /dev/cu.usbmodem9999' \"$FAKE_LOG\""
check "no hardcoded usbmodem0 anywhere in executed commands" \
  "! grep -q 'usbmodem0' \"$FAKE_LOG\""
reset_env
export FAKE_FASTBOOT_OUT="8009200c    fastboot" FAKE_ACM_DEV="" ACM_TIMEOUT=5
out="$(bash "$RUNBOOK" --live 2>&1)"; rc=$?
R="$WORK/out/$(printf '%s\n' "$out" | grep -oE 'run-id=[^ ]+' | cut -d= -f2)/receipt.tsv"
check "no ACM device -> discovery receipted missing, console steps skipped, run completes" \
  "[ \"$rc\" -eq 0 ] && [ \"$(status_of "$R" P3 console-found)\" = missing ] && ! grep -q 'stty' \"$FAKE_LOG\""

# --------------------------------- test: staging to the Mac (review bug 1) --
echo "== staging: payloads scp'd to the Mac and sha256-verified both ends =="
reset_env
export FAKE_FASTBOOT_OUT="8009200c    fastboot"
out="$(bash "$RUNBOOK" --live 2>&1)"; rc=$?
R="$WORK/out/$(printf '%s\n' "$out" | grep -oE 'run-id=[^ ]+' | cut -d= -f2)/receipt.tsv"
check "scp of payloads to macbook:...op6-runbook/<run-id> issued" \
  "grep -q 'scp .* macbook:.*op6-runbook/' \"$FAKE_LOG\" && grep -q 'u-boot-enchilada-v0.img' \"$FAKE_LOG\""
check "remote shasum -a 256 verification issued" "grep -q 'shasum -a 256' \"$FAKE_LOG\""
check "stage-verify u-boot receipted ok (local sha == remote sha)" \
  "[ \"$(status_of "$R" PS verify-u-boot-enchilada-v0.img)\" = ok ]"
check "P3 fastboot boot uses the Mac-staged path, never a 3090 path" \
  "grep 'fastboot boot' \"$FAKE_LOG\" | grep -q 'op6-runbook/' && ! grep 'fastboot boot' \"$FAKE_LOG\" | grep -q '$HOME'"
reset_env
out="$(bash "$RUNBOOK" 2>&1)"
check "dry-run prints the staging commands too" "printf '%s' \"\$out\" | grep -q 'DRYRUN scp '"
check "dry-run fastboot boot references op6-runbook, not $HOME" \
  "printf '%s' \"\$out\" | grep 'boot' | grep -q 'op6-runbook' && ! printf '%s' \"\$out\" | grep 'boot' | grep -q '$HOME'"

# ------------------------------------------- test: STUCK_EARLY diagnosis ---
echo "== STUCK_EARLY: pmOS serial visible, no ssh, no fastboot =="
reset_env
export FAKE_IOREG_OUT="postmarketOS"
out="$(bash "$RUNBOOK" --live 2>&1)"; rc=$?
R="$WORK/out/$(printf '%s\n' "$out" | grep -oE 'run-id=[^ ]+' | cut -d= -f2)/receipt.tsv"
check "classified STUCK_EARLY" "[ \"$(status_of "$R" P0 classify)\" = STUCK_EARLY ]"
check "P2 notes operator power-cycle instead of rebooting" \
  "[ \"$(status_of "$R" P2 note)\" = noted ] && ! grep -q 'reboot-argument' \"$FAKE_LOG\""
check "P3 skipped: no fastboot device to boot" \
  "[ \"$(status_of "$R" P3 skip)\" = skipped ] && ! grep -q 'fastboot boot' \"$FAKE_LOG\""

# ------------------------------------------- test: LINUX_UP path (P1/P2) ---
echo "== LINUX_UP: pmOS reachable over mesh =="
reset_env
export FAKE_SSH_ONEPLUS=0 P2_TIMEOUT=5
out="$(bash "$RUNBOOK" --live 2>&1)"; rc=$?
R="$WORK/out/$(printf '%s\n' "$out" | grep -oE 'run-id=[^ ]+' | cut -d= -f2)/receipt.tsv"
check "classified LINUX_UP (mesh ssh exit 0)" "[ \"$(status_of "$R" P0 classify)\" = LINUX_UP ]"
check "P1 recorded cmdline/cryptkey/keyslots/journal" \
  "[ \"$(status_of "$R" P1 cmdline)\" = ok ] && [ \"$(status_of "$R" P1 cryptkey)\" = ok ] && [ \"$(status_of "$R" P1 keyslots)\" = ok ] && [ \"$(status_of "$R" P1 journal-last-boot)\" = ok ]"
check "P1 did NOT reboot (P2 owns the bootloader reboot)" \
  "[ \"$(status_of "$R" P1 reboot)\" = not-rebooting ]"
check "P2 rebooted to bootloader via phone ssh" "grep -q 'reboot-argument=bootloader' \"$FAKE_LOG\""

# ------------------------------------------------ test: timeout recorded ---
echo "== hard timeout fires and is recorded as timeout, not fail =="
reset_env
export SSH_CONNECT_TIMEOUT=1 FAKE_SSH_SLEEP=3
out="$(bash "$RUNBOOK" --live 2>&1)"; rc=$?
R="$WORK/out/$(printf '%s\n' "$out" | grep -oE 'run-id=[^ ]+' | cut -d= -f2)/receipt.tsv"
check "mesh-ssh step recorded as timeout rc=124" \
  "[ \"$(status_of "$R" P0 mesh-ssh)\" = timeout ]"
check "run still completes (exit 0)" "[ $rc -eq 0 ]"

# -------------------------------------- test: one Redox try per run (P4) ---
echo "== one Redox try per run (--redox, marker) =="
reset_env
export FAKE_FASTBOOT_OUT="8009200c    fastboot"
export RUN_ID=t-redox1
bash "$RUNBOOK" --live --redox >/dev/null 2>&1
bash "$RUNBOOK" --live --redox >/dev/null 2>&1
R="$WORK/out/t-redox1/receipt.tsv"
n="$(grep -c '^mac .*fastboot stage .*efi-diag' "$FAKE_LOG" || true)"
check "exactly one efi-diag/redox sequence across two invocations" "[ \"$n\" -eq 1 ]"
check "second P4 refused by marker (one try per run)" \
  "[ \"$(status_of "$R" P4 refuse)\" = refused ]"
unset RUN_ID

# --------------------------------------- test: guard refuses at runtime ----
echo "== guard refuses a forbidden string even in live mode =="
reset_env
export FAKE_FASTBOOT_OUT="8009200c    fastboot"
# inject a forbidden string via an env knob the script interpolates
export UBOOT_IMG="/dev/persist"
out="$(bash "$RUNBOOK" --live 2>&1)"; rc=$?
R="$WORK/out/$(printf '%s\n' "$out" | grep -oE 'run-id=[^ ]+' | cut -d= -f2)/receipt.tsv"
check "P3 uboot-ramboot REFUSED (persist in payload path)" \
  "[ \"$(status_of "$R" P3 uboot-ramboot)\" = refused ]"
check "nothing forbidden executed" "! grep -q 'persist' \"$FAKE_LOG\""
check "run completes despite refusal (exit 0)" "[ $rc -eq 0 ]"
unset UBOOT_IMG

# --------------------------------------- test: script text safety scan -----
echo "== no destructive verb in any command the script can issue =="
check "no flash/erase/format in exec_cmd/probe/mac_cmd/phone_cmd lines" \
  "! grep -nE '(exec_cmd|probe|mac_cmd|phone_cmd)[^#]*\b(flash|erase|format|oem unlock)\b' \"$RUNBOOK\""

# ------------------------------------------------------------------ done --
echo
echo "tests: $PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ]
