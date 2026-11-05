# voice-bridge

The seam between the phone and the homelab speech engines: a tiny CLI
that streams a 16 kHz mono WAV to a Wyoming STT server, or text to a
Wyoming TTS server, emitting **every step as trace-events JSONL on
stdout**. Nothing is silent — a closed or erroring server always ends
with an explicit error event.

```
voice-bridge --stt HOST:PORT --wav IN.WAV [--language LANG]
voice-bridge --tts HOST:PORT --text TEXT [--out OUT.WAV] [--voice NAME] [--language LANG]
```

The transcript goes to stderr; the journal goes to stdout, so
`voice-bridge --stt … | jq` works. Exit code is non-zero on any failure.

## What lands in the journal

| Step | Event |
| --- | --- |
| session start / end | `op6 handoff` (started → completed/failed) |
| the server exchange | GenAI `execute_tool wyoming-stt` / `wyoming-tts` span (start/end, ok/error) |
| each streamed audio chunk | `op6 mic_frame` with the chunk's RMS level |
| partial / final transcript | `op6 stt_partial`, `op6 stt_final` (with `latency_us`) |
| any failure | span end `error` + `ag_ui RUN_ERROR` + `handoff failed` |

Wyoming TTS has no dedicated op6 event type; the handoff + tool span +
RunError triad carries it so the phone UI can announce and show the
hand-off (VISION §5) without inventing new envelope variants.

A TTS WAV is written **only after a complete stream**: a server that
closes mid-audio, sends an `error` event, or ends with an odd byte
total (half a 16-bit sample) produces an explicit failure and no file.

## Design notes

- Deps: `crates/wyoming` + `crates/trace-events` only — no tokio, no
  rand. Trace/span ids come from a deterministic counter; timestamps
  are wall-clock µs.
- `wav.rs` walks real RIFF chunk lists (skips unknown chunks with their
  word pad) and refuses float/8-bit input explicitly.
- Journal lines are flushed per event, so a killed process leaves a
  readable, gapless prefix.

## Tests

```sh
cargo test          # 14 unit + 6 integration tests
```

Integration tests run against in-process fake Wyoming servers on
loopback, including the negative cases (server closes mid-stream,
server `error` event, truncated TTS audio) and assert the journal
order, gapless seq, and sink/ring agreement.
