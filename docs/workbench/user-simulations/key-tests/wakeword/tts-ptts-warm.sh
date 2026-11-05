#!/bin/sh
# voiced TTS adapter, CV0 warm path — same contract as tts-ptts.sh:
#
#   stdin: the text to speak      argv[1]: where to write the WAV
#   exit: 0 ok | 2 bad input | 1 engine failure
#
# The engine is the persistent voiced-warm-tts service (ptts/Phonon
# loaded once, managed by wakeword.sh start/stop with the listener).
# This wrapper only files a job into the engine's inbox and waits
# for the WAV — no container starts here. Nothing is played; voiced
# decides whether to play, exactly as with the cold adapter.
set -u
root=$HOME/agentic
out=${1:-}
[ -n "$out" ] || { echo "tts-warm: usage: tts-ptts-warm.sh OUT.WAV < text" >&2; exit 2; }
dir=$root/warm-tts
mkdir -p "$dir/inbox" "$dir/outbox" "$root/tmp"
text=$(cat)
[ -n "$(printf '%s' "$text" | tr -d '[:space:]')" ] || { echo "tts-warm: empty text" >&2; exit 2; }

job="tts-$$-$(date +%s%N)"
printf '%s' "$text" > "$dir/inbox/.$job.txt.tmp"
mv "$dir/inbox/.$job.txt.tmp" "$dir/inbox/$job.txt" || exit 1

i=0
while [ $i -lt 1200 ]; do
    if [ -f "$dir/outbox/$job.wav" ]; then
        cp "$dir/outbox/$job.wav" "$out" || exit 1
        [ -f "$dir/outbox/$job.meta" ] && cat "$dir/outbox/$job.meta" >&2
        rm -f "$dir/outbox/$job.wav" "$dir/outbox/$job.meta"
        exit 0
    fi
    if [ -f "$dir/outbox/$job.err" ]; then
        cat "$dir/outbox/$job.err" >&2
        rm -f "$dir/outbox/$job.err"
        exit 1
    fi
    sleep 0.1
    i=$((i + 1))
done
echo "tts-warm: engine did not answer in 120 s (is voiced-warm-tts running? see wakeword.sh status)" >&2
exit 1
