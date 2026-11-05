# INTEGRATION — Conversational voice, folded into our plan

Companion to RESEARCH.md (same directory). Works-first ordering; every
step names its component, its user purpose (no-bloat law), effort
(S/M/L), and its proof. Nothing here is decided — §3 is the operator's.

## 1. Gap table

| Conversational quality | Where we are today | The gap, in one line |
|---|---|---|
| End-of-turn detection | Fixed 8 s capture window (phrase-only path); one-breath reuse otherwise | We pay up to 8 s of dead air where the field pays 0.3–0.5 s, and a mid-thought pause is indistinguishable from "done" |
| Response latency | ~19 s acoustic turn, fully serial, throwaway STT/TTS containers per turn | No overlap and cold processes: ~20× the ≤800 ms–2 s first-audio band the field targets |
| Streaming | None — batch STT, batch LLM, batch TTS | Every stage waits for the previous one to finish completely |
| Follow-up turns | None — every turn needs "hey phone" again | Conversation is impossible by construction; Key Test 1's "hold a conversation" passes only as discrete commands |
| Conversation state | agentd handles one request; no session entity carry-over | "And how long does it take?" has nothing to resolve against |
| Barge-in | None — mic isn't live while TTS plays; user must wait out any answer | A long or wrong answer cannot be stopped by voice |
| Backchannel / acknowledgment | None — silence from wake to answer | 19 s of silence reads as "broken," the exact failure the ack pattern exists to kill |
| Repair / clarification | BRAINFIX covers inferred-destination confirmation (provenance-based) | No ASR-confidence repair: a misheard word has no recovery path short of starting over |
| Expressive speech out | Piper/Phonon, flat narration voice | No prosody control exists in the engine; only text-side tricks are available |
| Full-duplex feel | Half-duplex walkie-talkie | True duplex requires a speech-to-speech model class our phone silicon cannot run (RESEARCH §8) |
| Proactive conversation | None — phone never speaks first | The operator's defining example (car Bluetooth → "where to?") needs the phone to open a turn, not just answer one |

## 2. Phased plan (works-first)

**CV0 OUTCOME (measured 2026-11-03, adopted):** all three mechanisms built and proven on the phone (warm services, instant ack, 0.7 s VAD endpointing). The stretch target (first answer audio at most half of baseline) was NOT met: warm full turns 17.1 s vs baselines 15.8/20.9 s. Measured decomposition of a warm turn: Moonshine-small transcribe ~3.5 s, gateway brain 2-8 s (run-variable), Phonon synth RTF ~0.45; process startup was only ~2-3 s of the old ~19 s. The Phase 0 premise (dead air is mostly our startup) is CORRECTED by these numbers: the remaining perceived latency lives in STT compute, brain wait, and synth — i.e. CV3 streaming/preemptive territory. The ack does land its purpose: first audio 0.6 s after the transcript instead of after the whole chain. Deployed ack_delay_ms is 1500 (root retune from the specced 600: the on-device ready path — health + POST + SSE via three curl spawns — has a measured ~1.0 s floor, so 600 ms fired the ack even on instant answers and delayed them). Pause-cut at 1.5 s mid-sentence pauses proven and logged, as predicted — CV2 owns that fix.

### Phase 0 — Kill the dead air we already own (no architecture change)
- **Warm processes**: replace the throwaway per-turn podman STT/TTS
  containers with persistent Moonshine and Piper services on the
  phone, started with the wakeword listener.
  Component: wakeword listener / voiced runtime.
  Purpose: the operator waits ~19 s for an answer today; most of that
  is our own process startup, not thinking. Effort: **S**.
  Proof: acoustic turn time (wake → first answer audio) measured
  before/after on the Key Test 1 sentence; target ≥2× faster.
- **Instant acknowledgment**: on turn start, if the brain/tools are
  still working after ~600 ms, play a pre-rendered short ack clip
  ("One sec—" / "Got it—"), capped at one per turn.
  Component: voiced. Purpose: silence reads as failure; the ack says
  "heard you, working on it." Effort: **S**.
  Proof: in Key Test 2 runs, no turn has >2 s of unacknowledged
  silence; operator confirms it doesn't annoy across 5 turns.
- **VAD endpointing on the phrase-only path**: end the bare-wake-word
  capture on ~0.7 s of trailing silence instead of the fixed 8 s.
  Component: voiced capture. Purpose: the 8 s window is pure dead air
  on the most common interaction shape. Effort: **S**.
  Proof: phrase-only turn completes; a deliberate mid-sentence 1.5 s
  pause is NOT cut off (that failure is Phase 2's to fix properly —
  record it, don't hide it).

### Phase 1 — Conversation session (the Key Test 2 core)
- **Follow-up window**: after each spoken answer, the listener keeps
  the mic hot for a short window (length = decision §3.1) for a
  follow-up with no wake word; kimono shows a "still listening"
  waveform state for the whole window; window expiry is announced by
  the waveform going quiet, never by a spoken prompt.
  Component: wakeword listener + kimono. Purpose: this is the
  difference between a command line and a conversation — and the
  car-Bluetooth example is a follow-up-shaped interaction.
  Effort: **M**. Proof: Key Test 2 beats T1/T2 (§2.7).
- **Session state in agentd**: a session object (turns, entities,
  pending slots) living for the window's duration plus a short tail;
  follow-ups resolve against it ("and how long does it take?" →
  the route just opened). Component: agentd. Purpose: follow-ups
  are meaningless without something to refer to. Effort: **M**.
  Proof: Key Test 2 T1; unit tests on entity carry-over in the
  agentd suite style.

### Phase 2 — Learned endpointing + confidence repair
- **Smart Turn v3 on-device**: run the 8 MB BSD-2 ONNX model in the
  listener (Rust glue, upstream model artifact — same exception
  class as mkbootimg) at each VAD pause; "incomplete" extends the
  window, "complete" ends the turn in ~0.3–0.5 s. Replaces fixed
  silence thresholds everywhere, including the wake utterance.
  Component: wakeword listener (and voiced's capture, unified).
  Purpose: stops both dead air AND mid-thought cut-offs — the two
  timing failures users actually remember. Effort: **M**.
  Proof: scripted pause test — a request spoken with a 1.5 s
  mid-thought pause completes as ONE turn; a finished request ends
  ≤0.6 s after the last word (trace timestamps).
- **ASR-confidence repair strata in agentd**, composed with BRAINFIX:
  high confidence → act; medium → implicit confirmation folded into
  the reply; low → one targeted question about the uncertain span
  only, slots preserved. BRAINFIX still governs whether an inferred
  *value* may act; confidence governs how we handle a misheard
  *word*. Component: agentd (+ BRAINFIX-INSTRUCTION.md addendum).
  Purpose: a mishearing today means starting over; repair means the
  conversation survives it. Effort: **M**. Proof: Key Test 2 T4,
  plus a deliberately mumbled destination in testing.

### Phase 3 — Streaming pipeline (first audio fast)
- **Overlap the stages**: agentd streams tokens from the gateway
  brain; voiced synthesizes and plays sentence-by-sentence as they
  arrive; STT partials appear live on the kimono waveform view.
  Poor-man's STT streaming first (chunked re-decode of the growing
  utterance) — honest note: Moonshine has no streaming API, so true
  partials wait for an STT decision; do not fake them.
  Component: voiced + agentd + gateway path. Purpose: first audio
  in ~1–2 s instead of full-answer latency; long answers start
  speaking immediately. Effort: **L**.
  Proof: median wake→first-audio ≤2 s over the Key Test 2 script on
  the gateway path; no sentence is ever played twice or out of order
  across 10 runs.
- **Preemptive start (tightening, after streaming works)**: begin
  brain generation on the endpointing model's "likely complete"
  signal; discard silently if the user resumes. Purpose: shaves
  the endpointing tail off perceived latency. Effort: **S** on top
  of Phases 2–3. Proof: perceived gap measurement in traces; zero
  cases of the phone answering a turn the user hadn't finished.

### Phase 4 — Barge-in lite (then real AEC as the tightening pass)
- **Duck-and-confirm**: while TTS plays, the listener's VAD stays
  live. On speech onset: duck playback ~24 dB instantly; if the
  sound survives the backchannel filter ("mm-hm"/"okay" don't
  count) and ~300 ms of sustained speech, stop playback and treat
  it as the next turn. Self-echo defense v1: echo-guard (ignore
  the first ~300 ms after playback ends) + compare against the
  transcript of what we just said.
  Component: wakeword listener + voiced playback. Purpose: the
  operator can stop a wrong or long answer by talking — the single
  most "it's alive" interaction there is. Effort: **M**.
  Proof: Key Test 2 T3; zero self-interruptions across 20 played
  answers in a quiet room and a noisy room (echo is the known risk —
  measure it, publish the count either way).
- **Tightening pass**: real AEC via the pmOS audio stack
  (PulseAudio/PipeWire echo-cancel with the TTS stream as
  reference) replaces the guard heuristics if Phase 4's numbers
  demand it. Effort: **M**. Proof: same 20-answer test, false-stop
  count strictly lower than lite.

### Phase 5 — A better voice (measured, not assumed)
- **Kokoro-82M evaluation on the phone**: benchmark Kokoro ONNX
  vs Piper on the SDM845 itself — real-time factor, first-audio
  latency, memory next to Moonshine + the brain, and a blind A/B
  listen by the operator. Adopt only if it wins on the phone, not
  on a Jetson chart. Component: voiced TTS. Purpose: Piper's
  flatness is the one quality gap with a realistic on-device fix.
  Effort: **S** (evaluation) / **S** (swap if it wins).
  Proof: the benchmark record + operator A/B verdict, filed next
  to the key-tests.
- **Text-side prosody rules** in agentd's reply style: short
  spoken-shaped sentences, sparing discourse markers, numbers and
  lists rephrased for ears. Purpose: free naturalness on any TTS.
  Effort: **S**. Proof: operator listen test on 5 stock answers.

### Phase 6 — Homelab duplex experiment (gated, at-home only)
- **Split-brain duplex pilot**: Moshi-class model served from the
  3090 (its reference hardware class), phone as a streaming audio
  endpoint over the LAN; the duplex model owns greeting, small
  talk, backchannels and barge-in, and delegates anything with a
  tool to agentd. Explicitly NOT the offline path and NOT a
  replacement for Phases 0–5 — an experiment with a kill criterion.
  Component: new homelab service + a thin phone-side streamer.
  Purpose: learn what full-duplex feel is worth to the operator
  before anyone designs around it. Effort: **L**.
  Proof: a Key Test 2 variant run through the pilot; kill if
  end-to-end gap isn't visibly better than Phase 3's cascade or if
  tool delegation leaks into the chit-chat layer.

### 2.7 Key Test 2: Conversation — spec sketch
Style and rigor follow Key Test 1 (same key-tests directory; verdict
language as careful as KT1's P1/P2/P3). Setup: phone on pmOS,
listener running, voice only, wake word allowed **once** at the
start. The operator runs four beats in one sitting:

- **T1 — Task + follow-up.** "Hey phone, open maps and give me
  directions to San Francisco." Then, no wake word: "How long does
  it take to get there?" Pass: both answered from one session; the
  follow-up resolves against the route just opened.
- **T2 — Repair without restart.** Mid-flow, the operator corrects:
  "No — the other San Francisco exit" / or corrects a destination
  the agent misheard. Pass: the agent repairs the one value and
  continues; nothing already established is re-asked.
- **T3 — Barge-in.** The agent begins a long answer; the operator
  interrupts: "Stop — just tell me the time." Pass: playback stops
  within ~0.5 s of the interruption and the new request is answered;
  the interrupted answer never resumes.
- **T4 — Clarification recovery.** A deliberately ambiguous ask
  ("directions to the airport" with two plausible airports in
  context, or a mumbled street name). Pass: exactly ONE targeted
  question, then the task completes on the answer.

Verdicts:
- **P1 — Conversational pass.** All four beats pass in one session:
  one wake word total, no task restarts, no clarification loops,
  median response gap ≤3 s, zero self-interruptions.
- **P2 — Conversational with assistance.** Beats complete, but with
  exactly one assist: one re-wake, one full repeat of a request, or
  one extra clarifying question beyond the first.
- **P3 — Turn-based pass.** All tasks complete but only as discrete
  commands (follow-up window unused or failed; barge-in absent) —
  i.e., Key Test 1 quality, honestly labeled as not yet
  conversation.
- **FAIL**, with codes: **C1** follow-up ignored / treated as new
  session · **C2** repair restarted the task · **C3** barge-in
  ignored, or agent interrupted itself on its own echo · **C4**
  clarification loop (≥3 questions on one value) · **C5** latency
  (median response gap >5 s across the script) · **C6** a bystander
  or TV utterance was accepted as the owner's turn.

## 3. Decision points for the operator (unbundled, undecided)

1. **Follow-up window shape.** After an answer, how long does the
   phone keep listening without a wake word — ~5 s (Alexa-like:
   fewer false accepts, more missed follow-ups), ~10 s
   (Google-like: more conversational, more household speech
   processed), or until an explicit "that's all" (most natural,
   most mic-hot time)? All processing is on-device either way.
   **DECIDED 2026-11-03 (operator): about 10 seconds.** On-device processing;
   the kimono waveform shows the listening state for the whole window.
2. **Where the duplex experiment sits.** Phase 6 (homelab
   speech-to-speech front-end) buys the best conversational *feel*
   at home at the cost of a second brain to keep honest; Phases 0–5
   keep one brain and work offline. Do both in this order, or is
   the cascade's feel enough that Phase 6 never earns its keep?
   **DECIDED 2026-11-03 (operator): both, cascade first.** CV0-CV5 build the
   on-device cascade; the duplex pilot (CV6) runs last, gated, judged against
   the cascade with its kill criterion.
3. **Brain for conversation turns.** The gateway brain (5090 lane)
   already deviated once on BRAINFIX discipline in live probes.
   Conversation turns could route to the on-device LFM line
   (INTENT's pinned direction; faster, private, weaker) with the
   gateway reserved for hard tasks — or stay gateway-first for
   smarts. This decides where Phase 3's streaming work lands.
4. **Proactive turns.** Key Test 2 is reactive. The car-Bluetooth
   example needs the phone to *open* a conversation on an event.
   Is a spoken proactive opener in scope for this plan's next
   milestone, or does it wait until reactive conversation passes
   Key Test 2 cleanly?

## 4. What NOT to do

- **Cloud realtime APIs (OpenAI Realtime, Gemini Live, Hume EVI) as
  the path** — cloud-dependence violates the on-device/homelab law
  and dies offline; they are benchmarks, not components.
- **Moshi (or any 7B S2S model) on the phone** — no real-time path
  exists on SDM845; pretending otherwise burns a milestone.
- **Adopting TEN / LiveKit / Pipecat as our runtime** — Python/
  WebRTC platform stacks fight the Rust law and duplicate agentd;
  steal mechanisms (Smart Turn, preemptive generation), not chassis.
- **Always-on cloud streaming for follow-ups** — the privacy cost
  the whole on-device design exists to avoid; the window is local
  or it doesn't ship.
- **Chasing <300 ms on the phone** — physics of this silicon plus a
  network brain; honest targets are ≤2 s first audio (cascade) and
  ≤0.6 s endpointing. Sub-300 ms lives only in the Phase 6 room.
- **Expressive-TTS envy (Sesame CSM / EVI-class) on-device** —
  GPU-class models; the phone's realistic ceiling is measured
  Kokoro vs Piper (Phase 5), judged on our hardware.
- **Backchannel spam** — firing "mm-hm" on a timer reads as a
  gimmick within one conversation; acks are capped, purposeful, or
  absent (Phase 0's rule).

## 5. Where this touches existing plan docs (for root adoption)

- `docs/PLAN.md` §"Track A — agentic OS on postmarketOS" (line 24):
  Phases 0–5 slot in as Track A's voice workstream; Phase 6 is a
  Track A experiment with homelab serving.
- `docs/PLAN.md` §"Phase M — on-device LFM evaluation" (line 44) and
  `docs/VISION.md`/INTENT: decision §3.3 interacts with the pinned
  LFM3-1.7B on-device brain direction; LFM2.5-Audio belongs on that
  evaluation list if the operator wants it there.
- `docs/workbench/user-simulations/key-tests/`: Key Test 2 spec
  lands beside `KEY-TEST-1-PHONE-RUN.md`; the repair strata extend
  `BRAINFIX-INSTRUCTION.md` as an addendum (confidence axis beside
  the provenance axis); the follow-up window changes behavior
  documented in `KEY-TEST-1-WAKEWORD.md`.
- `docs/STATUS.md` scorecard: none of Phases 0–5 moves the Redox
  layers; they move the Track A / Key Test line and should be
  reported as such, not as kernel progress.
