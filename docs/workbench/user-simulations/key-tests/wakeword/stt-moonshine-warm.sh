#!/bin/sh
# voiced STT adapter, CV0 warm path — same contract as stt-moonshine.sh:
#
#   stdin: the captured audio as a 16-bit WAV   stdout: the transcript, one line
#   exit: 0 ok | 3 silence (empty transcript) | 1 engine failure
#
# The engine is the persistent voiced-warm-stt service (Moonshine
# `small`, loaded once, managed by wakeword.sh start/stop with the
# listener). This wrapper only files a job into the engine's inbox
# and waits for the transcript — no container starts here.
set -u
root=$HOME/agentic
dir=$root/warm-stt
mkdir -p "$dir/inbox" "$dir/outbox" "$root/tmp"
job="stt-$$-$(date +%s%N)"
tmp="$dir/inbox/.$job.wav.tmp"
cat > "$tmp" || exit 1
mv "$tmp" "$dir/inbox/$job.wav" || exit 1

# The engine polls at 100 ms and a warm transcribe of a few seconds
# of audio takes ~3 s; 120 s is a failure bound, not a budget.
i=0
while [ $i -lt 1200 ]; do
    if [ -f "$dir/outbox/$job.txt" ]; then
        out=$(cat "$dir/outbox/$job.txt")
        rm -f "$dir/outbox/$job.txt" "$dir/outbox/$job.meta"
        if [ -z "$(printf '%s' "$out" | tr -d '[:space:]')" ]; then
            echo "stt-warm: empty transcript" >&2
            exit 3
        fi
        printf '%s\n' "$out"
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
echo "stt-warm: engine did not answer in 120 s (is voiced-warm-stt running? see wakeword.sh status)" >&2
exit 1
