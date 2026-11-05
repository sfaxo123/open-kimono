#!/bin/sh
# voiced TTS adapter for the OnePlus 6 (PLAN A4, decision L2-D1/L2-D2/L2-D3).
#
#   stdin: the text to speak      argv[1]: where to write the WAV
#   exit: 0 ok | 2 bad input | 1 engine failure
#
# ptts (Phonon) lives in the phone's voice image, so it runs in a throwaway
# `--rm` container: no audio devices, no ports, and the existing `voice`
# container is never started or exec'd. Nothing is played — voiced decides
# whether to play (quiet mode makes no sound at all).
set -u
root=$HOME/agentic
out=${1:-}
[ -n "$out" ] || { echo "tts-ptts: usage: tts-ptts.sh OUT.WAV < text" >&2; exit 2; }
mkdir -p "$root/tmp" "$root/out"
cat > "$root/tmp/tts-text.txt" || exit 1
[ -s "$root/tmp/tts-text.txt" ] || { echo "tts-ptts: empty text" >&2; exit 2; }

staged=$HOME/agentic/out/voiced-tts.wav
rm -f "$staged"
timeout 240 sudo -n podman run --rm --network none --userns=keep-id \
    -v /opt/models:/opt/models:ro \
    -v "$root:/agentic" \
    -v $HOME/.local/bin/voice-listen-app/brainkit/scripts:/bk:ro \
    localhost/voice-snapshot:20261024 \
    sh -c 't=$(tr "\n" " " < /agentic/tmp/tts-text.txt); /opt/tts/venv/bin/python /bk/tts_driver.py --synth /agentic/out/voiced-tts.wav "$t"' \
    > "$root/tmp/tts-stats.txt" 2>&1
code=$?
if [ "$code" -ne 0 ] || [ ! -s "$staged" ]; then
    tail -3 "$root/tmp/tts-stats.txt" >&2
    echo "tts-ptts: engine exited $code" >&2
    exit 1
fi
cp "$staged" "$out" || exit 1
cat "$root/tmp/tts-stats.txt" >&2
