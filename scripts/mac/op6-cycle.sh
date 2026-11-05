#!/usr/bin/env bash
# op6-cycle - one hands-free OnePlus 6 RAM-boot cycle (P1, PLAN-20261031-agent-control).
#
# pmOS -> reboot bootloader -> ONE RAM-only `fastboot boot` -> classify the USB
# outcome -> JSON receipt. Nothing is flashed; no EDL tooling exists here.
#
# Usage: op6-cycle <image> [--ttl-expect N] [--webcam] [--attended [--attended-by NOTE]]
# An allowlist entry whose 5th field is ATTENDED-ONLY refuses to run
# unless --attended is passed (first device runs of new hardware
# writes: P3 watchdog, P4 PON, P5 SMMU). --attended records operator
# APPROVAL, not physical presence: the operator may approve remotely,
# and --attended-by NOTE says who/how (stored as receipt field
# "attended_by"; --attended-by requires --attended).
#
# Hard refusals (exit 4, nothing sent to the phone):
#   - image sha256 not pinned in the allowlist (or basename mismatch)
#   - allowlist entry has no TTL or no 8 GiB gate-evidence reference
#   - --ttl-expect disagrees with the allowlist TTL
#   - battery unreadable or below the floor (30%)
#   - phone not reachable in pmOS at start
#   - previous receipt in the ledger is not PASS (stop-on-first-non-PASS)
#   - another op6-cycle is running (lock dir)
# Outcomes: PASS (exit 0), ABORTED_NO_FASTBOOT / BOOT_CMD_FAILED (exit 1),
#           CRASHDUMP (exit 2), HUNG (exit 3), FASTBOOT (exit 5).
# Test hooks (env): OP6_CYCLE_FB, OP6_CYCLE_SSH, OP6_CYCLE_USB_CMD,
#   OP6_CYCLE_ALLOWLIST, OP6_CYCLE_RECEIPTS, OP6_CYCLE_POLL,
#   OP6_CYCLE_BL_WAIT, OP6_CYCLE_SLACK, OP6_CYCLE_BATTERY_FLOOR,
#   OP6_CYCLE_WEBCAM (capture helper path; --webcam enables),
#   OP6_CYCLE_WEBCAM_AT (seconds after boot to start capturing; default
#   TTL-10), OP6_CYCLE_OCR (OCR helper; default: op6-ocr next to the
#   capture helper), OP6_CYCLE_SAHARA_DIR (crashdump-watcher receipts
#   dir; default $HOME/op6-r1/out/sahara-receipts),
#   OP6_CYCLE_RECOVERY_WAIT_S (CRASHDUMP watcher-recovery wait bound;
#   default 90). The webcam step is fail-soft (R-3): burst capture
#   every poll from WEBCAM_AT until classification ends, keep the last
#   3 frames, OCR them, and record which frame shows the reset (the
#   SCM result line, per REVIEW-20261031-1930 F-1). It only adds
#   receipt fields, never changes outcomes.
#
# CRASHDUMP receipts gain a "recovery" object (REVIEW-20261101-0930
# A-2): after classification the cycle releases its lock (the
# crashdump watcher defers while a cycle holds it) and waits up to
# OP6_CYCLE_RECOVERY_WAIT_S for the watcher's RECOVERED line in
# $OP6_CYCLE_SAHARA_DIR/recovery.log, recording
# {watcher_recovered, recovered_at, sahara_receipt}. The outcome
# stays CRASHDUMP and the exit code stays 2 either way.
#
# FASTBOOT outcome (exit 5; operator order 2026-11-02 item 3,
# design reviewed by Muse Wonder): an image may deliberately land
# in fastboot after its reset (P4's PON reboot-reason byte). The
# detector ARMS only once the boot is proven - fastboot boot
# returned OK (a failure exits earlier) AND a post-boot poll saw a
# non-d00d USB state (the phone actually left the bootloader; a
# never-booted phone sits in d00d from the start and must stay
# HUNG). Once armed, d00d persisting past TTL +
# OP6_CYCLE_FASTBOOT_GRACE (default 30) for at least
# OP6_CYCLE_FASTBOOT_PERSIST_S (default 5) classifies FASTBOOT;
# the same rule applies at the deadline. Closeout issues
# `fastboot reboot` and waits up to
# OP6_CYCLE_FASTBOOT_RECOVERY_WAIT_S (default 120) for pmOS,
# recording "recovery":{"recovered_to_pmos":bool,...}. The
# ledger accepts a FASTBOOT head only when recovered_to_pmos is
# true. Receipts are evidence: they are never moved or archived
# to fix the ledger; context goes in HANDOFF or a sidecar note.
set -u

SERIAL_DEFAULT=0a1b2c3d
FB=${OP6_CYCLE_FB:-$HOME/tools/platform-tools/fastboot}
SSH_PM=${OP6_CYCLE_SSH:-$HOME/.local/bin/ssh-op6cf}
ALLOWLIST=${OP6_CYCLE_ALLOWLIST:-$HOME/op6-r1/op6-cycle.allowlist}
RECEIPTS=${OP6_CYCLE_RECEIPTS:-$HOME/op6-r1/out/device-receipts}
POLL=${OP6_CYCLE_POLL:-2}
BL_WAIT=${OP6_CYCLE_BL_WAIT:-240}
SLACK=${OP6_CYCLE_SLACK:-180}
BATTERY_FLOOR=${OP6_CYCLE_BATTERY_FLOOR:-30}
SERIAL=${OP6_CYCLE_SERIAL:-$SERIAL_DEFAULT}
WEBCAM=${OP6_CYCLE_WEBCAM:-}
SAHARA_DIR=${OP6_CYCLE_SAHARA_DIR:-$HOME/op6-r1/out/sahara-receipts}
RECOVERY_LOG=$SAHARA_DIR/recovery.log
RECOVERY_WAIT=${OP6_CYCLE_RECOVERY_WAIT_S:-90}
FB_GRACE=${OP6_CYCLE_FASTBOOT_GRACE:-30}
FB_PERSIST=${OP6_CYCLE_FASTBOOT_PERSIST_S:-5}
FB_RECOVERY_WAIT=${OP6_CYCLE_FASTBOOT_RECOVERY_WAIT_S:-120}
LOCK=$RECEIPTS/.op6-cycle.lock

iso(){ date -u '+%Y-%m-%dT%H:%M:%SZ'; }
esc(){ printf '%s' "$1" | sed 's/\\/\\\\/g; s/"/\\"/g'; }

IMG=""
TTL_EXPECT=""
ATTENDED=""
ATTENDED_BY=""
while [ $# -gt 0 ]; do
  case "$1" in
    --ttl-expect) TTL_EXPECT=$2; shift 2;;
    --attended) ATTENDED=1; shift;;
    --attended-by)
      [ $# -ge 2 ] || { echo "op6-cycle: --attended-by needs a note argument" >&2; exit 64; }
      ATTENDED_BY=$2; shift 2;;
    --webcam) [ -n "$WEBCAM" ] || WEBCAM=$HOME/op6-r1/op6-webcam.sh; shift;;
    --) shift; IMG=$1; shift;;
    -*) echo "op6-cycle: unknown option $1" >&2; exit 64;;
    *) IMG=$1; shift;;
  esac
done
[ -n "$IMG" ] || { echo "usage: op6-cycle <image> [--ttl-expect N] [--attended [--attended-by NOTE]] [--webcam]" >&2; exit 64; }
[ -f "$IMG" ] || { echo "op6-cycle: image not found: $IMG" >&2; exit 64; }
if [ -n "$ATTENDED_BY" ] && [ -z "$ATTENDED" ]; then
  echo "op6-cycle: --attended-by requires --attended" >&2; exit 64
fi

mkdir -p "$RECEIPTS/refusals"
STAMP=$(date -u '+%Y%m%dT%H%M%SZ')
LOG=$RECEIPTS/run-$STAMP.log
exec > >(tee -a "$LOG") 2>&1
echo "$(iso) op6-cycle starting; image=$IMG log=$LOG"

refuse(){
  echo "$(iso) REFUSED: $1"
  printf '{"tool":"op6-cycle","outcome":"REFUSED","reason":"%s","image":"%s","at":"%s"}\n' \
    "$(esc "$1")" "$(esc "$IMG")" "$(iso)" > "$RECEIPTS/refusals/refusal-$STAMP.json"
  exit 4
}

# --- lock (mkdir is atomic) ---
if ! mkdir "$LOCK" 2>/dev/null; then refuse "another op6-cycle is running (lock $LOCK)"; fi
trap 'rmdir "$LOCK" 2>/dev/null' EXIT

# --- allowlist pin: "<sha256> <basename> <ttl_seconds> <gate_evidence_ref>" ---
SHA=$(shasum -a 256 "$IMG" | cut -d' ' -f1)
BASE=$(basename "$IMG")
ENTRY=""
while read -r a_sha a_base a_ttl a_gate a_mode _rest; do
  case "$a_sha" in ''|\#*) continue;; esac
  if [ "$a_sha" = "$SHA" ] && [ "$a_base" = "$BASE" ]; then
    ENTRY="$a_sha $a_base $a_ttl $a_gate"; ENTRY_TTL=$a_ttl; ENTRY_GATE=$a_gate; ENTRY_MODE=${a_mode:-}
  fi
done < "$ALLOWLIST"
[ -n "$ENTRY" ] || refuse "image sha256 $SHA ($BASE) not in allowlist $ALLOWLIST"
if [ "${ENTRY_MODE:-}" = "ATTENDED-ONLY" ] && [ -z "$ATTENDED" ]; then
  refuse "image is ATTENDED-ONLY (new hardware write - operator approval required); rerun with --attended"
fi
if [ -n "$ATTENDED" ]; then
  if [ -n "$ATTENDED_BY" ]; then
    echo "$(iso) attended run (--attended given; operator-approved: $ATTENDED_BY)"
  else
    echo "$(iso) attended run (--attended given; operator-approved)"
  fi
fi
[ -n "${ENTRY_TTL:-}" ] && [ "$ENTRY_TTL" -gt 0 ] 2>/dev/null || refuse "allowlist entry has no TTL"
[ -n "${ENTRY_GATE:-}" ] || refuse "allowlist entry has no gate-evidence reference"
if [ -n "$TTL_EXPECT" ] && [ "$TTL_EXPECT" != "$ENTRY_TTL" ]; then
  refuse "--ttl-expect $TTL_EXPECT disagrees with allowlist TTL $ENTRY_TTL"
fi
TTL=$ENTRY_TTL
echo "$(iso) pin OK: sha256=$SHA ttl=${TTL}s gate_evidence=$ENTRY_GATE"

# --- stop-on-first-non-PASS: newest ledger receipt must be PASS,
#     or FASTBOOT with recovered_to_pmos:true (a clean head: the
#     phone was verifiably returned to pmOS by the cycle itself) ---
LAST=$(ls -t "$RECEIPTS"/op6-cycle-*.json 2>/dev/null | head -1)
if [ -n "$LAST" ]; then
  LAST_OUTCOME=$(grep -o '"outcome": *"[^"]*"' "$LAST" | head -1 | sed 's/.*"\([^"]*\)"$/\1/')
  LEDGER_OK=""
  [ "$LAST_OUTCOME" = "PASS" ] && LEDGER_OK=1
  if [ "$LAST_OUTCOME" = "FASTBOOT" ] && grep -q '"recovered_to_pmos":true' "$LAST"; then LEDGER_OK=1; fi
  [ -n "$LEDGER_OK" ] || refuse "previous receipt $(basename "$LAST") outcome=$LAST_OUTCOME (stop-on-first-non-PASS)"
  echo "$(iso) previous receipt $LAST_OUTCOME (clean): $(basename "$LAST")"
fi

# --- pmOS reachability + battery floor ---
BATTERY=$("$SSH_PM" 'cat /sys/class/power_supply/*/capacity 2>/dev/null | head -1' 2>/dev/null | tr -dc '0-9')
[ -n "$BATTERY" ] || refuse "phone not reachable in pmOS (battery read failed via $SSH_PM)"
[ "$BATTERY" -ge "$BATTERY_FLOOR" ] || refuse "battery ${BATTERY}% below floor ${BATTERY_FLOOR}%"
echo "$(iso) pmOS reachable; battery ${BATTERY}%"

usb_state(){
  if [ -n "${OP6_CYCLE_USB_CMD:-}" ]; then "$OP6_CYCLE_USB_CMD"; return; fi
  system_profiler SPUSBDataType 2>/dev/null | awk '/Product ID/{p=$3} /Vendor ID/{v=$3; if (v ~ /0x18d1|0x05c6/) print v":"p}' | head -1
}

STARTED=$(iso); T0=$(date +%s)
OUTCOME=""; DETAIL=""; BOOT_AT=""; PMOS_AT=""; PRODUCT=""
CRASH_AT=""; CRASH_EPOCH=0; REC_FIELDS=""

WC_FILE=$RECEIPTS/webcam-$STAMP.fields
write_receipt(){
  local now=$(iso)
  local wc_fields=""
  if [ -n "$WEBCAM" ]; then
    if [ -s "$WC_FILE" ]; then wc_fields=",$(tr -d '\n' < "$WC_FILE")";
    else wc_fields=',"webcam_status":"not-captured"'; fi
  fi
  local attended_json=null
  [ -n "$ATTENDED" ] && attended_json="\"$(esc "$ATTENDED_BY")\""
  cat > "$RECEIPTS/op6-cycle-$STAMP.json" <<EOF
{"tool":"op6-cycle","version":1,"started":"$STARTED","finished":"$now","host":"$(esc "$(hostname)")","image":"$(esc "$IMG")","image_sha256":"$SHA","image_bytes":$(stat -f %z "$IMG" 2>/dev/null || stat -c %s "$IMG"),"ttl_seconds":$TTL,"gate_evidence":"$(esc "$ENTRY_GATE")","attended_by":$attended_json,"battery_pct_before":$BATTERY,"fastboot_serial":"$SERIAL","getvar_product":"$(esc "$PRODUCT")","boot_sent_at":"$BOOT_AT","outcome":"$OUTCOME","detail":"$(esc "$DETAIL")","pmos_returned_at":"$PMOS_AT","log":"$(esc "$LOG")"$wc_fields$REC_FIELDS}
EOF
  echo "$(iso) receipt: $RECEIPTS/op6-cycle-$STAMP.json outcome=$OUTCOME"
}

# --- pmOS -> bootloader (wait <= BL_WAIT for the serial) ---
if "$FB" devices | grep -q "$SERIAL"; then
  echo "$(iso) phone already in fastboot"
else
  echo "$(iso) pmOS -> reboot bootloader"
  "$SSH_PM" 'sudo reboot bootloader' || true
  waited=0; found=""
  while [ "$waited" -lt "$BL_WAIT" ]; do
    if "$FB" devices | grep -q "$SERIAL"; then found=1; break; fi
    sleep "$POLL"; waited=$((waited + POLL))
  done
  if [ -z "$found" ]; then
    OUTCOME="ABORTED_NO_FASTBOOT"; DETAIL="no fastboot serial $SERIAL after ${BL_WAIT}s; usb=$(usb_state)"
    write_receipt; exit 1
  fi
fi
PRODUCT=$("$FB" -s "$SERIAL" getvar product 2>&1 | head -1)
echo "$(iso) fastboot $SERIAL present; $PRODUCT"

# --- exactly ONE fastboot boot ---
BOOT_AT=$(iso)
echo "$(iso) fastboot boot (RAM only, single attempt)"
if ! "$FB" -s "$SERIAL" boot "$IMG"; then
  OUTCOME="BOOT_CMD_FAILED"; DETAIL="fastboot boot exited non-zero"
  write_receipt; exit 1
fi
BOOT_EPOCH=$(date +%s)
if [ -n "${OP6_CYCLE_WEBCAM_AT:-}" ]; then WC_AT=$((BOOT_EPOCH + OP6_CYCLE_WEBCAM_AT));
else WC_AT=$((BOOT_EPOCH + TTL - 10)); [ "$WC_AT" -le "$BOOT_EPOCH" ] && WC_AT=$((BOOT_EPOCH + 1)); fi
OCR=${OP6_CYCLE_OCR:-}
[ -n "$WEBCAM" ] && [ -z "$OCR" ] && OCR=$(dirname "$WEBCAM")/op6-ocr
WC_FRAMES=""; WC_SHAS=""; WC_COUNT=0; WC_FAILS=0; WC_STOP=""; WC_STATUS=""; WC_DETAIL=""
DEADLINE=$((BOOT_EPOCH + TTL + SLACK))
echo "$(iso) boot sent; classifying until $(date -u -r "$DEADLINE" '+%H:%M:%SZ' 2>/dev/null || echo "+$((TTL + SLACK))s") (TTL ${TTL}s + slack ${SLACK}s)"

# Burst capture (F-1): one frame per poll once the window opens. The
# helper contract: op6-webcam.sh <frame.jpg> prints "ok <sha256>" or a
# single status token (not-authorized / no-camera / capture-failed /
# unavailable). A hard status stops the burst; capture-failed retries
# up to 3 times. Nothing here may affect the cycle outcome.
wc_capture(){
  local frame="$RECEIPTS/panel-$STAMP-b$(printf '%02d' $((WC_COUNT + WC_FAILS + 1))).jpg"
  local out st
  if [ ! -x "$WEBCAM" ]; then WC_STATUS="unavailable"; WC_DETAIL="helper not executable: $WEBCAM"; WC_STOP=1; return; fi
  out=$("$WEBCAM" "$frame" 2>/dev/null); st=${out%% *}
  case "$st" in
    ok) WC_COUNT=$((WC_COUNT + 1)); WC_FRAMES="$WC_FRAMES $frame"; WC_SHAS="$WC_SHAS ${out#ok }";;
    not-authorized|no-camera|unavailable) WC_STATUS=$st; WC_STOP=1;;
    *) WC_STATUS="capture-failed"; WC_FAILS=$((WC_FAILS + 1)); [ "$WC_FAILS" -ge 3 ] && WC_STOP=1;;
  esac
}

# After classification: prune to the last 3 frames, OCR the kept ones,
# find the frame whose text shows the reset, and write the fields file
# that write_receipt merges. Runs before write_receipt on loop exits.
finalize_webcam(){
  [ -n "$WEBCAM" ] || return 0
  if [ "$WC_COUNT" -eq 0 ]; then
    if [ -n "$WC_STATUS" ]; then
      printf '"webcam_status":"%s","webcam_detail":"%s"' "$WC_STATUS" "$(esc "$WC_DETAIL")" > "$WC_FILE"
    fi
    return 0
  fi
  local kept="" kept_shas="" n=0 total=$WC_COUNT
  for f in $WC_FRAMES; do
    n=$((n + 1))
    if [ $((total - n)) -lt 3 ]; then kept="$kept $f"; else rm -f "$f"; fi
  done
  n=0; local ki=0
  for s in $WC_SHAS; do n=$((n + 1)); [ $((total - n)) -lt 3 ] && kept_shas="$kept_shas $s"; done
  local scm="null" ocr_pick="" idx=0 last_sha=""
  for f in $kept; do
    local txt=""
    [ -x "$OCR" ] && txt=$("$OCR" "$f" 2>/dev/null | tr '\n' ' ')
    if [ "$scm" = "null" ] && printf '%s' "$txt" | grep -qiE 'kreset:|download-mode'; then scm=$idx; ocr_pick=$txt; fi
    [ -z "$ocr_pick" ] && ocr_pick=$txt
    idx=$((idx + 1))
  done
  for s in $kept_shas; do last_sha=$s; done
  printf '"webcam_status":"ok","webcam_burst_frames":%s,"webcam_frames_kept":%s,"webcam_frame_sha256":"%s","webcam_scm_frame":%s,"webcam_ocr_text":"%s"' \
    "$total" "$idx" "$last_sha" "$scm" "$(esc "$(printf '%s' "$ocr_pick" | cut -c1-400)")" > "$WC_FILE"
  echo "$(iso) webcam burst: $total frames, kept $idx, scm_frame=$scm"
}

# CRASHDUMP only: enrich the receipt with the crashdump watcher's
# recovery (REVIEW-20261101-0930 A-2). The watcher
# (com.redox.op6-crashdump-watch) DEFERS while a cycle holds the
# lock, so release the lock first, then wait up to RECOVERY_WAIT for
# its RECOVERED line in the recovery log, e.g.:
#   2026-11-01T21:22:48Z RECOVERED: pmOS back (tool rc=0)
# The watcher writes RECOVERED only after seeing pmOS USB
# (0x18d1:0xd001) again, so the line is itself the phone-is-back
# confirmation; the current USB state is logged for the record but
# does not gate the match. The watcher's run also leaves a
# "receipt <path> outcome=..." line in the same log naming the
# per-event sahara JSON. Sets REC_FIELDS for write_receipt; the
# outcome stays CRASHDUMP and the exit code stays 2 either way.
await_recovery(){
  REC_FIELDS=',"recovery":{"watcher_recovered":false}'
  [ -n "$CRASH_AT" ] || { CRASH_AT=$STARTED; CRASH_EPOCH=$T0; }
  if [ -d "$LOCK" ]; then
    rmdir "$LOCK" 2>/dev/null
    echo "$(iso) crashdump: released cycle lock so the sahara watcher can act"
  fi
  echo "$(iso) crashdump: waiting up to ${RECOVERY_WAIT}s for watcher recovery in $RECOVERY_LOG"
  local deadline=$(( $(date +%s) + RECOVERY_WAIT ))
  local line="" ts="" sahara="" m=""
  while [ "$(date +%s)" -lt "$deadline" ]; do
    line=$(grep ' RECOVERED: ' "$RECOVERY_LOG" 2>/dev/null | tail -1)
    if [ -n "$line" ]; then
      ts=${line%% *}
      if [[ ! "$ts" < "$CRASH_AT" ]]; then
        sahara=$(awk -v lo="$CRASH_AT" -v hi="$ts" '$2 == "receipt" && $1 >= lo && $1 <= hi { p=$3 } END { print p }' "$RECOVERY_LOG" 2>/dev/null)
        if [ -z "$sahara" ]; then
          sahara=$(ls -t "$SAHARA_DIR"/sahara-*.json 2>/dev/null | head -1)
          if [ -n "$sahara" ]; then
            m=$(stat -f %m "$sahara" 2>/dev/null || stat -c %Y "$sahara" 2>/dev/null)
            if [ -z "$m" ] || [ "$m" -lt "$CRASH_EPOCH" ]; then sahara=""; fi
          fi
        fi
        REC_FIELDS=",\"recovery\":{\"watcher_recovered\":true,\"recovered_at\":\"$ts\",\"sahara_receipt\":\"$(esc "$sahara")\"}"
        echo "$(iso) crashdump: watcher recovered the phone at $ts (sahara receipt: ${sahara:-unknown}; usb now: $(usb_state))"
        return 0
      fi
    fi
    sleep "$POLL"
  done
  echo "$(iso) crashdump: no watcher recovery within ${RECOVERY_WAIT}s"
}

# FASTBOOT only: bring the phone home before writing the receipt.
# A fastboot landing is a known, software-recoverable state: issue
# `fastboot reboot` and wait for pmOS USB. Sets REC_FIELDS with
# recovered_to_pmos; the outcome stays FASTBOOT and the exit code
# stays 5 either way. If recovery fails the receipt is NOT a clean
# ledger head, by design.
recover_fastboot(){
  echo "$(iso) fastboot landing: issuing fastboot reboot, waiting up to ${FB_RECOVERY_WAIT}s for pmOS"
  "$FB" -s "$SERIAL" reboot >/dev/null 2>&1 || true
  local deadline=$(( $(date +%s) + FB_RECOVERY_WAIT ))
  while [ "$(date +%s)" -lt "$deadline" ]; do
    if [ "$(usb_state)" = "0x18d1:0xd001" ]; then
      PMOS_AT=$(iso)
      REC_FIELDS=",\"recovery\":{\"recovered_to_pmos\":true,\"recovered_at\":\"$PMOS_AT\",\"method\":\"fastboot reboot\"}"
      echo "$(iso) fastboot landing: pmOS back at $PMOS_AT"
      return 0
    fi
    sleep "$POLL"
  done
  REC_FIELDS=',"recovery":{"recovered_to_pmos":false,"method":"fastboot reboot"}'
  echo "$(iso) fastboot landing: pmOS did not return within ${FB_RECOVERY_WAIT}s (phone left in fastboot; recover by hand)"
}

EXIT_CODE=3
last=""
FB_ARMED=""; FB_SINCE=0
while [ "$(date +%s)" -lt "$DEADLINE" ]; do
  now=$(usb_state)
  ep=$(date +%s)
  if [ "$now" != "$last" ]; then echo "$(iso) USB: ${now:-none}"; last=$now; fi
  if [ -n "$WEBCAM" ] && [ -z "$WC_STOP" ] && [ "$(date +%s)" -ge "$WC_AT" ]; then wc_capture; fi
  case "$now" in
    0x18d1:0xd001) OUTCOME="PASS"; PMOS_AT=$(iso); DETAIL="pmOS USB gadget enumerated after self-reset"; EXIT_CODE=0; break;;
    0x05c6:0x900e|0x05c6:0x9008) OUTCOME="CRASHDUMP"; CRASH_AT=$(iso); CRASH_EPOCH=$(date +%s); DETAIL="Qualcomm crash-dump/download mode USB id $now"; EXIT_CODE=2; break;;
    0x18d1:0xd00d)
      # fastboot sighting: only meaningful once armed (see header)
      if [ -n "$FB_ARMED" ]; then
        [ "$FB_SINCE" -eq 0 ] && FB_SINCE=$ep
        if [ "$ep" -ge $((BOOT_EPOCH + TTL + FB_GRACE)) ] && [ $((ep - FB_SINCE)) -ge "$FB_PERSIST" ]; then
          OUTCOME="FASTBOOT"; DETAIL="landed in fastboot (18d1:d00d) after self-reset and stayed; PON-style bootloader landing"; EXIT_CODE=5; break
        fi
      fi;;
    *)
      # any non-fastboot state after the boot proves the phone left
      # the bootloader: arm the FASTBOOT detector, reset persistence
      FB_ARMED=1; FB_SINCE=0;;
  esac
  sleep "$POLL"
done
if [ -z "$OUTCOME" ]; then
  ep=$(date +%s)
  if [ -n "$FB_ARMED" ] && [ "$last" = "0x18d1:0xd00d" ] && [ "$FB_SINCE" -gt 0 ] && [ $((ep - FB_SINCE)) -ge "$FB_PERSIST" ]; then
    OUTCOME="FASTBOOT"; DETAIL="landed in fastboot (18d1:d00d) after self-reset and stayed until the deadline"; EXIT_CODE=5
  else
    OUTCOME="HUNG"; DETAIL="no pmOS USB after TTL+${SLACK}s; usb=$(usb_state)"; EXIT_CODE=3
  fi
fi
finalize_webcam
[ "$OUTCOME" = "CRASHDUMP" ] && await_recovery
[ "$OUTCOME" = "FASTBOOT" ] && recover_fastboot
write_receipt; exit $EXIT_CODE
