#!/bin/sh
# Smart Turn judge adapter (CV2) — the contract voiced's `smart_turn`
# and the wakeword listener both speak:
#
#   stdin: the utterance SO FAR as a 16 kHz mono WAV
#   stdout: "complete <prob>" | "incomplete <prob>"   (exit 0)
#   exit 1: engine unavailable or too slow — the caller falls back to
#           the CV0 fixed-silence endpointer and journals the fallback.
#
# The engine is the persistent voiced-smartturn service (Pipecat
# Smart Turn v3, loaded once, managed by wakeword.sh start/stop with
# the listener). This wrapper only files a job into the engine's
# inbox and waits for the verdict — no model loads here.
set -u
root=$HOME/agentic
dir=$root/smartturn
mkdir -p "$dir/inbox" "$dir/outbox"
job="turn-$$-$(date +%s%N)"
tmp="$dir/inbox/.$job.wav.tmp"
cat > "$tmp" || exit 1
mv "$tmp" "$dir/inbox/$job.wav" || exit 1

# The engine answers a warm prediction in well under a second (see
# CV2-RESULTS.md for the measured cost). 2.5 s is a failure bound: a
# slower verdict is worth less than the fallback it would delay.
i=0
while [ $i -lt 25 ]; do
    if [ -f "$dir/outbox/$job.txt" ]; then
        out=$(cat "$dir/outbox/$job.txt")
        rm -f "$dir/outbox/$job.txt"
        case "$out" in
            complete\ *|incomplete\ *)
                printf '%s\n' "$out" | cut -d' ' -f1-2
                exit 0
                ;;
            *)
                echo "smartturn-judge: engine answered: $out" >&2
                exit 1
                ;;
        esac
    fi
    sleep 0.1
    i=$((i + 1))
done
echo "smartturn-judge: engine did not answer in 2.5 s (is voiced-smartturn running? see wakeword.sh status)" >&2
exit 1
