#!/usr/bin/env python3
# wakeword.py - host-side "hey phone" wake-word listener (KEY TEST 1,
# design 2).
#
# Pure Python 3 stdlib. Captures 16 kHz mono audio (mic via arecord, or a
# WAV file with --file), segments speech with an energy VAD, sends each
# utterance to the persistent Moonshine engine (ww_engine.py, one container
# for the whole session) through the inbox/outbox directory contract, and
# matches /\bhey[\s,]+phone\b/i on the transcript. On a match it fires
# exactly one voiced turn (the KEY-TEST-1 run sheet Step 3 command shape),
# then goes back to listening. If the matched utterance itself contains a
# request after the wake phrase, the turn runs on that utterance's own
# audio (voiced --wav <heard wav>) instead of opening a fresh live
# capture - the one-breath fire (2026-11-03). A phrase-only utterance
# still gets the live 8 s capture window.
#
# CV1 (2026-11-03): a fired turn opens a conversation session. After
# the answer finishes, a ~10 s follow-up window opens: speech with a
# real transcript fires the next turn WITHOUT the wake word, and each
# follow-up answer re-opens the window. Turns carry a session id to
# agentd (voiced --session), whose session store gives the brain the
# conversation's earlier turns and entities. The window's open/closed
# transitions are reported to agentd (POST /follow-up) by THIS process
# - the one that owns the microphone - so the kimono page's "still
# listening" indicator is a report, not a guess. Known limit, stated
# not solved: any voice inside the window is treated as the session's
# speaker (no speaker ID exists; KEY TEST 2 code C6).
#
# CV2 (2026-11-03): Smart Turn endpointing in the follow-up window.
# When a follow-up utterance completes on the plain hangover, the
# warm Smart Turn sidecar (voiced-smartturn, smartturn_engine.py) is
# asked whether the thought is complete. "incomplete" keeps the
# window open and concatenates the continuation, so a mid-sentence
# pause no longer cuts a follow-up in half. The wake-word path's own
# segmentation is unchanged (one breath, one utterance). If the judge
# is down or slow, the window closes utterances exactly as CV1 did
# and logs the fallback loudly - capture never depends on the model.
#
# Modes:
#   wakeword.py                 mic mode (arecord), listen forever
#   wakeword.py --file X.wav    file mode: detect-only, prints RESULT: FIRE
#                               or RESULT: NO-FIRE (add --fire to really
#                               run the voiced turn on a match)
# Env:
#   WW_HOME       state dir      (default ~/agentic/wakeword)
#   WW_RMS_MIN    min VAD floor  (default 300)
#   WW_TIMEOUT    engine reply timeout seconds (default 30)
#   WW_ONCE       1 = exit after the first fired turn (mic mode)
import argparse
import json
import math
import os
import re
import struct
import subprocess
import sys
import time
import urllib.request
import wave
from io import BytesIO

SR = 16000
FRAME_MS = 100
FRAME_SAMPLES = SR * FRAME_MS // 1000
FRAME_BYTES = FRAME_SAMPLES * 2
WAKE_RE = re.compile(r"\bhey[\s,]+phone\b", re.IGNORECASE)

# CV1: the follow-up window length (operator decision 2026-11-03:
# about 10 seconds) and where agentd listens for window reports and
# voiced turns. Both overridable for tests.
FOLLOWUP_S = float(os.environ.get("WW_FOLLOWUP_S", "10"))
AGENTD_URL = os.environ.get("WW_AGENTD", "http://127.0.0.1:8770")

WW_HOME = os.environ.get("WW_HOME", os.path.expanduser("~/agentic/wakeword"))
INBOX = os.path.join(WW_HOME, "inbox")
OUTBOX = os.path.join(WW_HOME, "outbox")
HEARD = os.path.join(WW_HOME, "heard")


def now():
    return time.strftime("%H:%M:%S")


def log(msg):
    print("%s wakeword: %s" % (now(), msg), flush=True)


def rms_of(frame_bytes):
    n = len(frame_bytes) // 2
    if n == 0:
        return 0.0
    samples = struct.unpack("<%dh" % n, frame_bytes[: n * 2])
    return math.sqrt(sum(s * s for s in samples) / n)


def wav_bytes(pcm):
    buf = BytesIO()
    with wave.open(buf, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(SR)
        w.writeframes(pcm)
    return buf.getvalue()


class Vad:
    """Energy VAD: adaptive noise floor, pre-roll, hangover segmentation."""

    def __init__(self):
        self.floor = 150.0
        self.rms_min = float(os.environ.get("WW_RMS_MIN", "300"))
        self.pre = []            # last 3 frames before speech
        self.buf = None          # current utterance frames or None
        self.above = 0           # consecutive above-threshold frames (in speech)
        self.below = 0           # consecutive below-threshold frames (in speech)
        self.speech_frames = 0

    def threshold(self):
        return max(self.rms_min, self.floor * 3.0)

    def feed(self, frame):
        """Return utterance PCM bytes when an utterance completes, else None."""
        r = rms_of(frame)
        thr = self.threshold()
        if self.buf is None:
            if r <= thr:
                self.floor = self.floor * 0.95 + r * 0.05
            self.pre.append(frame)
            self.pre = self.pre[-3:]
            if r > thr:
                self.above += 1
                if self.above >= 2:
                    self.buf = list(self.pre)
                    self.speech_frames = self.above
                    self.below = 0
            else:
                self.above = 0
            return None
        # in an utterance
        self.buf.append(frame)
        if r > thr:
            self.speech_frames += 1
            self.below = 0
        else:
            self.below += 1
        dur = len(self.buf) * FRAME_MS / 1000.0
        if self.below >= 6 or dur >= 5.0:
            pcm = b"".join(self.buf)
            voiced = self.speech_frames * FRAME_MS / 1000.0
            self.buf = None
            self.above = 0
            self.below = 0
            self.speech_frames = 0
            self.pre = []
            if voiced >= 0.35:
                return pcm
        return None


class Engine:
    def __init__(self):
        self.n = 0

    def ready(self):
        return os.path.exists(os.path.join(OUTBOX, "READY"))

    def transcribe(self, pcm):
        self.n += 1
        base = "utt-%d-%d" % (int(time.time()), self.n)
        heard_path = os.path.join(HEARD, base + ".wav")
        with open(heard_path, "wb") as f:
            f.write(wav_bytes(pcm))
        tmp = os.path.join(INBOX, "." + base + ".wav.tmp")
        dst = os.path.join(INBOX, base + ".wav")
        with open(tmp, "wb") as f:
            f.write(wav_bytes(pcm))
        os.rename(tmp, dst)
        timeout = float(os.environ.get("WW_TIMEOUT", "30"))
        t0 = time.monotonic()
        txt = os.path.join(OUTBOX, base + ".txt")
        err = os.path.join(OUTBOX, base + ".err")
        while time.monotonic() - t0 < timeout:
            if os.path.exists(txt):
                with open(txt) as f:
                    text = f.read().strip()
                os.remove(txt)
                meta = ""
                mp = os.path.join(OUTBOX, base + ".meta")
                if os.path.exists(mp):
                    with open(mp) as f:
                        meta = f.read().strip()
                    os.remove(mp)
                return text, int((time.monotonic() - t0) * 1000), meta, heard_path
            if os.path.exists(err):
                with open(err) as f:
                    why = f.read().strip()
                os.remove(err)
                raise RuntimeError("engine error: " + why)
            time.sleep(0.05)
        raise TimeoutError("engine reply timeout after %.0fs" % timeout)


class SmartTurn:
    """Client for the warm Smart Turn sidecar (voiced-smartturn).

    judge(pcm) -> (verdict, prob) with verdict "complete"/"incomplete",
    or None when the judge is unavailable for any reason - the caller
    then falls back to the plain rule and logs it. Same inbox/outbox
    shape as the Moonshine Engine, its own state dir.
    """

    def __init__(self):
        self.dir = os.environ.get(
            "WW_SMARTTURN_DIR", os.path.expanduser("~/agentic/smartturn")
        )
        self.inbox = os.path.join(self.dir, "inbox")
        self.outbox = os.path.join(self.dir, "outbox")
        self.n = 0

    def ready(self):
        return os.path.exists(os.path.join(self.dir, "READY")) and not os.path.exists(
            os.path.join(self.dir, "FAILED")
        )

    def judge(self, pcm):
        if not self.ready():
            return None
        self.n += 1
        base = "turn-%d-%d" % (int(time.time()), self.n)
        tmp = os.path.join(self.inbox, "." + base + ".wav.tmp")
        dst = os.path.join(self.inbox, base + ".wav")
        try:
            with open(tmp, "wb") as f:
                f.write(wav_bytes(pcm))
            os.rename(tmp, dst)
        except OSError:
            return None
        txt = os.path.join(self.outbox, base + ".txt")
        t0 = time.monotonic()
        while time.monotonic() - t0 < 1.5:
            if os.path.exists(txt):
                try:
                    with open(txt) as f:
                        line = f.read().strip()
                    os.remove(txt)
                except OSError:
                    return None
                parts = line.split()
                if len(parts) >= 2 and parts[0] in ("complete", "incomplete"):
                    try:
                        return parts[0], float(parts[1])
                    except ValueError:
                        return None
                return None
            time.sleep(0.02)
        return None


def fire_command(wav_path=None, session=None, followup=False):
    home = os.path.expanduser("~")
    # CV2/CV0: prefer the newest warm-path config present (CV2 adds
    # the Smart Turn judge to the CV0 warm path); fall back to the
    # older configs unchanged.
    conf = os.path.join(home, "agentic", "voiced-cv2.conf")
    if not os.path.exists(conf):
        conf = os.path.join(home, "agentic", "voiced-cv0.conf")
    if not os.path.exists(conf):
        conf = os.path.join(home, "agentic", "voiced-tonight.conf")
    if not os.path.exists(conf):
        conf = os.path.join(home, "agentic", "voiced.conf")
    if wav_path:
        # One-breath fire: the request is already inside the matched
        # utterance, so the turn consumes that utterance's own audio
        # instead of re-capturing a live mic window that may be silent.
        source = "--wav %s" % wav_path
    else:
        source = (
            '--mic "arecord -q -f S16_LE -r 16000 -c 1 -t raw" --seconds 8'
        )
    extra = ""
    if session:
        extra += " --session %s" % session
    if followup:
        extra += " --follow-up"
    return (
        "%s/agentic/bin/voiced %s "
        "--config %s "
        "--journal %s/agentic/out/wakeword-fire.trace "
        "--out %s/agentic/out/wakeword-answer.wav --timeout 150%s"
        % (home, source, conf, home, home, extra)
    )


def do_fire(wav_path=None, session=None, followup=False):
    cmd = fire_command(wav_path, session=session, followup=followup)
    log("FIRE - running voiced turn: %s" % cmd)
    t0 = time.monotonic()
    try:
        rc = subprocess.run(cmd, shell=True, timeout=220).returncode
    except subprocess.TimeoutExpired:
        log("voiced turn TIMED OUT")
        return False
    log("voiced turn exit=%d in %.1fs" % (rc, time.monotonic() - t0))
    return rc == 0


def transcribe(engine, pcm):
    """One utterance through the engine: (text, heard_path) or None."""
    try:
        text, ms, meta, heard_path = engine.transcribe(pcm)
    except Exception as exc:
        log("transcribe failed: %s" % exc)
        return None
    log("heard (%d ms %s): %r" % (ms, meta, text))
    return text, heard_path


def handle_utterance(engine, pcm, fire):
    got = transcribe(engine, pcm)
    if got is None:
        return False
    text, heard_path = got
    if WAKE_RE.search(text or ""):
        log("WAKE WORD MATCH")
        if fire:
            rest = WAKE_RE.sub(" ", text or "")
            rest = re.sub(r"[^A-Za-z0-9 ]+", " ", rest).strip()
            if rest:
                log(
                    "one-breath fire: a request follows the wake phrase, "
                    "running the turn on the utterance itself (%s)" % heard_path
                )
                do_fire(wav_path=heard_path)
            else:
                log("wake phrase only: opening the live capture window")
                do_fire()
        return True
    return False


def resample_to_16k(pcm, src_sr):
    """Crude linear-interpolation resample of s16 mono PCM to 16 kHz.
    Test files only (Piper writes 22050 Hz); the mic path is natively 16k."""
    if src_sr == SR:
        return pcm
    n = len(pcm) // 2
    src = struct.unpack("<%dh" % n, pcm[: n * 2])
    out_n = int(n * SR / src_sr)
    out = []
    for i in range(out_n):
        pos = i * src_sr / SR
        j = int(pos)
        frac = pos - j
        a = src[min(j, n - 1)]
        b = src[min(j + 1, n - 1)]
        out.append(int(a + (b - a) * frac))
    return struct.pack("<%dh" % len(out), *out)


def post_window(session, state):
    """Report the follow-up window's state to agentd (POST /follow-up).

    The kimono page renders its "still listening" indicator from the
    journal records this creates, so the report is made by the process
    that owns the microphone - this listener - at the moment of the
    transition. The report is posted BEFORE the window's capture starts
    on open, and AFTER it stops on close. A failed report is logged
    loudly; it never stops the conversation.
    """
    body = json.dumps({"session": session, "state": state}).encode()
    req = urllib.request.Request(
        AGENTD_URL.rstrip("/") + "/follow-up",
        data=body,
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    try:
        with urllib.request.urlopen(req, timeout=3) as resp:
            resp.read()
        log("window state reported: %s (%s)" % (state, session))
    except Exception as exc:
        log("WARNING: window state report failed (%s, %s): %s" % (state, session, exc))


def capture_window_utterance(engine, seconds):
    """Listen for one follow-up utterance for up to `seconds`.

    Returns (text, heard_path) for the first utterance whose transcript
    is real speech, or None when the window runs out. An utterance that
    STARTS inside the window may finish past it (bounded grace), so a
    reply begun at second 9 is not cut in half. Utterances with an
    empty transcript do not count and do not extend the window.
    """
    deadline = time.monotonic() + seconds
    hard_stop = deadline + 4.0
    vad = Vad()
    judge = SmartTurn()
    pending = b""  # CV2: segments the judge called incomplete, awaiting continuation
    pending_since = None
    proc = subprocess.Popen(
        ["arecord", "-q", "-f", "S16_LE", "-r", str(SR), "-c", "1", "-t", "raw"],
        stdout=subprocess.PIPE,
    )

    def finish(pcm):
        got = transcribe(engine, pcm)
        if got is None:
            return None
        text, heard_path = got
        if re.search(r"[A-Za-z0-9]", text or ""):
            return text, heard_path
        log("follow-up window: utterance had no usable transcript; still listening")
        return None

    try:
        while time.monotonic() < deadline or ((vad.buf is not None or pending) and time.monotonic() < hard_stop):
            if pending and time.monotonic() - pending_since > 2.5:
                # The judge said "not done" but nobody continued within
                # 2.5 s: close on what we hold rather than hanging.
                log("SMART TURN: no continuation within 2.5 s; closing on the held segment")
                return finish(pending)
            fr = proc.stdout.read(FRAME_BYTES)
            if not fr or len(fr) < FRAME_BYTES:
                return finish(pending) if pending else None
            utt = vad.feed(fr)
            if utt is not None:
                candidate = pending + utt
                verdict = judge.judge(candidate)
                if verdict is not None and verdict[0] == "complete" and verdict[1] < 0.9 and not pending:
                    # A middling complete on a first segment is not
                    # enough to fire on (voiced's capture applies the
                    # same confident-bar rule): hold the segment and
                    # let the continuation - or the silence cap -
                    # settle it.
                    log(
                        "SMART TURN complete-but-unsure (p=%.2f): holding the segment "
                        "for confirmation before firing" % verdict[1]
                    )
                    pending = candidate
                    pending_since = time.monotonic()
                    continue
                if verdict is not None and verdict[0] == "incomplete":
                    log(
                        "SMART TURN incomplete (p=%.2f): follow-up thought continues; "
                        "holding the segment and still listening" % verdict[1]
                    )
                    if not pending:
                        pending_since = time.monotonic()
                    pending = candidate
                    continue
                if verdict is None:
                    log("SMART TURN FALLBACK: judge unavailable; closing the utterance on the plain hangover")
                pending = b""
                pending_since = None
                done = finish(candidate)
                if done is not None:
                    return done
        return finish(pending) if pending else None
    finally:
        if proc.poll() is None:
            proc.terminate()


def follow_up_loop(engine, session):
    """The follow-up window conversation (CV1) for one session.

    Opens the ~10 s window; a real utterance fires the next turn with
    the session id and the follow-up mark; each completed follow-up
    turn re-opens the window. A silent window ends the session. The
    window is reported closed while a fired turn runs - during a turn
    the follow-up microphone is NOT hot, and the indicator must not
    claim otherwise.
    """
    while True:
        post_window(session, "open")
        log("follow-up window OPEN (%.0fs, session %s) - speak without the wake word" % (FOLLOWUP_S, session))
        got = capture_window_utterance(engine, FOLLOWUP_S)
        post_window(session, "closed")
        if got is None:
            log("follow-up window closed - no speech; session %s over" % session)
            return
        text, heard_path = got
        log("FOLLOW-UP turn (session %s): %r" % (session, text))
        if not do_fire(wav_path=heard_path, session=session, followup=True):
            log("follow-up turn failed; session %s over" % session)
            return


def run_file(path, fire):
    with wave.open(path, "rb") as w:
        if (w.getnchannels(), w.getsampwidth()) != (1, 2):
            log("ERROR: %s must be mono s16 WAV" % path)
            return 2
        src_sr = w.getframerate()
        raw = w.readframes(w.getnframes())
    if src_sr != SR:
        log("resampling %d Hz -> %d Hz for the detector path" % (src_sr, SR))
        raw = resample_to_16k(raw, src_sr)
    vad = Vad()
    engine = Engine()
    if not engine.ready():
        log("ERROR: engine not READY - start it first (wakeword.sh engine-start)")
        return 2
    fired = False
    frames = [raw[i : i + FRAME_BYTES] for i in range(0, len(raw), FRAME_BYTES)]
    for fr in frames:
        if len(fr) < FRAME_BYTES:
            fr = fr + b"\x00" * (FRAME_BYTES - len(fr))
        utt = vad.feed(fr)
        if utt is not None:
            if handle_utterance(engine, utt, fire):
                fired = True
    # flush a trailing utterance
    if vad.buf is not None:
        utt = b"".join(vad.buf)
        vad.buf = None
        if handle_utterance(engine, utt, fire):
            fired = True
    print("RESULT: %s" % ("FIRE" if fired else "NO-FIRE"), flush=True)
    return 0


def run_mic():
    engine = Engine()
    if not engine.ready():
        log("ERROR: engine not READY - start it first (wakeword.sh engine-start)")
        return 2
    log("listening (mic) - say the wake word; Ctrl-C to stop")
    session_n = 0
    while True:
        vad = Vad()
        proc = subprocess.Popen(
            ["arecord", "-q", "-f", "S16_LE", "-r", str(SR), "-c", "1", "-t", "raw"],
            stdout=subprocess.PIPE,
        )
        fired = False
        fired_session = None
        try:
            while True:
                fr = proc.stdout.read(FRAME_BYTES)
                if not fr or len(fr) < FRAME_BYTES:
                    break
                utt = vad.feed(fr)
                if utt is not None:
                    got = transcribe(engine, utt)
                    if got is None:
                        continue
                    text, heard_path = got
                    if not WAKE_RE.search(text or ""):
                        continue
                    log("WAKE WORD MATCH")
                    fired = True
                    session_n += 1
                    session = "sess-%d-%d" % (int(time.time()), session_n)
                    rest = WAKE_RE.sub(" ", text or "")
                    rest = re.sub(r"[^A-Za-z0-9 ]+", " ", rest).strip()
                    if rest:
                        log(
                            "one-breath fire: a request follows the wake phrase, "
                            "running the turn on the utterance itself (%s)" % heard_path
                        )
                        ok = do_fire(wav_path=heard_path, session=session)
                    else:
                        log("wake phrase only: opening the live capture window")
                        ok = do_fire(session=session)
                    if ok:
                        fired_session = session
                    break
        except KeyboardInterrupt:
            proc.terminate()
            log("stopped by user")
            return 0
        finally:
            if proc.poll() is None:
                proc.terminate()
        if fired:
            if os.environ.get("WW_ONCE") == "1":
                log("WW_ONCE=1 - exiting after one fired turn")
                return 0
            if fired_session is not None:
                follow_up_loop(engine, fired_session)
            log("resuming listening")
            time.sleep(0.5)


def main():
    ap = argparse.ArgumentParser(description='"hey phone" wake-word listener (design 2)')
    ap.add_argument("--file", metavar="WAV", help="file mode (detect-only unless --fire)")
    ap.add_argument("--fire", action="store_true", help="in file mode, really run the voiced turn on a match")
    a = ap.parse_args()
    for d in (INBOX, OUTBOX, HEARD):
        os.makedirs(d, exist_ok=True)
    if a.file:
        return run_file(a.file, a.fire)
    return run_mic()


if __name__ == "__main__":
    sys.exit(main())
