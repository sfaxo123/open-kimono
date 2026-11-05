#!/usr/bin/env python3
# ww_engine.py - container-side wake-word transcription engine (KEY TEST 1).
#
# Runs INSIDE the phone's voice image (localhost/voice-snapshot:20261024)
# under /opt/stt/venv/bin/python. Loads the Moonshine transcriber ONCE
# (reusing the proven /bk/stt_driver.py loaders), then polls a bind-mounted
# inbox directory for utterance WAVs, transcribes each, and writes the
# transcript to the outbox. One container start serves the whole listening
# session - this is what makes wake-word detection affordable on-device.
#
# Contract (all under the mounted /ww):
#   inbox/<name>.wav     host drops utterance WAVs here
#   outbox/<name>.txt    transcript (exactly the text, one line)
#   outbox/<name>.err    engine error for that job instead of .txt
#   outbox/READY         written once after model load; content: LOAD_MS=<n>
#   STOP                 host creates this file -> engine exits cleanly
import os
import sys
import time

sys.path.insert(0, "/bk")

WW = os.environ.get("WW_DIR", "/ww")
INBOX = os.path.join(WW, "inbox")
OUTBOX = os.path.join(WW, "outbox")


def log(msg):
    sys.stderr.write("ww_engine: %s\n" % msg)
    sys.stderr.flush()


def main():
    os.makedirs(INBOX, exist_ok=True)
    os.makedirs(OUTBOX, exist_ok=True)
    from stt_driver import load_transcriber, load_audio, text_of
    tr, load_ms = load_transcriber()
    with open(os.path.join(OUTBOX, "READY"), "w") as f:
        f.write("LOAD_MS=%d\n" % load_ms)
    log("ready (LOAD_MS=%d, STT_ARCH=%s)" % (load_ms, os.environ.get("STT_ARCH", "tiny")))
    try:
        while True:
            if os.path.exists(os.path.join(WW, "STOP")):
                log("STOP file seen - exiting")
                return 0
            try:
                jobs = sorted(n for n in os.listdir(INBOX) if n.endswith(".wav"))
            except OSError:
                jobs = []
            if not jobs:
                time.sleep(0.1)
                continue
            for name in jobs:
                src = os.path.join(INBOX, name)
                base = name[:-4]
                dst_txt = os.path.join(OUTBOX, base + ".txt")
                dst_err = os.path.join(OUTBOX, base + ".err")
                # A job file whose size is still changing is not ready.
                try:
                    s1 = os.path.getsize(src)
                    time.sleep(0.05)
                    if os.path.getsize(src) != s1:
                        continue
                except OSError:
                    continue
                t0 = time.monotonic()
                try:
                    audio, sr = load_audio(src)
                    tx = tr.transcribe_without_streaming(audio, sr)
                    text = text_of(tx)
                    ms = int((time.monotonic() - t0) * 1000)
                    with open(dst_txt, "w") as f:
                        f.write(text + "\n")
                    with open(os.path.join(OUTBOX, base + ".meta"), "w") as f:
                        f.write("TRANSCRIBE_MS=%d SAMPLES=%d\n" % (ms, len(audio)))
                    log("job %s -> %r (%d ms)" % (name, text, ms))
                except SystemExit as exc:
                    # stt_driver helpers exit() on bad input - convert to .err
                    with open(dst_err, "w") as f:
                        f.write("driver exit %s\n" % exc.code)
                    log("job %s -> driver exit %s" % (name, exc.code))
                except Exception as exc:  # keep the engine alive per-job
                    with open(dst_err, "w") as f:
                        f.write("%s: %s\n" % (type(exc).__name__, exc))
                    log("job %s -> ERROR %s: %s" % (name, type(exc).__name__, exc))
                finally:
                    try:
                        os.remove(src)
                    except OSError:
                        pass
    finally:
        tr.close()


if __name__ == "__main__":
    sys.exit(main())
