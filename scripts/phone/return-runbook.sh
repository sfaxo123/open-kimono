#!/usr/bin/env bash
# return-runbook.sh — OnePlus 6 (enchilada) phone-return runbook.
#
# Written device-free per ~/artifacts/handoff/op6-return-runbook/PLAN.md.
# A later agent runs this when the phone is reachable again.
#
#   DRY-RUN IS THE DEFAULT: every command is printed (prefixed DRYRUN) and
#   NOTHING is executed against the phone, the Mac, or fastboot. Pass --live
#   to act. --redox additionally unlocks phase P4 (one Redox try per run,
#   enforced by a marker file in the run dir).
#
# Hard rules baked in (PLAN):
#   - fastboot boot (RAM) ONLY. The guard refuses any flash/erase/format/
#     oem-unlock string and any mention of xbl/abl/devinfo/persist/modem.
#   - Every executed command runs under a hard `timeout` (per-command
#     CMD_TIMEOUT; phase budgets P0/P2/P3/P4_TIMEOUT).
#   - Every phase writes receipt lines to out/phone-runbook/<run-id>/receipt.tsv.
#
# Tests (scripts/phone/tests.sh) put fake ssh/fastboot/ioreg/networksetup
# shims on PATH and drive this script with the env knobs below.
set -uo pipefail

# ---------- configuration (env-overridable; tests shim these) ----------
MAC_SSH="${MAC_SSH:-macbook}"                  # ssh alias for the MacBook holding USB
PHONE_SSH="${PHONE_SSH:-oneplus}"              # mesh alias from the 3090
PHONE_USB_SSH="${PHONE_USB_SSH:-oneplus-usb}"  # USB-net alias ON the Mac (<phone-usb-ip>)
FASTBOOT="${FASTBOOT:-\$HOME/tools/platform-tools/fastboot}"  # literal $HOME: expanded by the Mac's shell, where fastboot actually runs
PAYLOAD_DIR="${PAYLOAD_DIR:-$HOME/artifacts/handoff/oneplus-redox-20261024/phone-boot}"
UBOOT_IMG="${UBOOT_IMG:-$PAYLOAD_DIR/u-boot-enchilada-v0.img}"
EFI_DIAG="${EFI_DIAG:-$PAYLOAD_DIR/efi-diag.efi}"
REDOX_IMG="${REDOX_IMG:-$PAYLOAD_DIR/redox-ramboot.efi}"
OUT_ROOT="${OUT_ROOT:-out/phone-runbook}"
RUN_ID="${RUN_ID:-$(date -u +%Y%m%dT%H%M%SZ)}"
# Staging (review bug 1): fastboot runs ON the Mac, so the 3090 payload
# paths above do not exist where the commands execute. Every payload is
# scp'd to the Mac under ~/op6-runbook/<run-id>/ and sha256-verified on
# both ends before anything references it. MAC_HOME is a literal $HOME —
# expanded by the Mac's shell (via ssh), never by this one.
MAC_HOME="${MAC_HOME:-\$HOME}"
STAGE_DIR_REL="op6-runbook/$RUN_ID"
CMD_TIMEOUT="${CMD_TIMEOUT:-20}"               # per-command hard timeout (s)
P0_TIMEOUT="${P0_TIMEOUT:-60}"                 # reachability probe budget
P2_TIMEOUT="${P2_TIMEOUT:-300}"                # fastboot poll budget
P3_TIMEOUT="${P3_TIMEOUT:-120}"                # U-Boot console budget
P4_TIMEOUT="${P4_TIMEOUT:-180}"                # Redox try budget
SSH_CONNECT_TIMEOUT="${SSH_CONNECT_TIMEOUT:-10}"
ACM_TIMEOUT="${ACM_TIMEOUT:-60}"                # console ACM discovery poll budget

LIVE=0
REDOX=0
SELFTEST=0

usage() {
  cat <<'EOF'
usage: return-runbook.sh [--live] [--redox] [--selftest-guard] [--help]
  (no flags)        dry-run: print every command, touch nothing
  --live            actually execute against Mac/phone/fastboot
  --redox           unlock phase P4 (one Redox try per run)
  --selftest-guard  run the flash/erase guard against known-bad strings and exit
EOF
}

while [ $# -gt 0 ]; do
  case "$1" in
    --live) LIVE=1 ;;
    --redox) REDOX=1 ;;
    --selftest-guard) SELFTEST=1 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown flag: $1" >&2; usage; exit 2 ;;
  esac
  shift
done

# ---------- safety guard ----------
# Refuses anything that could write to or destroy the phone's flash.
FORBIDDEN='fastboot[^;&|]*[[:space:]](flash|erase|format|oem[[:space:]]+unlock)|flash\.elf|\b(xbl|abl|devinfo|persist|modem)\b'
guard() { # $* = command string; 0 = safe, 1 = forbidden
  if printf '%s' "$*" | grep -Eq "$FORBIDDEN"; then
    return 1
  fi
  return 0
}

selftest_guard() {
  local bad=(
    'fastboot flash boot x.img'
    'fastboot erase persist'
    'fastboot format userdata'
    'fastboot --disable-verity --disable-verification flash vbmeta_a vbmeta.img'
    'fastboot oem unlock'
    'fastboot flash xbl xbl.img'
    'touch /dev/persist'
    'echo modem'
  )
  local good=(
    'fastboot devices'
    'fastboot boot u-boot-enchilada-v0.img'
    'ssh macbook fastboot devices'
    'fastboot usb 0 -l 0xa0000000 -s 0x6000000'
    'fastboot stage efi-diag.efi'
    "fastboot oem run:'bootefi 0xa0000000 \${fdtcontroladdr}'"
    'bootefi 0xa0000000 ${fdtcontroladdr}'
  )
  local rc=0 s
  for s in "${bad[@]}"; do
    if guard "$s"; then echo "GUARD-FAIL: accepted forbidden: $s" >&2; rc=1; fi
  done
  for s in "${good[@]}"; do
    if ! guard "$s"; then echo "GUARD-FAIL: refused safe: $s" >&2; rc=1; fi
  done
  if [ "$rc" -eq 0 ]; then echo "GUARD-SELFTEST: PASS"; else echo "GUARD-SELFTEST: FAIL" >&2; fi
  return "$rc"
}
if [ "$SELFTEST" -eq 1 ]; then
  selftest_guard
  exit $?
fi

# ---------- run dir + receipt ----------
OUT_DIR="$OUT_ROOT/$RUN_ID"
mkdir -p "$OUT_DIR"
RECEIPT="$OUT_DIR/receipt.tsv"
printf 'ts\tphase\taction\tstatus\tdetail\n' > "$RECEIPT"

log() { printf '%s\n' "$*" >&2; }

receipt() { # phase action status detail
  printf '%s\t%s\t%s\t%s\t%s\n' "$(date -u +%FT%TZ)" "$1" "$2" "$3" "${4:-}" >> "$RECEIPT"
}

# ---------- command runner ----------
# exec_cmd runs a command string under a hard `timeout` and tees to
# <label>.out. In dry-run it is only printed — never executed.
exec_cmd() { # phase label timeout [--] cmdstring
  local phase="$1" label="$2" t="$3" cmd="$4" rc
  [ "$cmd" = "--" ] && cmd="$5"
  if ! guard "$cmd"; then
    log "REFUSED (forbidden flash/erase string): $cmd"
    receipt "$phase" "$label" refused "$cmd"
    return 9
  fi
  if [ "$LIVE" -eq 0 ]; then
    log "DRYRUN $cmd"
    receipt "$phase" "$label" dryrun "$cmd"
    return 0
  fi
  timeout "$t" bash -c "$cmd" > "$OUT_DIR/$label.out" 2>&1
  rc=$?
  case "$rc" in
    0)   receipt "$phase" "$label" ok "rc=0" ;;
    124) receipt "$phase" "$label" timeout "rc=124 $cmd" ;;
    *)   receipt "$phase" "$label" fail "rc=$rc" ;;
  esac
  return "$rc"
}

# probe: like exec_cmd but captures stdout (empty string in dry-run)
probe() { # phase label timeout [--] cmdstring
  local phase="$1" label="$2" t="$3" cmd="$4" rc
  [ "$cmd" = "--" ] && cmd="$5"
  if ! guard "$cmd"; then
    log "REFUSED (forbidden flash/erase string): $cmd"
    receipt "$phase" "$label" refused "$cmd"
    return 9
  fi
  if [ "$LIVE" -eq 0 ]; then
    log "DRYRUN $cmd"
    receipt "$phase" "$label" dryrun "$cmd"
    return 0
  fi
  timeout "$t" bash -c "$cmd" > "$OUT_DIR/$label.out" 2> "$OUT_DIR/$label.err"
  rc=$?
  case "$rc" in
    0)   receipt "$phase" "$label" ok "rc=0" ;;
    124) receipt "$phase" "$label" timeout "rc=124 $cmd" ;;
    *)   receipt "$phase" "$label" fail "rc=$rc" ;;
  esac
  cat "$OUT_DIR/$label.out" 2>/dev/null || true
  return "$rc"
}

# The U-Boot serial console on the Mac is a CDC-ACM device (/dev/cu.usbmodem*)
# that only EXISTS AFTER U-Boot enables its USB gadget — so it is discovered
# by polling on the Mac, never hardcoded to usbmodem0.
find_console() { # phase
  local out
  out="$(probe "$1" console-find "$ACM_TIMEOUT" -- \
    "$(mac_cmd "for i in \$(seq 1 12); do d=\$(ls /dev/cu.usbmodem* 2>/dev/null); [ -n \"\$d\" ] && { printf '%s\n' \"\$d\" | head -1; exit 0; }; sleep 5; done; exit 1")")" || true
  ACM_DEV="$(printf '%s\n' "$out" | grep -m1 '^/dev/cu\.usbmodem' || true)"
  if [ -n "$ACM_DEV" ]; then
    receipt "$1" console-found ok "device=$ACM_DEV"
  elif [ "$LIVE" -eq 0 ]; then
    ACM_DEV="/dev/cu.usbmodemDISCOVERED"   # placeholder so dry-run prints the plan
    receipt "$1" console-found dryrun "ACM device is discovered at run time"
  else
    receipt "$1" console-found missing "no /dev/cu.usbmodem* appeared within $ACM_TIMEOUT s"
  fi
}

# ssh helpers: build the exact command strings; execution stays in exec_cmd/probe
mac_cmd() { printf 'ssh -o ConnectTimeout=%s %s %q' "$SSH_CONNECT_TIMEOUT" "$MAC_SSH" "$*"; }
phone_cmd() { printf 'ssh -o ConnectTimeout=%s %s ssh -o ConnectTimeout=%s %s %q' \
  "$SSH_CONNECT_TIMEOUT" "$MAC_SSH" "$SSH_CONNECT_TIMEOUT" "$PHONE_USB_SSH" "$*"; }

# ---------- staging (review bug 1) ----------
# Copy every payload to the Mac (where fastboot actually runs) and verify
# sha256 on both ends. After this, MAC_* paths are the only ones used.
MAC_UBOOT_IMG="$MAC_HOME/$STAGE_DIR_REL/$(basename "$UBOOT_IMG")"
MAC_EFI_DIAG="$MAC_HOME/$STAGE_DIR_REL/$(basename "$EFI_DIAG")"
MAC_REDOX_IMG="$MAC_HOME/$STAGE_DIR_REL/$(basename "$REDOX_IMG")"

stage_payloads() {
  local f base lhash rhash
  exec_cmd PS stage-mkdir "$CMD_TIMEOUT" -- \
    "$(mac_cmd "mkdir -p $MAC_HOME/$STAGE_DIR_REL")" || return 1
  for f in "$UBOOT_IMG" "$EFI_DIAG" "$REDOX_IMG"; do
    base="$(basename "$f")"
    lhash="$(sha256sum "$f" 2>/dev/null | awk '{print $1}')"
    exec_cmd PS "stage-$base" "$CMD_TIMEOUT" -- \
      "scp -o ConnectTimeout=$SSH_CONNECT_TIMEOUT $f $MAC_SSH:~/$STAGE_DIR_REL/" || continue
    rhash="$(probe PS "sha-$base" "$CMD_TIMEOUT" \
      "$(mac_cmd "shasum -a 256 $MAC_HOME/$STAGE_DIR_REL/$base | awk '{print \$1}'")")" || true
    if [ "$LIVE" -eq 0 ]; then
      receipt PS "verify-$base" dryrun "local-sha=$lhash"
    elif [ -n "$lhash" ] && [ "$lhash" = "$rhash" ]; then
      receipt PS "verify-$base" ok "sha256=$lhash"
    else
      receipt PS "verify-$base" mismatch "local=$lhash remote=$rhash"
    fi
  done
}

# ---------- phases ----------
STATE="UNKNOWN"

p0() {
  # Reachability probes, then classify: LINUX_UP | FASTBOOT | STUCK_EARLY | ABSENT
  local fb="" usb="" nic="" mesh_rc=0
  exec_cmd P0 mesh-ssh "$SSH_CONNECT_TIMEOUT" -- \
    "ssh -o ConnectTimeout=$SSH_CONNECT_TIMEOUT $PHONE_SSH true" || mesh_rc=$?
  exec_cmd P0 mac-ssh "$SSH_CONNECT_TIMEOUT" -- \
    "$(mac_cmd true)" || true
  fb="$(probe P0 fastboot-devices "$CMD_TIMEOUT" "$(mac_cmd "$FASTBOOT devices")")" || true
  usb="$(probe P0 usb-serial "$CMD_TIMEOUT" "$(mac_cmd 'ioreg -p IOUSB -l -w0 | grep -i postmarketOS || true')")" || true
  nic="$(probe P0 usb-nic "$CMD_TIMEOUT" "$(mac_cmd 'networksetup -listallhardwareports | grep -i -A2 "USB" || true')")" || true
  if [ "$LIVE" -eq 0 ]; then
    STATE="UNKNOWN(dry-run)"
  elif [ -n "$fb" ]; then
    STATE="FASTBOOT"
  elif [ "$mesh_rc" -eq 0 ]; then
    STATE="LINUX_UP"
  elif [ -n "$usb" ]; then
    STATE="STUCK_EARLY"
  else
    STATE="ABSENT"
  fi
  log "P0 state: $STATE"
  receipt P0 classify "$STATE" ""
}

p1() {
  # Only meaningful when pmOS is up. Record, copy back, DO NOT reboot here.
  if [ "$STATE" != "LINUX_UP" ]; then
    receipt P1 skip skipped "state=$STATE"
    return 0
  fi
  exec_cmd P1 cmdline "$CMD_TIMEOUT" -- "$(phone_cmd 'cat /proc/cmdline')" || true
  exec_cmd P1 cryptkey "$CMD_TIMEOUT" -- \
    "$(phone_cmd 'grep -o "cryptkey=[^[:space:]]*" /proc/cmdline || echo NO-CRYPTKEY')" || true
  exec_cmd P1 keyslots "$CMD_TIMEOUT" -- \
    "$(phone_cmd 'sudo cryptsetup status cryptroot || true')" || true
  exec_cmd P1 journal-last-boot "$CMD_TIMEOUT" -- \
    "$(phone_cmd 'journalctl -b -1 --no-pager')" || true   # the 8:55 AM PT RAM-boot run
  exec_cmd P1 copyback "$CMD_TIMEOUT" -- \
    "ssh -o ConnectTimeout=$SSH_CONNECT_TIMEOUT $MAC_SSH 'tar czf - phone-logs 2>/dev/null' > $OUT_DIR/phone-logs.tar.gz || true" || true
  if [ "$LIVE" -eq 1 ]; then
    receipt P1 reboot not-rebooting "P2 owns the bootloader reboot; never reboot casually (LUKS)"
  else
    receipt P1 reboot skipped "dry-run"
  fi
}

p2() {
  # Get to fastboot and confirm it.
  case "$STATE" in
    LINUX_UP)
      exec_cmd P2 reboot-bootloader "$CMD_TIMEOUT" -- \
        "$(phone_cmd 'systemctl reboot --reboot-argument=bootloader')" || true
      ;;
    STUCK_EARLY)
      receipt P2 note noted "STUCK_EARLY: operator power-cycle needed, then re-run"
      return 0
      ;;
    FASTBOOT)
      receipt P2 note noted "already in fastboot"
      return 0
      ;;
    *)
      receipt P2 skip skipped "state=$STATE"
      return 0
      ;;
  esac
  exec_cmd P2 poll-fastboot "$P2_TIMEOUT" -- \
    "$(mac_cmd "for i in \$(seq 1 60); do $FASTBOOT devices | grep -q . && exit 0; sleep 5; done; exit 1")"
}

p3() {
  # U-Boot via RAM boot only; console on the Mac's USB ACM device.
  # Only valid once the device is (or is expected to be) in fastboot.
  if [ "$LIVE" -eq 1 ] && { [ "$STATE" = "ABSENT" ] || [ "$STATE" = "STUCK_EARLY" ]; }; then
    receipt P3 skip skipped "state=$STATE: no fastboot device to boot"
    return 0
  fi
  exec_cmd P3 uboot-ramboot "$CMD_TIMEOUT" -- "$(mac_cmd "$FASTBOOT boot $MAC_UBOOT_IMG")" || {
    receipt P3 abort aborted "fastboot boot failed"
    return 1
  }
  find_console P3
  if [ -z "$ACM_DEV" ]; then
    receipt P3 console-skip skipped "no ACM console found; skipping console steps"
    return 0
  fi
  probe P3 console-capture "$P3_TIMEOUT" -- \
    "$(mac_cmd "stty -f $ACM_DEV 115200 raw -echo; perl -e 'alarm shift; exec @ARGV' $P3_TIMEOUT cat $ACM_DEV")" || true
  if [ "$LIVE" -eq 0 ]; then
    receipt P3 banner skipped "dry-run"
  elif ! grep -qi 'U-Boot' "$OUT_DIR/console-capture.out" 2>/dev/null; then
    receipt P3 banner missing "no U-Boot banner in console capture"
  else
    receipt P3 banner ok ""
  fi
  # bdinfo only. NO 'reset': on 2026-10-25 leaving U-Boot via its own reset after an ABL 'fastboot boot'
  # coincided with the phone dropping into Qualcomm EDL/ramdump (05c6:9008), which needs a person.
  # Until that is understood the run stops here and asks for a forced restart (long-press Power).
  exec_cmd P3 bdinfo "$CMD_TIMEOUT" -- \
    "$(mac_cmd "printf 'bdinfo\n' > $ACM_DEV")" || true
  receipt P3 await_operator "no U-Boot reset (EDL risk); operator long-press Power returns to pmOS"
}

p4() {
  # Redox: only with --redox; exactly one try per run.
  if [ "$REDOX" -ne 1 ]; then
    receipt P4 skip skipped "--redox not passed"
    return 0
  fi
  local marker="$OUT_DIR/redox-tried"
  if [ -f "$marker" ]; then
    receipt P4 refuse refused "one Redox try per run (marker exists)"
    return 1
  fi
  : > "$marker"
  # The phone's storage is UFS (no mmc) and the payloads are NOT on the
  # phone: load them over USB through U-Boot's own fastboot gadget. The
  # phone has ONE DWC3 gadget — the moment `fastboot usb 0` starts the
  # fastboot gadget the CDC-ACM console is GONE, so the console is typed
  # exactly ONCE and U-Boot is then driven exclusively via fastboot
  # oem run (CONFIG_FASTBOOT_OEM_RUN=y, phone-boot/u-boot-build/.config):
  #   console (once): fastboot usb 0 -l <addr> -s <size>
  #       starts the gadget and pins the download buffer (cmd/fastboot.c
  #       do_fastboot_usb; -l/-s override the buffer per
  #       doc/android/fastboot.rst; the boot menu's "Enable fastboot
  #       mode" entry does the same). -l is MANDATORY:
  #       CONFIG_FASTBOOT_BUF_ADDR=0x1A000000 in that .config is outside
  #       SDM845 RAM (0x80000000+), so the default buffer is unusable.
  #   Mac: fastboot stage <file>
  #       uploads into that buffer; ${filesize} is set on completion
  #       (drivers/fastboot/fb_command.c env_set_hex("filesize", ...)).
  #   Mac: fastboot oem run:'bootefi <addr> ${fdtcontroladdr}'
  #       run_ucmd() executes the parameter via run_command()
  #       (drivers/fastboot/fb_command.c); this fork's
  #       fastboot_handle_command() splits the wire command at the first
  #       ':' (strsep), so the client syntax is fastboot oem run:'<ucmd>'
  #       — doc/android/fastboot.rst example:
  #       fastboot oem run:'echo Hello at $baudrate baud'. U-Boot's hush
  #       expands ${fdtcontroladdr}, not the Mac's shell (single quotes).
  # This fork does NOT set ${fastboot_buffer}/${fastboot_blocks}, so
  # bootefi boots from the pinned -l address. efi-diag first, then
  # redox-ramboot — both without ever touching the dead ACM console.
  local fb_addr=0xa0000000 fb_size=0x6000000
  exec_cmd P4 uboot-again "$CMD_TIMEOUT" -- "$(mac_cmd "$FASTBOOT boot $MAC_UBOOT_IMG")" || true
  find_console P4
  if [ -z "$ACM_DEV" ]; then
    receipt P4 console-skip skipped "no ACM console; cannot start the fastboot gadget from U-Boot"
    return 0
  fi
  exec_cmd P4 gadget-start "$CMD_TIMEOUT" -- \
    "$(mac_cmd "printf 'fastboot usb 0 -l $fb_addr -s $fb_size\\n' > $ACM_DEV")" || true
  exec_cmd P4 efi-diag-stage "$P4_TIMEOUT" -- \
    "$(mac_cmd "$FASTBOOT stage $MAC_EFI_DIAG")" || true
  exec_cmd P4 efi-diag-run "$P4_TIMEOUT" -- \
    "$(mac_cmd "$FASTBOOT oem run:'bootefi $fb_addr \${fdtcontroladdr}'")" || true
  exec_cmd P4 redox-stage "$P4_TIMEOUT" -- \
    "$(mac_cmd "$FASTBOOT stage $MAC_REDOX_IMG")" || true
  exec_cmd P4 redox-run "$P4_TIMEOUT" -- \
    "$(mac_cmd "$FASTBOOT oem run:'bootefi $fb_addr \${fdtcontroladdr}'")" || true
  # Read-only watch: the ACM device may reappear if a payload exits and
  # U-Boot re-enables the console; never write to it after the switch.
  probe P4 console-watch "$P4_TIMEOUT" -- \
    "$(mac_cmd "perl -e 'alarm shift; exec @ARGV' $P4_TIMEOUT cat $ACM_DEV")" || true
  if [ "$LIVE" -eq 0 ]; then
    receipt P4 console-check skipped "dry-run"
  elif [ ! -s "$OUT_DIR/console-watch.out" ]; then
    log "P4: no console after Redox — awaiting operator power-button."
    receipt P4 awaiting-operator noted "no console after Redox; operator must press the power button"
  else
    receipt P4 console-check ok ""
  fi
}

# ---------- main ----------
log "runbook: run-id=$RUN_ID mode=$([ "$LIVE" -eq 1 ] && echo LIVE || echo DRY-RUN) redox=$REDOX"
log "receipt: $RECEIPT"
p0
p1
p2
stage_payloads
p3
p4
log "done. state=$STATE receipt=$RECEIPT"
