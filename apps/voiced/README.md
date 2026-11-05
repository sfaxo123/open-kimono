# voiced — the voice in/out service (Track A, step A4)

One supervised service does the whole voice turn on the phone:

```
capture → STT → POST /request to agentd → tail the journal for the answer → TTS → play
```

Every step writes a `trace-events` record, so the kimono view shows what was
heard, what was understood, which engine answered, how long it took, and what
failed. **The microphone belongs to this service, not to the browser**: a lock
file names the live holder and a second voiced refuses.

## Run

```sh
voiced --wav ask.wav --out answer.wav                 # recorded input (the safe path)
voiced --mic 'arecord -q -f S16_LE -r 16000 -c 1 -t raw' --seconds 8   # live capture, only when asked
voiced --config $HOME/agentic/voiced.conf --wav ask.wav
```

The journal goes to stdout (or `--journal PATH`); human-readable lines go to
stderr, so the journal always stays parseable.

## Engines are selected by config, not by compile time

No Wyoming STT/TTS server exists on this fleet today, so the adapters are
chosen at runtime (decision L2-D1 in the handoff STATUS):

| config | what it does |
| --- | --- |
| `stt = wyoming 127.0.0.1:10300` | Wyoming STT, bridged by `apps/voice-bridge` |
| `stt = cmd <command>` | the audio WAV arrives on stdin, the transcript on stdout; exit 3 means silence |
| `stt = stub <text>` | a fixed transcript — lets a phone trial run without a live mic |
| `tts = wyoming 127.0.0.1:10200` | Wyoming TTS, bridged by `apps/voice-bridge` |
| `tts = cmd <command>` | the text arrives on stdin, the command writes a WAV to argv[1] |
| `tts = none` | speak nothing; the answer is still journalled |

`apps/voice-bridge` is used unchanged for the Wyoming paths, so a Wyoming
server appearing later needs a config line, not a rebuild.

## Phone safety (PLAN rules, enforced here)

- `--wav` is the only input an automated run may use. `--mic` must be asked for
  by name; the live-mic trial is the operator's.
- `quiet = true` (the default) writes the answer WAV and **does not play it**.
  A quiet test proves the audio by its bytes.
- Playback is a separate command (`play = ...`), so a missing output device is
  a named failure rather than a silent one.

## Tests

```sh
cargo test          # 11 tests, 7 of them negative
```

The host tests drive the pipeline in-process with WAV fixtures
(`fixtures/ask-tone.wav`, `fixtures/silence.wav`), shell adapters standing in
for the phone's engines, and a fake agentd that speaks the real SSE contract.
The negatives assert that: a dead STT engine reports its own words and sends
no request; silence never reaches agentd; an unreachable agentd names the
address it tried; a run that errored is not spoken; a truncated WAV is not
counted as audio; a second holder of the mic is refused; a malformed journal
line is rejected visibly instead of guessed.

## Cross-app smoke (recorded)

`docs/track-a/evidence/A4-voiced-smoke.trace` — a real run of voiced against
the kimono replay server (`node apps/kimono-ui/server.mjs --fixture
water_reminder`): 30 gapless records, `mic_started → mic_frame×13 →
mic_stopped → stt_final → handoff → span → handoff → wake_lock_released`, and
the answer spoken was the assistant's message from that journal, not an
invented one. Feeding the same file through `web/render.mjs` renders 30/30
with zero unhandled records.
