#!/usr/bin/env python3
"""smartturn_engine.py - Pipecat Smart Turn v3 as a warm sidecar (Phase CV2).

The third warm voice service, shaped exactly like ww_engine.py /
tts_engine.py: a persistent process draining an inbox/outbox directory
contract, so a prediction costs a file drop, not a process spawn.

  state dir (env WW_SMARTTURN_DIR, default ~/agentic/smartturn):
    inbox/<id>.wav     16 kHz mono S16_LE WAV: the utterance SO FAR
    outbox/<id>.txt    one line: "complete <prob> <ms>" |
                                 "incomplete <prob> <ms>" |
                                 "error <message>"
    READY              written once the model is loaded (first line:
                       the model sha256 it loaded)
    FAILED             written instead of READY when the model cannot
                       load; the process then exits nonzero.
    STOP               touch to stop the loop.

Model: smart-turn-v3.2-gpu.onnx - the FP32 build of Pipecat Smart
Turn v3 (BSD-2-Clause; provenance + sha256 in docs/PROVENANCE.md),
fetched by smartturn-setup.sh into <state dir>/model/. The engine
NEVER downloads. The FP32 build is deployed instead of the int8
"-cpu" build on purpose: the int8 model's borderline probabilities
proved runtime-dependent on this phone's aarch64 onnxruntime (a cut
the reference pipeline scores 0.64 came back 0.96), while the FP32
build reproduces the reference probabilities to 4 decimals on-device
at ~300 ms per prediction (CV2-RESULTS.md).

Preprocessing replicates upstream inference.py exactly: last 8 s of
audio (zero-padded at the BEGINNING when shorter), Whisper log-mel
features (80 mels, n_fft 400, hop 160, slaney bank, log10 clamped to
max-8, (x+4)/4) over the zero-mean/unit-variance-normalized waveform.
The NumPy implementation below was validated against transformers'
WhisperFeatureExtractor: max feature diff 1.1e-5, identical model
probability to 6 decimals (see CV2-RESULTS.md).

FAIL-OPERATIONAL is the clients' contract, not this file's: if this
engine is down, slow, or wrong, both capture paths fall back to the
CV0 fixed-silence endpointer and journal the fallback. This engine
only ever ADVISES; it never gates capture.
"""
import glob
import hashlib
import os
import sys
import time
import wave

import numpy as np

SR, N_FFT, HOP, N_MELS, MAXLEN = 16000, 400, 160, 80, 128000
THRESHOLD = 0.5

BASE = os.environ.get("WW_SMARTTURN_DIR") or os.path.expanduser("~/agentic/smartturn")
INBOX = os.path.join(BASE, "inbox")
OUTBOX = os.path.join(BASE, "outbox")
MODEL = os.environ.get("SMART_TURN_MODEL") or os.path.join(BASE, "model", "smart-turn-v3.2-gpu.onnx")


# --- Whisper log-mel features (validated vs transformers, see header) ---

def _hz2mel(f):
    f = np.asarray(f, dtype=np.float64)
    return np.where(f < 1000.0, f * 3.0 / 200.0,
                    15.0 + np.log(np.maximum(f, 1e-12) / 1000.0) * (27.0 / np.log(6.4)))


def _mel2hz(m):
    m = np.asarray(m, dtype=np.float64)
    return np.where(m < 15.0, m * 200.0 / 3.0, 1000.0 * (6.4 ** ((m - 15.0) / 27.0)))


def _mel_filters():
    mel_pts = np.linspace(_hz2mel(0.0), _hz2mel(8000.0), N_MELS + 2)
    ff = _mel2hz(mel_pts)
    fft_freqs = np.linspace(0, SR // 2, N_FFT // 2 + 1)
    diff = np.diff(ff)
    slopes = ff[None, :] - fft_freqs[:, None]
    fb = np.maximum(0.0, np.minimum(-slopes[:, :-2] / diff[:-1], slopes[:, 2:] / diff[1:]))
    enorm = 2.0 / (ff[2:N_MELS + 2] - ff[:N_MELS])
    return fb * enorm[None, :]


_MEL = _mel_filters()
_WIN = np.hanning(N_FFT + 1)[:-1]  # periodic Hann


def whisper_features(audio):
    """audio: float32 mono 16 kHz, any length. Returns (1, 80, 800) float32."""
    if len(audio) > MAXLEN:
        audio = audio[-MAXLEN:]
    elif len(audio) < MAXLEN:
        audio = np.pad(audio, (MAXLEN - len(audio), 0))
    x = audio.astype(np.float64)
    x = (x - x.mean()) / np.sqrt(x.var() + 1e-7)
    x = np.pad(x, (N_FFT // 2, N_FFT // 2), mode="reflect")
    n_frames = 1 + int((len(x) - N_FFT) // HOP)
    idx = np.arange(N_FFT)[None, :] + HOP * np.arange(n_frames)[:, None]
    frames = x[idx] * _WIN[None, :]
    power = np.abs(np.fft.rfft(frames, n=N_FFT, axis=1)) ** 2.0
    mel = np.maximum(1e-10, _MEL.T @ power.T)
    log = np.log10(mel)[:, :-1]
    log = np.maximum(log, log.max() - 8.0)
    log = (log + 4.0) / 4.0
    return log.astype(np.float32)[None, :, :]


def load_wav_float(path):
    with wave.open(path, "rb") as w:
        sr, n, ch, sw = w.getframerate(), w.getnframes(), w.getnchannels(), w.getsampwidth()
        raw = w.readframes(n)
    if ch != 1 or sw != 2:
        raise ValueError("want mono S16_LE wav, got ch=%d sw=%d" % (ch, sw))
    if sr != SR:
        raise ValueError("want 16000 Hz wav, got %d" % sr)
    return np.frombuffer(raw, dtype=np.int16).astype(np.float32) / 32768.0


def log(msg):
    sys.stdout.write("smartturn: %s\n" % msg)
    sys.stdout.flush()


def main():
    os.makedirs(INBOX, exist_ok=True)
    os.makedirs(OUTBOX, exist_ok=True)
    try:
        import onnxruntime as ort
        so = ort.SessionOptions()
        so.execution_mode = ort.ExecutionMode.ORT_SEQUENTIAL
        so.inter_op_num_threads = 1
        so.graph_optimization_level = ort.GraphOptimizationLevel.ORT_ENABLE_ALL
        if not os.path.isfile(MODEL):
            raise FileNotFoundError("model missing: %s" % MODEL)
        sess = ort.InferenceSession(MODEL, sess_options=so, providers=["CPUExecutionProvider"])
        with open(MODEL, "rb") as f:
            sha = hashlib.sha256(f.read()).hexdigest()
    except Exception as e:
        with open(os.path.join(BASE, "FAILED"), "w") as f:
            f.write("model load failed: %r\n" % (e,))
        log("FAILED to load model: %r" % (e,))
        return 1
    with open(os.path.join(BASE, "READY"), "w") as f:
        f.write("%s\n" % sha)
    log("ready (model sha256 %s)" % sha[:12])

    stop = os.path.join(BASE, "STOP")
    while not os.path.exists(stop):
        jobs = sorted(glob.glob(os.path.join(INBOX, "*.wav")))
        if not jobs:
            time.sleep(0.02)
            continue
        for job in jobs:
            jid = os.path.basename(job)[:-4]
            out = os.path.join(OUTBOX, jid + ".txt")
            t0 = time.time()
            try:
                out_arr = sess.run(None, {"input_features": whisper_features(load_wav_float(job))})
                prob = float(out_arr[0].ravel()[0])
                ms = int((time.time() - t0) * 1000)
                verdict = "complete" if prob > THRESHOLD else "incomplete"
                line = "%s %.4f %d" % (verdict, prob, ms)
            except Exception as e:
                line = "error %r" % (e,)
            tmp = out + ".tmp"
            with open(tmp, "w") as f:
                f.write(line + "\n")
            os.replace(tmp, out)
            try:
                os.remove(job)
            except OSError:
                pass
            log("job %s -> %s" % (jid, line))
    log("stopped (STOP file)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
