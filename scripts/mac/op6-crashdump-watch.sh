#!/bin/bash
# One watcher pass (launchd runs it every 15 s): if the phone has sat in Qualcomm
# crash-dump mode (05c6:900e) for >= 20 s, send the Sahara reset
# (op6-sahara-reset.py: no memory reads, no flashing) and wait for pmOS (18d1:d001).
# Guards: disable file, op6-cycle lock (let a running cycle classify first), at most
# 3 reset attempts per rolling hour. Every attempt appends a line to the recovery log.
set -u
OP6=$HOME/op6-r1
OUT=$OP6/out/sahara-receipts
STATE=$OUT/.watch-first-seen
ATTEMPTS=$OUT/.watch-attempts
LOG=$OUT/recovery.log
DISABLE=$OP6/crashdump-watch.disabled
CYCLE_LOCK=$OP6/out/device-receipts/.op6-cycle.lock
PY=$OP6/venv/bin/python
TOOL=$(cd "$(dirname "$0")" && pwd)/op6-sahara-reset.py
mkdir -p "$OUT"
iso(){ date -u +%Y-%m-%dT%H:%M:%SZ; }
usb(){ system_profiler SPUSBDataType 2>/dev/null | awk '/Product ID/{p=$3} /Vendor ID/{v=$3; if (v ~ /0x18d1|0x05c6/) print v":"p}' | head -1; }

[ -e "$DISABLE" ] && exit 0
now_state=$(usb)
if [ "$now_state" != "0x05c6:0x900e" ]; then rm -f "$STATE"; exit 0; fi

now=$(date +%s)
[ -f "$STATE" ] || { echo "$now" > "$STATE"; exit 0; }
first=$(cat "$STATE")
[ $((now - first)) -ge 20 ] || exit 0
[ -d "$CYCLE_LOCK" ] && exit 0

touch "$ATTEMPTS"
recent=$(awk -v t=$((now - 3600)) '$1 >= t' "$ATTEMPTS" | wc -l | tr -d ' ')
if [ "$recent" -ge 3 ]; then
  [ -f "$OUT/.watch-capped" ] || { echo "$(iso) CAPPED: 3 attempts in the last hour; operator needed" >> "$LOG"; touch "$OUT/.watch-capped"; }
  exit 0
fi
rm -f "$OUT/.watch-capped"
echo "$now" >> "$ATTEMPTS"

echo "$(iso) crash-dump for $((now - first)) s; sending Sahara reset" >> "$LOG"
"$PY" "$TOOL" >> "$LOG" 2>&1
rc=$?
rm -f "$STATE"
for _ in $(seq 1 60); do            # up to 5 min for pmOS
  s=$(usb)
  if [ "$s" = "0x18d1:0xd001" ]; then echo "$(iso) RECOVERED: pmOS back (tool rc=$rc)" >> "$LOG"; exit 0; fi
  sleep 5
done
echo "$(iso) NOT RECOVERED after 5 min (tool rc=$rc, USB now: $(usb))" >> "$LOG"
exit 1
