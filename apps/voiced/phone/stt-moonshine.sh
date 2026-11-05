#!/bin/sh
# voiced STT adapter for the OnePlus 6 (PLAN A4, decision L2-D1/L2-D2).
#
#   stdin: the captured audio as a 16-bit WAV   stdout: the transcript, one line
#   exit: 0 ok | 3 silence (empty transcript) | 1 engine failure
#
# Moonshine lives in the phone's voice image, so the engine runs in a
# throwaway `--rm` container: no microphone, no audio devices, no ports, and
# the existing `voice` container is never touched. The text never goes through
# a second level of shell interpolation — the container reads it from a file.
set -u
root=$HOME/agentic
in=$root/tmp/stt-in.wav
mkdir -p "$root/tmp" "$root/out"
cat > "$in" || exit 1

out=$(timeout 180 sudo -n podman run --rm --network none --userns=keep-id \
    -e MOONSHINE_VOICE_CACHE=/opt/models/moonshine -e STT_ARCH=small \
    -v /opt/models:/opt/models:ro \
    -v "$root:/agentic" \
    -v $HOME/.local/bin/voice-listen-app/brainkit/scripts:/bk:ro \
    localhost/voice-snapshot:20261024 \
    sh -c '/opt/stt/venv/bin/python /bk/stt_driver.py --transcribe /agentic/tmp/stt-in.wav' 2>"$root/tmp/stt-err.txt")
code=$?

if [ "$code" -eq 3 ]; then
    exit 3
fi
if [ "$code" -ne 0 ]; then
    tail -3 "$root/tmp/stt-err.txt" >&2
    echo "stt-moonshine: engine exited $code" >&2
    exit 1
fi
if [ -z "$out" ]; then
    echo "stt-moonshine: empty transcript" >&2
    exit 3
fi
printf '%s\n' "$out"
