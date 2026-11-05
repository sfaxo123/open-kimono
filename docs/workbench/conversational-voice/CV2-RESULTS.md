# CV2 RESULTS — Smart Turn endpointing + ASR-confidence repair strata

Measured 2026-11-03/04 (PT) on the OnePlus 6, wav-driven proofs
(the phone's speaker/mic path was not used; audio was fed as WAV
files and paced file streams, and every claim below names its
trace). Built in a scratch clone off main `c91d903`; nothing here
is committed — adoption is the root session's call.

## Part A — Smart Turn v3 endpointing

**What runs.** A third warm service in the CV0 pattern:
`voiced-smartturn.service` runs `smartturn_engine.py` inside the
voice image (`--network none`), holding Pipecat Smart Turn v3
loaded and answering file-contract jobs from
`~/agentic/smartturn/{inbox,outbox}`. Two clients:
`smartturn-judge.sh` (for voiced, stdin WAV → verdict line) and
the wakeword listener's own `SmartTurn` class (for CV1 follow-up
captures). Both capture paths consult the model at each VAD
pause; the wake-word one-breath path is untouched.

**Model artifact (deviation from the brief, with evidence).** The
brief's verified artifact was the int8 build
(`smart-turn-v3.2-cpu.onnx`, sha256 `2bb026…`). On this phone's
aarch64 onnxruntime it is numerically unreliable: identical
features scored 0.092 (x86 ORT 1.31.1), 0.155 (x86 ORT 1.26.0),
0.559 (arm ORT 1.26.0), and a reference 0.64 pause fixture came
back 0.96 on arm. The FP32 build of the same weights
(`smart-turn-v3.2-gpu.onnx`, 32,411,198 B, sha256 `ab8dc64b…`,
same HF repo `pipecat-ai/smart-turn-v3` @ `f766f81d`, BSD-2)
reproduces the reference exactly on-device (0.6410 vs 0.6410) at
282–357 ms per prediction, so **FP32 is deployed**, with
onnxruntime pinned to 1.31.1. Feature extraction is replicated
from upstream `inference.py` in NumPy and validated against
`WhisperFeatureExtractor` (max feature diff 1.0e-5; phone features
bit-identical to the 3090's). Provenance and the int8 warning are
recorded in `docs/PROVENANCE.md` (CV2 section).

**Endpoint policy (v3, "first-verdict lock").** The model's raw
probabilities forced the policy; the measurements are the design:

- Across one real 1.5 s mid-sentence pause, the model's
  probability climbs with silence length alone: 0.32 → 0.86 →
  0.99. Any rule that lets a later verdict of the same pause end
  the capture eventually cuts the pause (proven live: an early
  policy cut at 1.3 s trailing on a 0.99 re-read).
- Its early readings are jagged: the same fixture scored 0.32 at
  0.3 s of trailing silence and 0.64 at 0.2 s.

So: **only a CONFIDENT (≥ 0.9) FIRST verdict of a pause may end
it.** Any other first verdict — incomplete or middling — locks
the pause; it ends when speech resumes or at the 2.5 s trailing
cap. Observed finished requests score 0.95–0.99 on their first
verdict, so real completions still end fast; a finished request
whose first verdict lands middling rides to the cap (bounded,
journaled dead air — the deliberate side of the asymmetry: a
false end mid-thought is the CV0 defect itself).

**Proofs (phone, wav-driven).**

1. **Pause survives** — feeder-paced `pause-mid` fixture (speech,
   1.5 s pause, "San Francisco"): first verdict `complete
   (p=0.64)` → journaled "pause locked"; re-reads at 0.81/0.98
   held by the lock; one turn, stt_final **"Open maps and give me
   directions to tour. San francisco"**, full destination served.
   (Trace: phone `/tmp/cv2-a1.trace`, run cv2-a1v3b.)
2. **End of turn** — finished request: `mic_stopped` **0.52 s**
   after the last speech frame (RMS > 0.01; 0.62 s vs a 0.02
   threshold — the difference is the final word's decay tail).
   Report as ≈0.55 ± 0.07 s, inside the 0.6 s bar.
3. **Regressions** — wake fixtures: pos fires; neg-nowake,
   neg-nearmiss, neg-oldword do not. Full listener run through the
   arecord shim: wake match → judged capture survives the pause
   in the real listener path (full transcript, session
   `sess-1793758883-1`) → follow-up window opens → "How long does
   it take to get there?" fires inside the window with the same
   session, answered with San Francisco context.
4. **Model killed** — with `voiced-smartturn` stopped, the
   capture journaled `smart turn unavailable …; cv0 endpointing
   fallback` (Handoff/Failed) and ended with the exact CV0 stop
   reason; the turn completed end to end. Capture never depends
   on the model.

Prediction cost on-device: **230–360 ms** per consult (FP32),
paid inside trailing silence that has already elapsed; consults
at 0.2 s trailing, then every 0.5 s.

**Scope note.** The listener's wake segmentation `Vad` is
unchanged: Smart Turn judges capture endpoints in the two
live-capture paths the brief names (phrase-only capture, CV1
follow-up captures). Wake-utterance endpointing was left alone
deliberately — the brief also requires the one-breath path
unchanged, and wake segmentation feeds it.

## Part B — ASR-confidence repair strata (agentd)

**The signal, as found.** Nothing reached agentd before CV2:
`POST /request` carried `{text, confirm, session, follow_up}`
only. The confidence in voiced's traces was voiced-local — and
it is an **acoustic-level proxy**, not a decoder score:
whole-utterance RMS mapped to 0.8 (clearly audible) or 0.4
(quiet). Moonshine's driver contract exposes no confidence at
all; the Wyoming path reports none; the stub reports 1.0. CV2
plumbs the real signal through honestly named: voiced returns
its computed confidence from `stt::transcribe` and posts it as
`asr_confidence` when one exists (omitted otherwise, so the old
wire shape is byte-identical for signal-less turns).

**Strata** (config `repair_high` = 0.7, `repair_low` = 0.55 —
the live proxy's 0.8 lands HIGH, its 0.4 lands LOW, and the band
between waits for a finer signal). Every classified turn is
journaled as `repair_stratum` (stratum, confidence, reason,
span); a consumed repair is journaled as `repair_resumed`
(original, heard span, answer, composed — verbatim).

- **HIGH** — act exactly as before CV2. No signal also lands
  here, with the reason saying so.
- **MEDIUM** — proceed; a user-role background note (data, same
  shape as CV1's entity summary) tells the brain to naturally
  restate the heard span in its answer. System prompt
  byte-identical — test-asserted against `Agent::system_prompt()`.
- **LOW** — no brain call, no tool, no broker. Exactly ONE
  targeted question about the extracted span (destination /
  app / whole-request fallback), stored as session repair state
  beside the pending list. The next turn in the session composes
  the original request with the span replaced by the spoken
  answer and serves that. The answer is accepted even at LOW
  confidence — a repair never asks twice.

**Composition with BRAINFIX** is written as an addendum to
`docs/workbench/user-simulations/key-tests/BRAINFIX-INSTRUCTION.md`:
confidence governs how a misheard word is handled, provenance
still governs whether an inferred value may act; a LOW turn
cannot weaken a must_confirm case (it cannot act at all), a
MEDIUM restatement is not a confirmation, a repair answer is
not a confirmation token, and composition never launders an
inferred value into user-stated words. Regressions run
explicitly in `apps/agentd/tests/repair_run.rs`: gate fires
under MEDIUM, gate fires on a resumed turn, LOW never reaches
the broker or a tool, prompt byte-identity under MEDIUM.

**Proofs (phone, wav-driven, agentd = the CV2 successor build).**

- **Mumbled → one question.** Attenuated request (whole-buffer
  RMS 0.0078 → confidence 0.4): agentd answered with exactly
  *"I heard the destination as \"San Francisco\", but I'm not
  confident I got it right. Where did you want directions to?"*
  — no brain call, no launch.
- **Answer resumes.** Follow-up turn "San francisco": journal
  `repair_resumed` composed *"Hey phone, open maps and give me
  directions to San francisco."*; Maps launched; answer served.
- **Clear → no question.** Full-gain request (confidence 0.8):
  `repair_stratum` HIGH, straight to the maps answer, no added
  question.

One transcript note: the warm Moonshine engine twice returned
the quiet fixture's words with a spurious "Hey phone," prefix —
recognizer variance at very low gain, recorded here because it
is what the journal shows; the strata behaved correctly on the
signal they are defined over (the acoustic proxy and the
resulting transcript) in every run.

## Test counts

- voiced: **56** (18 lib, 7 hardening, 29 pipeline, 2 session
  wire) — includes 4 new black-box CV2 capture tests and the
  confidence-on-the-wire assertion.
- agentd: **118** (107 pre-existing + 11 new in
  `tests/repair_run.rs`: strata classification/extraction/
  composition units, LOW→question→resume end-to-end, HIGH and
  no-signal invariance, MEDIUM restate + prompt identity, and
  the BRAINFIX/broker regressions).
- kimono-ui: **32** (`node --test test/*.mjs`), unchanged.

## Phone state left

Listener and warm services stopped; Smart Turn service stopped
with them (it is best-effort in the listener lifecycle and never
blocks a start). `agentd-keytest` runs the CV2 successor build
(`agentd-main-keytest`, backup of the replaced binary at
`agentd-main-keytest.bak-20261103-precv2`; deployed `agentd-main`
untouched). Deployed configs untouched except the additive
`voiced-cv2.conf`; agentd's deployed TOML untouched (strata
thresholds are code defaults until a config chooses otherwise).
