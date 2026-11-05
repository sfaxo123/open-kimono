#!/usr/bin/env bash
# op6-cycle unit tests - fake fastboot / pmOS ssh / USB state. No device touched.
# Run on the Mac (or any bash host): bash op6-cycle-tests.sh [path-to-op6-cycle]
set -u
TOOL=${1:-$HOME/op6-r1/op6-cycle}
WORK=$(mktemp -d /tmp/op6-cycle-tests.XXXXXX)
PASS=0; FAIL=0
say(){ printf '%s\n' "$*"; }
check(){ # check <name> <condition...>
  local name=$1; shift
  if "$@"; then PASS=$((PASS+1)); say "ok   - $name"; else FAIL=$((FAIL+1)); say "FAIL - $name"; fi
}

FAKEBIN=$WORK/fakebin; mkdir -p "$FAKEBIN"
cat > "$FAKEBIN/fastboot" <<'EOF'
#!/usr/bin/env bash
S=$FAKE_STATE
case "$1" in
  devices)
    [ -f "$S/fastboot_present" ] && printf '0a1b2c3d\tfastboot\n';;
  -s)
    case "$3" in
      getvar) echo "product: sdm845";;
      boot)
        echo "boot $4" >> "$S/calls"
        if [ -f "$S/boot_fail" ]; then echo "FAILED (remote: 'fake failure')" >&2; exit 1; fi
        rm -f "$S/fastboot_present";;
      reboot)
        echo "reboot" >> "$S/calls"
        printf '0x18d1:0xd001\n' >> "$S/usb_seq";;
    esac;;
esac
exit 0
EOF
cat > "$FAKEBIN/ssh-op6cf" <<'EOF'
#!/usr/bin/env bash
S=$FAKE_STATE
case "$*" in
  *capacity*) cat "$S/battery";;
  *"reboot bootloader"*)
    echo "reboot-bootloader" >> "$S/calls"
    [ -f "$S/no_fb" ] || touch "$S/fastboot_present";;
esac
exit 0
EOF
cat > "$FAKEBIN/usb" <<'EOF'
#!/usr/bin/env bash
S=$FAKE_STATE
C=0; [ -f "$S/usb_counter" ] && C=$(cat "$S/usb_counter")
echo $((C+1)) > "$S/usb_counter"
LINE=$(sed -n "$((C+1))p" "$S/usb_seq")
[ -z "$LINE" ] && LINE=$(tail -1 "$S/usb_seq")
[ "$LINE" = "NONE" ] || printf '%s\n' "$LINE"
exit 0
EOF
cat > "$FAKEBIN/webcam-ok" <<'EOF'
#!/usr/bin/env bash
printf 'fake-frame' > "$1"
echo "ok aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
exit 0
EOF
cat > "$FAKEBIN/webcam-denied" <<'EOF'
#!/usr/bin/env bash
echo "not-authorized"
exit 3
EOF
cat > "$FAKEBIN/ocr-fake" <<'EOF'
#!/usr/bin/env bash
printf 'ram:~$ reset\nkreset: holding 5 s before SYSTEM_RESET (panel capture window)\n'
exit 0
EOF
chmod +x "$FAKEBIN"/*

new_case(){
  CASE=$WORK/$1; mkdir -p "$CASE/state" "$CASE/receipts" "$CASE/sahara"
  echo 100 > "$CASE/state/battery"
  printf 'NONE\n' > "$CASE/state/usb_seq"
  : > "$CASE/state/calls"
  IMG=$CASE/test.img; printf 'fake-image-bytes' > "$IMG"
  SHA=$(shasum -a 256 "$IMG" | cut -d' ' -f1)
  printf '%s %s %s %s\n' "$SHA" "test.img" "${2:-120}" "gate-evidence:RESULT.json#deadbeef" > "$CASE/allowlist"
}
run_tool(){
  FAKE_STATE=$CASE/state \
  OP6_CYCLE_FB=$FAKEBIN/fastboot OP6_CYCLE_SSH=$FAKEBIN/ssh-op6cf OP6_CYCLE_USB_CMD=$FAKEBIN/usb \
  OP6_CYCLE_ALLOWLIST=$CASE/allowlist OP6_CYCLE_RECEIPTS=$CASE/receipts \
  OP6_CYCLE_SAHARA_DIR=$CASE/sahara OP6_CYCLE_RECOVERY_WAIT_S=${REC_WAIT:-2} \
  OP6_CYCLE_FASTBOOT_GRACE=${FB_GRACE:-30} OP6_CYCLE_FASTBOOT_PERSIST_S=${FB_PERSIST:-5} \
  OP6_CYCLE_FASTBOOT_RECOVERY_WAIT_S=${FB_REC_WAIT:-120} \
  OP6_CYCLE_WEBCAM=${WC:-} OP6_CYCLE_WEBCAM_AT=${WC_AT:-1} OP6_CYCLE_OCR=${OCR:-} \
  OP6_CYCLE_POLL=1 OP6_CYCLE_BL_WAIT=${BL_WAIT:-10} OP6_CYCLE_SLACK=${SLACK:-3} \
  bash "$TOOL" "$IMG" "$@" > "$CASE/tool.out" 2>&1
  echo $?
}
outcome(){ grep -o '"outcome": *"[^"]*"' "$CASE/receipts"/op6-cycle-*.json 2>/dev/null | head -1 | sed 's/.*"\([^"]*\)"$/\1/'; }
boots(){ grep -c '^boot ' "$CASE/state/calls"; }

# 1. PASS
new_case pass; printf 'NONE\nNONE\n0x18d1:0xd001\n' > "$CASE/state/usb_seq"
RC=$(run_tool)
check "pass: exit 0" [ "$RC" = 0 ]
check "pass: outcome PASS" [ "$(outcome)" = "PASS" ]
check "pass: exactly one boot" [ "$(boots)" = 1 ]

# 2. CRASHDUMP
new_case crash; printf 'NONE\n0x05c6:0x9008\n' > "$CASE/state/usb_seq"
RC=$(run_tool)
check "crashdump: exit 2" [ "$RC" = 2 ]
check "crashdump: outcome CRASHDUMP" [ "$(outcome)" = "CRASHDUMP" ]

# 3. HUNG (ttl 2 + slack 3)
new_case hung 2
RC=$(run_tool)
check "hung: exit 3" [ "$RC" = 3 ]
check "hung: outcome HUNG" [ "$(outcome)" = "HUNG" ]

# 4. REFUSED - sha not in allowlist
new_case refuse_sha; printf '%064d %s 120 gate-evidence:x\n' 0 "test.img" > "$CASE/allowlist"
RC=$(run_tool)
check "refuse sha: exit 4" [ "$RC" = 4 ]
check "refuse sha: no boot attempted" [ "$(boots)" = 0 ]
check "refuse sha: refusal receipt written" test -n "$(ls "$CASE/receipts/refusals/"*.json 2>/dev/null)"

# 5. REFUSED - battery below floor
new_case refuse_batt; echo 20 > "$CASE/state/battery"
RC=$(run_tool)
check "refuse battery: exit 4" [ "$RC" = 4 ]
check "refuse battery: no boot attempted" [ "$(boots)" = 0 ]

# 6. REFUSED - previous receipt not PASS
new_case refuse_prev
printf '{"tool":"op6-cycle","outcome":"CRASHDUMP"}\n' > "$CASE/receipts/op6-cycle-20000101T000000Z.json"
RC=$(run_tool)
check "refuse prev: exit 4" [ "$RC" = 4 ]
check "refuse prev: no boot attempted" [ "$(boots)" = 0 ]

# 7. REFUSED - ttl-expect mismatch
new_case refuse_ttl
RC=$(run_tool --ttl-expect 999)
check "refuse ttl: exit 4" [ "$RC" = 4 ]

# 8. ABORTED_NO_FASTBOOT
new_case nofb; touch "$CASE/state/no_fb"; BL_WAIT=4
RC=$(run_tool)
check "no-fastboot: exit 1" [ "$RC" = 1 ]
check "no-fastboot: outcome ABORTED_NO_FASTBOOT" [ "$(outcome)" = "ABORTED_NO_FASTBOOT" ]
check "no-fastboot: no boot attempted" [ "$(boots)" = 0 ]

# 9. PASS with webcam burst ok (F-1: burst fields + SCM frame detection)
new_case webcam_ok; printf 'NONE\nNONE\n0x18d1:0xd001\n' > "$CASE/state/usb_seq"
WC=$FAKEBIN/webcam-ok; OCR=$FAKEBIN/ocr-fake
RC=$(run_tool --webcam)
check "webcam ok: exit 0" [ "$RC" = 0 ]
check "webcam ok: outcome PASS" [ "$(outcome)" = "PASS" ]
check "webcam ok: receipt has webcam_status ok" grep -q '"webcam_status":"ok"' "$CASE/receipts"/op6-cycle-*.json
check "webcam ok: receipt has frame sha" grep -q '"webcam_frame_sha256":"aaaa' "$CASE/receipts"/op6-cycle-*.json
check "webcam ok: burst frames >= 1" grep -qE '"webcam_burst_frames":[1-9]' "$CASE/receipts"/op6-cycle-*.json
check "webcam ok: scm frame detected" grep -qE '"webcam_scm_frame":[0-9]' "$CASE/receipts"/op6-cycle-*.json
check "webcam ok: kept frames <= 3" grep -qE '"webcam_frames_kept":[1-3]' "$CASE/receipts"/op6-cycle-*.json
WC=""; OCR=""

# 10. PASS with webcam denied (fail-soft: outcome unaffected, status recorded)
new_case webcam_denied; printf 'NONE\nNONE\n0x18d1:0xd001\n' > "$CASE/state/usb_seq"
WC=$FAKEBIN/webcam-denied
RC=$(run_tool --webcam)
check "webcam denied: exit 0" [ "$RC" = 0 ]
check "webcam denied: outcome PASS" [ "$(outcome)" = "PASS" ]
check "webcam denied: receipt records not-authorized" grep -q '"webcam_status":"not-authorized"' "$CASE/receipts"/op6-cycle-*.json
WC=""

# 12. PASS with webcam helper missing (fail-soft: unavailable recorded)
new_case webcam_missing; printf 'NONE\nNONE\n0x18d1:0xd001\n' > "$CASE/state/usb_seq"
WC=$FAKEBIN/no-such-helper
RC=$(run_tool --webcam)
check "webcam missing: exit 0" [ "$RC" = 0 ]
check "webcam missing: outcome PASS" [ "$(outcome)" = "PASS" ]
check "webcam missing: receipt records unavailable" grep -q '"webcam_status":"unavailable"' "$CASE/receipts"/op6-cycle-*.json
WC=""

# 13. ATTENDED-ONLY entry refuses without --attended
new_case attended_refuse; printf 'NONE\nNONE\n0x18d1:0xd001\n' > "$CASE/state/usb_seq"
printf '%s %s %s %s %s\n' "$SHA" "test.img" "120" "gate-evidence:RESULT.json#deadbeef" "ATTENDED-ONLY" > "$CASE/allowlist"
RC=$(run_tool)
check "attended refuse: exit 4" [ "$RC" = 4 ]
check "attended refuse: no boot attempted" [ "$(boots)" = 0 ]

# 14. ATTENDED-ONLY entry runs with --attended
new_case attended_ok; printf 'NONE\nNONE\n0x18d1:0xd001\n' > "$CASE/state/usb_seq"
printf '%s %s %s %s %s\n' "$SHA" "test.img" "120" "gate-evidence:RESULT.json#deadbeef" "ATTENDED-ONLY" > "$CASE/allowlist"
RC=$(run_tool --attended)
check "attended ok: exit 0" [ "$RC" = 0 ]
check "attended ok: outcome PASS" [ "$(outcome)" = "PASS" ]
check "attended ok: exactly one boot" [ "$(boots)" = 1 ]

# 11. No webcam flag: receipt carries no webcam fields
new_case webcam_off; printf 'NONE\nNONE\n0x18d1:0xd001\n' > "$CASE/state/usb_seq"
RC=$(run_tool)
check "webcam off: exit 0" [ "$RC" = 0 ]
check "webcam off: no webcam_status in receipt" [ -z "$(grep -l 'webcam_status' "$CASE/receipts"/op6-cycle-*.json 2>/dev/null)" ]

# 15. --attended --attended-by: receipt field + log wording operator-approved (A-2)
new_case attended_by; printf 'NONE\nNONE\n0x18d1:0xd001\n' > "$CASE/state/usb_seq"
printf '%s %s %s %s %s\n' "$SHA" "test.img" "120" "gate-evidence:RESULT.json#deadbeef" "ATTENDED-ONLY" > "$CASE/allowlist"
RC=$(run_tool --attended --attended-by "remote order 08:00 PT")
check "attended-by: exit 0" [ "$RC" = 0 ]
check "attended-by: outcome PASS" [ "$(outcome)" = "PASS" ]
check "attended-by: receipt attended_by" grep -q '"attended_by":"remote order 08:00 PT"' "$CASE/receipts"/op6-cycle-*.json
check "attended-by: log operator-approved with note" grep -q 'operator-approved: remote order 08:00 PT' "$CASE/tool.out"
check "attended-by: log has no 'operator at the phone'" [ -z "$(grep 'operator at the phone' "$CASE/tool.out")" ]

# 16. --attended without a note: attended_by empty string, plain operator-approved
new_case attended_nonote; printf 'NONE\nNONE\n0x18d1:0xd001\n' > "$CASE/state/usb_seq"
RC=$(run_tool --attended)
check "attended no-note: exit 0" [ "$RC" = 0 ]
check "attended no-note: receipt attended_by empty" grep -q '"attended_by":""' "$CASE/receipts"/op6-cycle-*.json
check "attended no-note: log operator-approved" grep -q 'operator-approved' "$CASE/tool.out"

# 17. Unattended run: attended_by is null
new_case unattended_fields; printf 'NONE\nNONE\n0x18d1:0xd001\n' > "$CASE/state/usb_seq"
RC=$(run_tool)
check "unattended: exit 0" [ "$RC" = 0 ]
check "unattended: receipt attended_by null" grep -q '"attended_by":null' "$CASE/receipts"/op6-cycle-*.json

# 18. --attended-by without --attended: usage error, no boot
new_case attended_by_alone; printf 'NONE\nNONE\n0x18d1:0xd001\n' > "$CASE/state/usb_seq"
RC=$(run_tool --attended-by "remote order")
check "attended-by alone: exit 64" [ "$RC" = 64 ]
check "attended-by alone: no boot attempted" [ "$(boots)" = 0 ]

# 19. CRASHDUMP + watcher RECOVERED during the wait: recovery object populated,
#     outcome stays CRASHDUMP (A-2). Fake log lines mirror the real formats:
#     "<ts> receipt <path> outcome=RESET_SENT" then "<ts> RECOVERED: pmOS back (tool rc=0)".
new_case crash_recovered; printf 'NONE\n0x05c6:0x900e\n' > "$CASE/state/usb_seq"
: > "$CASE/sahara/recovery.log"
printf '{}\n' > "$CASE/sahara/sahara-29990101T000000Z.json"
( sleep 2
  printf '%s receipt %s outcome=RESET_SENT\n' "2999-01-01T00:00:00Z" "$CASE/sahara/sahara-29990101T000000Z.json" >> "$CASE/sahara/recovery.log"
  printf '%s RECOVERED: pmOS back (tool rc=0)\n' "2999-01-01T00:00:01Z" >> "$CASE/sahara/recovery.log" ) &
WRITER=$!
REC_WAIT=6
RC=$(run_tool)
wait $WRITER 2>/dev/null
check "crash recovered: exit 2" [ "$RC" = 2 ]
check "crash recovered: outcome CRASHDUMP" [ "$(outcome)" = "CRASHDUMP" ]
check "crash recovered: watcher_recovered true" grep -q '"watcher_recovered":true' "$CASE/receipts"/op6-cycle-*.json
check "crash recovered: recovered_at recorded" grep -q '"recovered_at":"2999-01-01T00:00:01Z"' "$CASE/receipts"/op6-cycle-*.json
check "crash recovered: sahara receipt path" grep -q "\"sahara_receipt\":\"$CASE/sahara/sahara-29990101T000000Z.json\"" "$CASE/receipts"/op6-cycle-*.json
REC_WAIT=""

# 20. CRASHDUMP + no watcher recovery: watcher_recovered false, outcome stays CRASHDUMP
new_case crash_norecovery; printf 'NONE\n0x05c6:0x900e\n' > "$CASE/state/usb_seq"
: > "$CASE/sahara/recovery.log"
RC=$(run_tool)
check "crash no-recovery: exit 2" [ "$RC" = 2 ]
check "crash no-recovery: outcome CRASHDUMP" [ "$(outcome)" = "CRASHDUMP" ]
check "crash no-recovery: watcher_recovered false" grep -q '"watcher_recovered":false' "$CASE/receipts"/op6-cycle-*.json

# 21. FASTBOOT landing (item 3): boot proven (NONE poll arms the
#     detector), then d00d persists past TTL+grace -> FASTBOOT,
#     exit 5; closeout fastboot-reboots and pmOS returns, so the
#     receipt records recovered_to_pmos:true.
new_case fb_land 2
printf 'NONE\n0x18d1:0xd00d\n0x18d1:0xd00d\n0x18d1:0xd00d\n0x18d1:0xd00d\n0x18d1:0xd00d\n0x18d1:0xd00d\n' > "$CASE/state/usb_seq"
FB_GRACE=1; FB_PERSIST=2; FB_REC_WAIT=5; SLACK=6
RC=$(run_tool)
check "fastboot land: exit 5" [ "$RC" = 5 ]
check "fastboot land: outcome FASTBOOT" [ "$(outcome)" = "FASTBOOT" ]
check "fastboot land: recovered_to_pmos true" grep -q '"recovered_to_pmos":true' "$CASE/receipts"/op6-cycle-*.json
check "fastboot land: reboot issued in closeout" grep -q '^reboot' "$CASE/state/calls"
FB_GRACE=""; FB_PERSIST=""; FB_REC_WAIT=""; SLACK=""

# 22. Never-booted guard: d00d from the first poll, no non-d00d
#     sighting ever -> the detector never arms -> HUNG, not FASTBOOT.
new_case fb_never 2
printf '0x18d1:0xd00d\n0x18d1:0xd00d\n0x18d1:0xd00d\n0x18d1:0xd00d\n0x18d1:0xd00d\n0x18d1:0xd00d\n0x18d1:0xd00d\n0x18d1:0xd00d\n' > "$CASE/state/usb_seq"
FB_GRACE=1; FB_PERSIST=2; SLACK=4
RC=$(run_tool)
check "fastboot never-booted: exit 3" [ "$RC" = 3 ]
check "fastboot never-booted: outcome HUNG" [ "$(outcome)" = "HUNG" ]
FB_GRACE=""; FB_PERSIST=""; SLACK=""

# 23. Ledger: a FASTBOOT head with recovered_to_pmos:true is clean
new_case fb_ledger_ok; printf 'NONE\nNONE\n0x18d1:0xd001\n' > "$CASE/state/usb_seq"
printf '{"tool":"op6-cycle","outcome":"FASTBOOT","recovery":{"recovered_to_pmos":true}}\n' > "$CASE/receipts/op6-cycle-20000101T000000Z.json"
RC=$(run_tool)
check "fastboot ledger ok: exit 0" [ "$RC" = 0 ]
check "fastboot ledger ok: newest receipt PASS" grep -q '"outcome":"PASS"' "$(ls -t "$CASE/receipts"/op6-cycle-*.json | head -1)"

# 24. Ledger: a FASTBOOT head without recovery is NOT clean
new_case fb_ledger_bad; printf 'NONE\nNONE\n0x18d1:0xd001\n' > "$CASE/state/usb_seq"
printf '{"tool":"op6-cycle","outcome":"FASTBOOT","recovery":{"recovered_to_pmos":false}}\n' > "$CASE/receipts/op6-cycle-20000101T000000Z.json"
RC=$(run_tool)
check "fastboot ledger bad: exit 4" [ "$RC" = 4 ]
check "fastboot ledger bad: no boot attempted" [ "$(boots)" = 0 ]

say ""
say "op6-cycle tests: $PASS passed, $FAIL failed (workdir $WORK)"
[ "$FAIL" = 0 ]
