#!/usr/bin/env python3
# warm_tts_engine.py - container-side persistent TTS engine (CV0).
#
# Runs INSIDE the phone's voice image (localhost/voice-snapshot:20261024)
# under /opt/tts/venv/bin/python. Loads the ptts (Phonon) model ONCE
# (reusing the proven /bk/tts_driver.py loaders), then polls a
# bind-mounted inbox directory for text jobs, synthesizes each, and
# writes the WAV to the outbox. One container start serves every voice
# turn while the wake-word listener is up - this removes the per-turn
# throwaway-container cost that made a turn cost ~19 s.
#
# Contract (all under the mounted TW_DIR):
#   inbox/<name>.txt     host drops text jobs here
#   outbox/<name>.wav    synthesized speech for that job
#   outbox/<name>.meta   one SYNTH-STATS json line for that job
#   outbox/<name>.err    engine error for that job instead of .wav
#   outbox/READY         written once after model load
#   STOP                 host creates this file -> engine exits cleanly
import json
import os
import sys
import time
import wave

sys.path.insert(0, "/bk")

TW = os.environ.get("TW_DIR", "/tw")
INBOX = os.path.join(TW, "inbox")
OUTBOX = os.path.join(TW, "outbox")


def log(msg):
    sys.stderr.write("warm_tts: %s\n" % msg)
    sys.stderr.flush()


def synth_wav(tts, voice, sr, text, out_path):
    """Same synthesis as tts_driver.mode_synth, minus the per-call
    model load: stream the chunks, write one mono s16 WAV, and
    refuse silence (peak 0) the way the one-shot driver does."""
    import numpy as np

    t0 = time.monotonic()
    ttfa = None
    frames = []
    with tts.stream(text, voice=voice) as audio:
        for chunk in audio:
            if ttfa is None:
                ttfa = time.monotonic() - t0
            frames.append(chunk)
    wall = time.monotonic() - t0
    pcm = np.concatenate(frames) if frames else np.zeros(0, dtype="float32")
    i16 = np.clip(pcm * 32767.0, -32768, 32767).astype("<i2")
    peak = int(np.abs(i16).max()) if i16.size else 0
    if peak == 0:
        raise RuntimeError("synthesized wav is silent (peak=0)")
    tmp = out_path + ".tmp"
    with wave.open(tmp, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(sr)
        w.writeframes(i16.tobytes())
    os.rename(tmp, out_path)
    dur = i16.size / float(sr) if sr else 0.0
    return {
        "wall_s": round(wall, 3),
        "ttfa_s": round(ttfa, 3) if ttfa is not None else None,
        "dur_s": round(dur, 3),
        "rtf": round(wall / dur, 3) if dur > 0 else None,
        "peak": peak,
        "sr": sr,
        "voice": voice,
    }


def main():
    os.makedirs(INBOX, exist_ok=True)
    os.makedirs(OUTBOX, exist_ok=True)
    from tts_driver import check_model_dir, load_tts, pick_voice

    check_model_dir()
    tts, load_s = load_tts()
    voice = pick_voice(tts)
    sr = tts.sample_rate
    with open(os.path.join(OUTBOX, "READY"), "w") as f:
        f.write("LOAD_S=%.2f SR=%d VOICE=%s\n" % (load_s, sr, voice))
    log("ready (LOAD_S=%.2f, SR=%d, VOICE=%s)" % (load_s, sr, voice))
    while True:
        if os.path.exists(os.path.join(TW, "STOP")):
            log("STOP file seen - exiting")
            return 0
        try:
            jobs = sorted(n for n in os.listdir(INBOX) if n.endswith(".txt"))
        except OSError:
            jobs = []
        if not jobs:
            time.sleep(0.1)
            continue
        for name in jobs:
            src = os.path.join(INBOX, name)
            base = name[:-4]
            dst_wav = os.path.join(OUTBOX, base + ".wav")
            dst_err = os.path.join(OUTBOX, base + ".err")
            # A job file whose size is still changing is not ready.
            try:
                s1 = os.path.getsize(src)
                time.sleep(0.05)
                if os.path.getsize(src) != s1:
                    continue
            except OSError:
                continue
            try:
                with open(src) as f:
                    text = " ".join(f.read().split())
                if not text:
                    raise ValueError("empty text")
                t0 = time.monotonic()
                stats = synth_wav(tts, voice, sr, text, dst_wav)
                stats["engine_wall_s"] = round(time.monotonic() - t0, 3)
                with open(os.path.join(OUTBOX, base + ".meta"), "w") as f:
                    f.write("SYNTH-STATS " + json.dumps(stats) + "\n")
                log("job %s -> wav (%s)" % (name, stats))
            except SystemExit as exc:
                # tts_driver helpers exit() on model problems - per-job .err
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


if __name__ == "__main__":
    sys.exit(main())
