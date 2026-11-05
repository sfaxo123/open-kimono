# CV3 RESULTS — streaming execution (first audio in seconds, not ~17 s)

Status: implemented, host-proven, phone-proven (wav-driven), NOT
committed — root adopts. Base: main @ bb40adb, scratch clone
~/work/cv3 on the 3090.

## What shipped

The serial turn is broken at both ends:

**agentd — the brain streams, the journal fills inside the run.**
- `model::chat_stream` sends `"stream": true` and reads the SSE
  body incrementally (chunked-transfer aware). A brain that
  answers with plain JSON instead is parsed whole and reported
  as buffered — never an error for not streaming.
- Sentences are cut by `stream::SentenceChunker` and journaled
  LIVE as `answer_chunk` Custom records (seq-ordered inside the
  run), followed at round end by one `answer_stream` record
  naming the mode: `streamed` / `buffered` / `preamble` /
  `aborted`. `RUN_FINISHED`'s result now carries `streamed` and
  `chunks`.
- Two sentence sources, because the deployed brains answer two
  ways: streamed **prose content**, and the **`answer` tool's
  arguments** — the answer text streams inside
  `{"text": "…"}` argument fragments, so
  `stream::AnswerTextExtractor` decodes that JSON string
  incrementally (escapes, \uXXXX, surrogate pairs, fragments
  split anywhere). A round that ends in other tool calls marks
  its streamed content `preamble` — spoken text is never
  mistaken for the answer.
- Mid-stream failure after ≥1 journaled chunk: the run FAILS
  (`stream_aborted`) rather than falling back — a turn never
  mixes two brains' text. Failure before any complete sentence
  falls back through the brain chain exactly as before.
- Run-id adoption: `POST /request` accepts an optional `run_id`
  (validated shape); the run adopts it verbatim. Absent the
  field, the wire shape and behavior are byte-identical to
  pre-CV3.

**voiced — synthesis starts at the first sentence.**
- voiced mints the run id, starts the journal tail BEFORE the
  POST (the POST runs on its own thread), and hands the id to
  agentd, so chunk records bind to the turn from the first byte.
- `stream::ChunkPlayer` synthesizes and plays each chunk
  sentence in order as it lands (existing cmd adapters, existing
  player), concatenating chunk WAVs into the turn's `out_wav`.
  Whole-answer synthesis now happens ONLY when zero chunks
  played (pre-CV3 agentd, buffered brain) or after a preamble.
- Ack interaction: the player owns the turn's AckWatch and
  settles it before the first chunk — at most one ack per turn,
  and a fired ack never overlaps answer audio.
- The tail classifies endings with the pre-CV3 rules verbatim
  (finished bracket → answer; unconfirmed bracket → the
  JournalDropped note; established-but-unfinished → loud
  failure, never a re-tail). If agentd named a DIFFERENT run
  than the minted id (pre-CV3 server), the tail REBINDS:
  buffered frames are replayed under the adopted id on the same
  connection — no chunk can have been spoken, because chunks
  bind to the minted id only.

## Measurements — phone, warm, wav-driven (2026-11-04)

Fixture: `~/agentic/fixtures/ask-maps-cv3.wav` (see deviation
6), transcript verified verbatim on every run: "Open maps and
give me directions to San Francisco." Each turn its own session
(sess-cv3m-1..5), config `voiced-cv3.conf`, warm STT/TTS up,
gateway brain. Times from the agentd journal (µs) and chunk WAV
mtimes (1 s granularity); wall is the voiced process wall.

| turn | transcript→run | run→1st chunk | transcript→1st chunk | 1st chunk WAV written | brain total | chunks | wall |
|------|---------------:|--------------:|---------------------:|----------------------:|------------:|-------:|-----:|
| maps1 | 0.00 s | 2.18 s | 2.19 s | 6.2 s | 2.46 s | 2 | 21 s |
| maps2 | 0.00 s | 1.79 s | 1.79 s | 6.5 s | 1.79 s | 1 | 18 s |
| maps3 | 0.00 s | 1.63 s | 1.63 s | 6.5 s | 1.63 s | 1 | 18 s |
| maps4 | 0.00 s | 1.54 s | 1.54 s | 4.7 s | 1.54 s | 1 | 14 s |
| maps5 | 0.00 s | 1.80 s | 1.80 s | 6.6 s | 1.80 s | 1 | 18 s |

All five runs journal `answer_stream` mode `streamed`;
RUN_FINISHED carries `streamed: true`. maps1's two chunks came
through the answer-tool extraction path ("homelab-gateway
streamed the answer tool's text in 2 sentence chunk(s)") and
chunk1 was synthesized while chunk0 was still playing — the
pipeline overlaps. Five further battery-question turns the same
morning (the shipped fixture, sess-cv3-1..5) also all streamed,
1 chunk each, walls 11–20 s.

**Honest reading against the brief's bars:**
- First answer sentence EXISTS 1.5–2.2 s after the transcript —
  inside the ~2.5 s bar.
- First answer AUDIO reaches the speaker at ≈4.7–6.6 s (chunk
  WAV written; playback starts immediately after). The gap is
  the CV0 ack settling first BY DESIGN (1.5 s delay + the
  "got-it" clip played to completion, ~2–3 s total) plus
  first-sentence synthesis (~2 s at RTF ≈0.45 for these long
  first sentences). The ~2.5 s first-audio bar is therefore
  MISSED as deployed; the lever is named, not hidden: overlap
  first-chunk synthesis with the ack, or drop the ack when the
  first chunk lands before the delay fires.
- Total wall (14–21 s) is NOT materially below CV0's ~17 s warm
  turns, and on these runs could not be: the gateway was fast
  (brain total 1.5–2.5 s vs CV0's 2–8 s), and the wall is
  dominated by realtime playback of the fixture (~3.5 s) and of
  the spoken answer (~8–10 s), which streaming cannot compress
  — it can only start sooner. The structural win scales with
  brain latency and answer length: pre-CV3, first audio waited
  for the LAST sentence plus whole-answer synthesis; now it
  waits for the first sentence plus one synthesis.

## Test counts

| suite | before | after |
|-------|-------:|------:|
| agentd | 118 | **138** (+8 streaming integration, +12 chunker/extractor unit) |
| voiced | 56 | **59** (+3 streaming: chunk-before-finish, no-chunk classic path, chunk-then-error) |
| kimono-ui | 32 | 32 (untouched) |

All hardening tests (c91d903) and the cce8a80 run-bracket
behavior stay green, including the fakes' connection-count
invariants (pipeline: exactly 3 connections; mid-run death:
exactly 1, loud failure).

## Deviations (with evidence)

1. **Wire facts corrected from source, not the brief's
   summary.** voiced POSTs `{"text": …}`; tail frames carry
   camelCase `runId`; the POST returns `202 {"run_id": …}`;
   and deployed answers flow through the `answer` tool's
   arguments — the reason the extractor exists. Found by
   reading apps/voiced/src/agentd.rs and apps/agentd/src/*.rs
   before designing.
2. **Sentence-proof rule is stricter than first sketched.** A
   terminator only cuts when followed by whitespace AND a
   plausible sentence start (capital, digit, opening quote) —
   "p.m. and…" no longer splits. Two inherited unit tests were
   reconciled to the documented rules (the abbreviations case
   now asserts the no-split property via the finish() tail).
   Rationale recorded in stream.rs.
3. **Rebind lives inside the tail.** The first design handed a
   foreign run id back to run() for a classic re-tail; the
   hardening fakes (order-independent, replay-based, exact
   connection counts) proved that shape wrong. Rebinding over
   buffered frames on the same connection satisfies both the
   real pre-CV3 server and every fake invariant.
4. **JournalOut is not Send.** Helper threads (POST, chunk
   player) keep private journals and return their trace events;
   the turn merges them (trace ids preserved; merge re-stamps
   ts_us, which is why chunk timing above is read from the
   agentd journal and WAV mtimes, not the merged voiced
   journal).
5. **A non-streaming brain degrades, never fails** — mode
   `buffered` is journaled and voiced speaks the whole answer
   exactly as pre-CV3 (proven by the buffered-brain integration
   test).
6. **The shipped fixture is not the maps turn.**
   `ask-request.wav` transcribes as "What is the battery level
   right now?" on the current warm STT. The maps fixture was
   synthesized with the phone's own warm TTS
   (`ask-maps-cv3.wav`, Piper voice) and its transcript
   verified verbatim on all five runs. The five battery turns
   were kept as additional streaming evidence, not discarded.
7. **First driver launch was reaped with its SSH session**
   (phone logind scope cleanup; the attempt died inside STT
   and never reached agentd). All phone-side drivers after
   that ran under `systemd-run --user`, per the standing
   lesson.

## Phone state left

- `agentd-main-keytest`: CV3 build RUNNING (md5
  34b95f8756675aca23ff42c4995aaca8); backup of the CV2 build at
  `agentd-main-keytest.bak-20261104-precv3`.
- `voiced`: CV3 build in place (md5
  b106adb7d9bed94c871a44e4348bbc35); backup at
  `voiced.bak-20261104-precv3`.
- `agentd-main` untouched — sha256
  f165f7063bbad9ece1421da60718fd975f61d26533963ae8806873126c59bc27,
  verified before and after.
- Additive only: `voiced-cv3.conf` (keys identical to
  voiced-cv2.conf — streaming is negotiated over the journal,
  not config), `fixtures/ask-maps-cv3.wav`, `out/cv3/` evidence,
  `cv3-driver.sh` / `cv3-driver2.sh`.
- Warm services STOPPED, wake-word listener STOPPED — the
  phone is as found. Deployed configs untouched.
