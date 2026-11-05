# RESEARCH — How others make voice agents feel conversational

Compiled 2026-11-03. Tags: **[measured]** = peer-reviewed or independently
benchmarked · **[documented]** = primary vendor/framework docs · **[vendor
claim]** = maker's own numbers · **[synthesis]** = secondary roundup of the
above. Our baseline for contrast: one-shot `voiced`, fixed 8 s capture,
batch STT→LLM→TTS in throwaway containers, ~19 s acoustic turn.

## 1. Turn-taking & endpointing

- Human turn gap averages **~200 ms** across languages (Stivers et al.,
  PNAS 2009, 10 languages; Levinson & Torreira 2015). Listeners predict
  turn ends; they don't wait for silence. **[measured]**
  https://www.evalgent.com/blog/voice-agent-stack-guide
- Perception bands used across practitioner literature: <300 ms feels
  human, 300–500 responsive, 500–800 noticeable, >800 delayed, >1.5 s
  "broken / are you there?" **[synthesis]**
  https://github.com/whereiskurt/klanker-voice/blob/HEAD/.planning/research/FEATURES.md
- Silence-only endpointing defaults are far above the human gap:
  LiveKit endpointing `min_delay` 0.5 s / `max_delay` 3.0 s; with its
  audio turn detector, 0.3 s / 2.5 s; a `dynamic` mode adapts the delay
  by EMA. **[documented]** https://www.evalgent.com/blog/cascading-vs-speech-to-speech-voice-agents
- **Pipecat Smart Turn v3** is the reference open design: a tiny audio
  model (Whisper-tiny encoder + linear head) that judges "did they
  finish?" from prosody/semantics of the last ≤8 s of audio at each VAD
  pause. 8.68 MB int8 ONNX, BSD-2, **12–95 ms on CPU**, all local. If it
  says "incomplete," a 3.0 s silence fallback force-completes. Reclaims
  ~400 ms of dead air vs fixed-timeout VAD in one tuned deployment.
  **[documented + deployment report]**
  https://github.com/pipecat-ai/smart-turn ·
  https://github.com/arthurdevel/voicecc/blob/HEAD/.docs/notes/2026.03.12-turn-detection-options.md
- **OpenAI `semantic_vad`**: a classifier scores P(turn complete) and
  sets the wait dynamically; `eagerness` low/medium/high caps the wait
  at 8 s / 4 s / 2 s. Trailing off ("uhhm…") scores low → waits longer.
  **[documented]** https://developers.openai.com/api/docs/guides/realtime-vad
- Deepgram Flux "eager end-of-turn" claims ~150 ms median / ~350 ms p95
  saved by firing before silence confirms. **[vendor claim, via]**
  https://github.com/mohamedabdallah-14/unslop/blob/HEAD/docs/research/08-conversational-dialogue-systems/B-industry.md
- Pattern worth stealing: an STT that emits an **end-of-utterance token**
  (and a distinct end-of-backchannel token) during streaming decode —
  LocalAI documents this with Parakeet EOU models; the turn commits
  ~300 ms after the token instead of after a silence timer.
  **[documented]** https://github.com/dedyf5/localai/blob/HEAD/docs/content/features/openai-realtime.md
- Contrast: our fixed 8 s capture window is ~16–25× the endpointing
  tail these systems pay (0.3–0.5 s), paid on every phrase-only turn.

## 2. Streaming pipelines & latency budgets

- The cascaded-stack budget everyone aims at: **≤800 ms to first
  audio** — endpointing 200–500 ms, STT finalization 50–150 ms
  (streaming; batch STT adds 300–800 ms), LLM TTFT 150–400 ms, TTS
  first audio 50–150 ms (sentence-chunked), transport 50–150 ms.
  **[documented/synthesis]**
  https://www.forasoft.com/blog/article/livekit-ai-agent-development ·
  https://www.max-gherman.dev/design/anatomy-of-ai-agent/voice-and-real-time-conversational-agents/
- The three overlap mechanisms, in impact order: (1) **streaming ASR
  partials** so nothing waits for the full utterance; (2) **preemptive
  generation** — LiveKit starts the LLM on partial transcripts before
  end-of-turn confirms, discarding if the user continues
  **[documented]**; (3) **sentence/phrase-chunked TTS** — synthesize at
  the first clause boundary (~5–7 words) while the LLM keeps writing.
  Serial pipelines measure 2–3 s; overlapped ones 300–800 ms.
  https://github.com/nunomarques97/jarvis/blob/HEAD/docs/NATURALNESS.md
- **Jitter beats mean**: a consistent 600 ms feels better than a
  500 ms average with occasional 2 s turns. **[synthesis]**
  https://www.gmicloud.ai/en/blog/dedicated-inference-endpoints-for-real-time-voice-ai-latency-jitter-and-infrastructure-requirements
- GPT-4o's launch numbers — audio responses in as little as 232 ms,
  320 ms average — are the speech-to-speech reference point.
  **[vendor claim]** (via) https://github.com/mohamedabdallah-14/unslop/blob/HEAD/docs/research/08-conversational-dialogue-systems/E-practical.md
- Where *our* seconds go (from our own traces, brief's trusted facts):
  container spin-up per stage (throwaway podman), batch everything,
  no overlap anywhere. Warm processes + any overlap at all is the
  largest available win; the models are not the bottleneck.

## 3. Barge-in / interruption

- Barge-in = keep listening while speaking = full-duplex audio path.
  **AEC is table stakes**: feed the TTS output back as a reference and
  subtract it from the mic, or the agent hears itself. Documented
  failure without it: a car-speaker deployment cancelled **55 of 304**
  of its own responses via echo-triggered VAD, 51 of them false
  `turn_detected`. **[field report]**
  https://github.com/incidentfox/box/blob/HEAD/docs/voice-half-duplex.md
- Mitigation ladder, cheapest first: headphones → **echo-guard window**
  (drop mic audio ~300 ms after TTS stops) → **half-duplex mic gating**
  (mic muted while speaking; zero echo loops, but barge-in impossible)
  → true AEC (WebRTC AEC3 / SpeexDSP with reference signal).
  **[documented patterns]**
  https://github.com/protolabsai/protovoice/blob/HEAD/docs/guides/audio-handling.md
- Production handling is **two-stage**: on VAD fire during playback,
  duck agent audio (~24 dB) immediately; only yield the floor if the
  sound is confirmed as real speech — duration + a **backchannel
  lexicon filter** ("uh-huh", "okay", "right" never take the floor).
  Emerging target: <200 ms from user speech onset to TTS suppression.
  **[field report + synthesis]**
  http://dev.to/kenimo49/barge-in-is-the-voice-agent-feature-nobody-benchmarks-i-added-it-and-lost-120ms-4o7m
- Pipecat's interruption is VAD-driven at ~150–200 ms with pipeline
  cancellation frames; LiveKit markets "adaptive interruption" that
  separates a cough from an interruption. **[documented]**
  https://github.com/arthurdevel/voicecc/blob/HEAD/.docs/notes/2026.03.12-turn-detection-options.md
- Cost honesty: one builder measured barge-in **adding 120 ms** to
  their headline turn latency (continuous VAD + filtering isn't free).

## 4. Backchannels, fillers, acknowledgments

- In human corpora, **~19% of utterances are backchannels**
  (Switchboard, 1,155 calls; Ward & Tsukahara 2000), typically landing
  **~700 ms** after a prosodic cue (pitch drop, clause boundary).
  **[measured]** https://www.evalgent.com/blog/backchanneling-proactivity-voice-agents
- Split four behaviors that get conflated — different mechanisms,
  different risks: (1) **listener backchannel** while the user holds
  the floor ("mm-hm"); (2) **turn-initial acknowledgment** ("Got it —
  the 14th"); (3) **latency mask / filler** while tools or the brain
  run ("One sec, pulling that up"); (4) **proactive turn** after
  silence. Only (1) is a true backchannel, and it needs duplex audio;
  **(2)–(4) are all achievable in a half-duplex cascade today.**
  **[documented analysis]** (same Evalgent source)
- A turn-initial ack is the cheapest naturalness win in the field: it
  converts dead air into "it heard me, it's thinking." Practitioner
  consensus: an immediate ack makes ~900 ms of thinking feel like
  ~300 ms. Overuse is the failure mode — scripted-sounding repetition
  and fillers that delay fast answers or commit too early.
  **[synthesis]**
  https://github.com/whereiskurt/klanker-voice/blob/HEAD/.planning/research/FEATURES.md
- Even full-duplex models miss human backchannel timing: Full-Duplex-
  Bench scores timing distributions against human listeners; timing,
  not vocabulary, is what separates them. **[benchmark, via Evalgent]**
- RESPOND (2026) shows the controllable version: independent dials for
  **backchannel intensity** and **turn-claim aggressiveness**, tuned
  per context — a design pattern (make it a knob, default low), not a
  product to adopt. **[paper]** https://arxiv.org/pdf/2603.21682

## 5. Repair & clarification

- Industry standard is **confidence-stratified repair**, not a binary
  understood/not: high confidence → proceed; medium → **implicit
  confirmation** (embed the heard value in the reply: "So that's the
  14th — heading there now"); low → **targeted clarification of the
  uncertain span only** (ask about the street name, not the whole
  request); repeated failure → simplify the ask, then offer another
  input path. Generic "please repeat" loops are the anti-pattern.
  **[documented patterns]**
  https://deallogic.webflow.io/ai-voice-knowledge-node/voice-interaction-error-handling
- When a user just repeats themselves, **eliciting** the missing piece
  beats confirming a guess (learned dialog policies chose elicitation
  82% of the time vs 61% for fixed rules, with equal success in fewer
  turns). **[paper]**
  https://d39w7f4ix9f5s9.cloudfront.net/59/d9/4e08473141a3b64188797f7db70e/17nipsw-cai-deep-rl-dialog.pdf
- The governing rule: **calibrate clarification to the cost of being
  wrong**, not to a universal accuracy threshold. **[synthesis]**
  https://llmbook.icsgen-ai.org/part-8-conversational-ai-with-llms/module-37-conversational-ai/section-37.4.html
- Where our BRAINFIX discipline agrees: read-back confirmation before
  acting on an *inferred* value; never treat a guess as an instruction.
  Where BRAINFIX goes beyond industry practice: it also covers
  *speaker attribution* (a bystander's "yeah" is not the owner's
  confirmation) and *who framed the task* (agent-framed vs
  user-framed). Industry repair is keyed to ASR confidence; ours is
  keyed to provenance of the value. The two compose: confidence bands
  decide *how* to ask; BRAINFIX decides *whether the value may act*.
- Repair must not restart the task: targeted clarification preserves
  the slots already filled. Users reportedly abandon after ~3 failed
  correction rounds — one good targeted question up front saves them.
  **[secondary]** (Stanford figure via the llmbook source above)

## 6. Expressive speech out

- The frontier gets prosody *from the model, not from markup*:
  **Sesame CSM** (open weights, 2026) generates conversational
  prosody — pauses, emphasis, turn-taking cues — and runs on a single
  consumer GPU; **Hume EVI** takes prosody *in* (end-of-turn +
  response conditioning) and answers in a matching tone (~300 ms
  TTFB, vendor figure); `gpt-realtime` takes style as a prompt
  ("speak empathetically…"). **[documented/vendor]**
  https://github.com/mohamedabdallah-14/unslop/blob/HEAD/docs/research/08-conversational-dialogue-systems/B-industry.md
- **Piper (ours)** is a narration engine: VITS ~22M params, MOS ~3.65
  rated "slightly flat," no emotion control, effectively no usable
  SSML prosody control. It cannot do expressive speech; no prompt
  trick changes that. **[benchmark]**
  https://www.reciwo.com/articles/on-device-tts-benchmark-kokoro-82m-vs-cosyvoice-edge-npu
- The one real on-device step up: **Kokoro-82M** — Apache-2.0, 82M
  params (~90 MB int8), StyleTTS2-derived, measurably more natural
  prosody than Piper (MOS ~4.35 in the same edge benchmark), time to
  first audio 28–46 ms **on M5 Pro / Jetson Orin NX — not SDM845;
  must be measured on our silicon before believing any of it.** No
  emotion control, no cloning; its flatness ceiling is blamed on
  synthetic training data. Runs on CPU-class hardware per its model
  card and community reports. **[benchmark + model card]**
  https://github.com/calesthio/generative-media-skills/blob/HEAD/skills/providers/text-to-speech/kokoro-tts/SKILL.md
- CosyVoice2-0.5B-Stream scores higher (MOS ~4.48, 115 ms TTFA) at
  540 MB — a homelab-side candidate, a stretch on the phone.
  **[same benchmark]**
- What flat TTS *can* do: text-side prosody — short sentences,
  punctuation that forces pauses, sparse discourse markers ("Okay —
  ", "So…"), and pre-rendered ack clips. Zero model cost.

## 7. Conversation state & memory

- Consumer follow-up modes are a **listening window, not memory**:
  after each answer the mic stays hot for follow-ups without the
  wake word — Google Continued Conversation ~8–10 s; Alexa Follow-Up
  Mode ~5 s of silence tolerance (published figures vary by source
  and firmware; treat exact seconds as approximate). The audio buffer
  is RAM-only and session-scoped; the window closing ends the
  session. **[documented, numbers approximate]**
  http://umatechnology.org/how-voice-assistants-use-context-to-understand-what-you-mean/
- Context *understanding* is separate from the window: systems carry
  entities/slots across turns in the session so "what about
  tomorrow?" resolves against the previous turn (Alexa Conversations'
  short-term entity memory; Google's contextual rephrasing research).
  **[documented]** (same source)
- Privacy tradeoffs are the known cost: household speech inside the
  window gets processed (false-activation complaints are a documented
  class), bystanders are captured, and a hot mic is a battery and
  trust cost. Mitigations everyone converges on: on-device processing
  of the window, a **visible listening indicator**, short window, no
  raw-audio retention. Our kimono waveform (on by default per INTENT)
  is exactly that indicator.
- Multi-turn state lives in the agent, not the audio layer: session
  object holding turns + entities + pending slots, expiring with the
  window. That maps directly onto agentd; the wakeword listener only
  owns the window.

## 8. Full-duplex speech-to-speech models

- **Moshi (Kyutai)**: 7B temporal transformer over two simultaneous
  audio streams (user + self) plus an inner-monologue text stream;
  Mimi codec, 24 kHz → 12.5 Hz frames at 1.1 kbps, 80 ms frame
  latency. **Theoretical 160 ms, ~200 ms in practice on an L4 GPU.**
  Backends: PyTorch (~24 GB VRAM bf16), MLX (int4/int8, Apple
  silicon), Rust/Candle (int8). **No tool calling.** English-first;
  session context is minutes-scale. **[documented]**
  https://deepwiki.com/kyutai-labs/moshi ·
  https://github.com/kadirnar/voice-agent-next/blob/HEAD/docs/research/02-speech-to-speech-models.md
- **PersonaPlex-7B** (NVIDIA, Moshi-derived) reports ~170–250 ms with
  persona control. **[secondary]**
  https://github.com/jayleaton/local-live-1/blob/HEAD/docs/research/2026-09-local-voice-jarvis.md
- **SDM845 verdict: not viable on-device.** No published real-time
  result on SD8xx-class silicon; 7B audio-token generation at 12.5
  steps/s with a depth transformer, in 8 GB shared with the OS, is
  beyond this chip. MLX phone demos are iPhone 15 Pro-class (A17 Pro
  + unified memory), a different league. Anyone claiming otherwise is
  selling something.
- **Homelab verdict: plausible and interesting.** A 3090 (24 GB) is
  literally Moshi's reference hardware class. The documented pattern
  is **split-brain**: the duplex model owns conversation feel and
  barge-in; a tool-calling cortex (our agentd) owns actions, with the
  front-end delegating. S2S alone cannot run our tools.
  **[documented pattern]** (local-live source above)
- Also on the radar, catalog-sourced only: **LFM2.5-Audio-1.5B**
  (LiquidAI) listed among open S2S models — relevant because the
  project already pins Liquid's LFM line; specs unverified here.
  https://uniocommunity.com/open-source-voice-ai-models

## 9. Frameworks — mechanisms worth stealing (not adopting)

- **Pipecat** (BSD-2): frame-based pipeline with first-class
  interruption/cancellation frames; sentence aggregator before TTS;
  Smart Turn v3 as a pluggable turn-stop strategy. *Steal: Smart Turn
  itself (8 MB ONNX runs anywhere) + the cancellation-frame idea for
  voiced/agentd.*
- **LiveKit Agents** (Apache-2.0): `turn_handling` as one config
  surface (min/max delay, dynamic EMA endpointing, turn detector);
  **preemptive generation** on partial transcripts; adaptive
  interruption classification. *Steal: the config surface shape and
  preemptive generation.*
- **Kyutai Unmute**: streaming STT/TTS wrapped around *any* text LLM.
  *Steal: the shape — it's the closest published analog to bolting
  streaming voice onto our existing agentd brain.*
- **TEN Framework** (Agora): graph-composed agents, ships its own VAD
  and turn-detection models. *Steal: nothing structural — its runtime
  and Agora RTC coupling fight our Rust law; note and pass.*
- **Ultravox** (Fixie): speech encoder feeding an LLM directly —
  keeps prosody that ASR text throws away, low TTFT. *Steal: the
  idea as a homelab experiment class, not a component.*
- **Deepgram Flux / OpenAI semantic_vad**: eager/semantic endpointing
  as a *service*. *Steal: the eagerness knob concept — one operator-
  facing dial, three named settings.*

## The one-paragraph summary

Naturalness in 2026 voice systems is mostly **timing, not voice
quality**: a ~200 ms human turn gap as the reference, learned
end-of-turn prediction instead of silence timers, overlapped
streaming stages with an ≤800 ms first-audio budget, an instant
acknowledgment while thinking, confidence-stratified repair that
never restarts the task, and — only at the frontier — full-duplex
audio models that get prosody and barge-in for free but can't call
tools. Every one of those except full duplex is achievable inside a
cascade like ours; the cascade's ceiling is set by discipline
(endpointing, overlap, warm processes), not by model size.
