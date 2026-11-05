#!/usr/bin/env bash
# Water-reminder demo with Meta Muse Spark as agentd's brain (host or phone).
# Needs: muse-relay on $RELAY (tools/muse-relay) and agentd on $AGENTD.
# Usage: scripts/hackathon/muse-brain-demo.sh [wait_seconds]
set -u
AGENTD=${AGENTD:-http://127.0.0.1:8793}
WAIT=${1:-250}
say() { printf '\n\033[1m> %s\033[0m\n' "$*"; }
ask() { # $1 text, $2 optional confirm label
  local body
  if [ -n "${2:-}" ]; then body=$(printf '{"text":"%s","confirm":"%s"}' "$1" "$2"); else body=$(printf '{"text":"%s"}' "$1"); fi
  curl -s -m 300 -X POST "$AGENTD/request" -H 'Content-Type: application/json' -d "$body"
}
show() { python3 -c 'import json,sys,os; d=json.load(sys.stdin); print("  brain:", os.environ.get("BRAIN_LABEL","meta/muse-spark-1.3"), "via slot", d.get("brain"), "| answer:", d.get("answer") or d.get("error"))'; }

say "remind me every 2 minutes to drink water"
R=$(ask "remind me every 2 minutes to drink water"); echo "$R" | show
# Commit-class: agentd refuses to install until the exact label is confirmed.
for i in 1 2 3; do
  L=$(echo "$R" | grep -o 'service_create svc\.[a-z0-9_.]*' | head -1)
  [ -z "$L" ] && break
  say "[user taps Confirm: $L]"
  R=$(ask "remind me every 2 minutes to drink water" "$L"); echo "$R" | show
done
say "(waiting ${WAIT}s for the reminder to fire)"; sleep "$WAIT"
say "list my services"; ask "list my services" | show
say "stop the water reminder"; ask "stop the water reminder" | show
say "list my services"; ask "list my services" | show
