#!/bin/sh
# ONE quiet voice turn on the phone (PLAN A4: "run ONE quiet test with a
# pre-recorded WAV file as input (no live mic, volume low)").
#
# Run it from the homelab as:
#     ssh oneplus 'sh ~/agentic/bin/run-quiet-test.sh'
#
# It starts agentd, drives voiced, and reaps agentd on every exit path — the
# phone is never left with a daemon behind. No microphone is opened and no
# audio is played: voiced runs in quiet mode and the answer is proven by the
# bytes of the WAV it writes.
set -u
root=$HOME/agentic
in=${IN_WAV:-$root/fixtures/ask-request.wav}
journal=$root/out/a4-voiced-phone.trace
answer=$root/out/a4-answer.wav
apid=''

cleanup() {
    if [ -n "$apid" ]; then
        kill "$apid" 2>/dev/null
        i=0
        while kill -0 "$apid" 2>/dev/null && [ "$i" -lt 10 ]; do sleep 1; i=$((i + 1)); done
        kill -9 "$apid" 2>/dev/null
    fi
    echo "agentd pids after cleanup: [$(pgrep -x agentd | tr '\n' ' ')]"
}
trap cleanup EXIT INT TERM

[ -r "$in" ] || { echo "no input WAV at $in" >&2; exit 2; }
rm -f "$answer" "$journal"

"$root/bin/agentd" --config "$root/agentd.toml" --trace-out "$root/out/a4-agentd.trace" > "$root/out/a4-agentd.log" 2>&1 &
apid=$!
sleep 2
echo "agentd pid $apid health: $(curl -s -m 3 http://127.0.0.1:8770/health)"

timeout 240 "$root/bin/voiced" --wav "$in" --config "$root/voiced.conf" --journal "$journal" --out "$answer" --timeout 150
vc=$?
echo "[voiced exit $vc]"

echo "--- journal summary ---"
python3 - "$journal" <<'PY'
import json, sys
rows = [json.loads(l) for l in open(sys.argv[1]) if l.strip()]
print(f"{len(rows)} records; gapless seq: {all(rows[i+1]['seq'] == rows[i]['seq'] + 1 for i in range(len(rows)-1))}")
for r in rows:
    p = r['payload']
    t = p.get('type') or p.get('event')
    extra = p.get('reason') or p.get('text') or p.get('name') or ''
    print(f"  {r['seq']:>3} {p.get('kind'):>6} {t:<22} {str(extra)[:78]}")
PY

echo "--- the spoken answer ---"
python3 - "$answer" <<'PY'
import os, sys, wave
p = sys.argv[1]
if not os.path.exists(p):
    print("NO ANSWER WAV — the turn did not reach TTS")
    sys.exit(1)
w = wave.open(p)
print(f"{w.getnchannels()} ch, {w.getframerate()} Hz, {round(w.getnframes() / w.getframerate(), 2)} s, {os.path.getsize(p)} bytes")
PY
exit $vc
