# Track A · voice in / voice out (`voiced`)

`apps/voiced/` is one supervised service on the phone that turns speech into an
agentd request and the agent's answer back into sound, with every step written
to the same trace-events journal the kimono view renders. It is the voice half
of "the same request over voice and text must produce the same journal shape".

## The turn

```
lock the mic → power_change(on) → wake_lock → capture → power_change(off)
  → stt → text_cleaned → POST /request → tail the journal → tts → play
  → wake_lock_released
```

Each arrow is a record: `mic_started` / `mic_frame` / `mic_stopped`,
`stt_partial` / `stt_final`, `text_cleaned`, `handoff` (started / completed /
failed), a `gen_ai` `execute_tool` span around the STT call, `power_change`,
`wake_lock_acquired` / `wake_lock_released`, and `playback` (in quiet mode:
`playback:none` with the reason). A failed step journals a span error plus
`RUN_ERROR` and a failed handoff — never a silent success.

## Engines are config, not code

No Wyoming STT/TTS server exists anywhere in the fleet, so the engine is chosen
in `voiced.conf`:

| key | meaning |
|---|---|
| `stt` | `wyoming <host:port>` · `cmd <script>` · `stub <text>` |
| `tts` | `wyoming <host:port>` · `cmd <script>` · `none` |
| `agentd` | `http://127.0.0.1:8770` (no path, port required) |
| `play` | shell command that plays a WAV, or `none` |
| `quiet` | `true` → write the answer WAV, never run `play` |
| `lock` | mic ownership lock file (one holder at a time) |

`cmd` adapters are deliberately dumb: STT reads the WAV on stdin and writes the
transcript on stdout, **exit 3 means silence**; TTS reads the text on stdin and
writes a WAV to `argv[1]`, and voiced re-reads that WAV — a tool that exits 0
and produces unusable audio is a `TtsFailed`, not a success.

## Phone adapters (`apps/voiced/phone/`)

The phone's own `say.sh` / `listen.sh` drive moonshine and piper-TTS by
`podman exec` into the long-running `voice` container. Track A's phone-safety
rule forbids touching that container, so voiced ships its own adapters: one
throwaway `sudo -n podman run --rm --network none --userns=keep-id` per call,
`/opt/models` mounted read-only, the existing brainkit drivers mounted
read-only, no audio devices and no ports. The text is staged in a file rather
than interpolated through a second shell level.

Deploy them with `scp` to `~/agentic/bin/` and point `voiced.conf` at them.

## Correlating the answer

agentd answers `POST /request` and journals the run, but journals **no user
message** (see `docs/track-a/KIMONO-UI.md`). So voiced reads the journal length
from `GET /health` before posting and resumes `GET /events` with
`last-event-id: <last seq>`, which makes the run it reads back its own. When the
server journals no user message it writes a journal note saying the answer was
matched by position rather than by text.

## Running it on the phone

```sh
# one command, from the homelab; it reaps agentd on every exit path
ssh oneplus 'sh ~/agentic/bin/run-quiet-test.sh'
```

The test is **quiet**: the input is a pre-recorded WAV (no live microphone) and
`quiet = true` means no sound is produced — the answer is proven by the bytes of
`out/a4-answer.wav`, never by listening to the speaker. `IN_WAV=<path>` overrides
the input. Pull the journal back as
`docs/track-a/evidence/A4-phone-quiet-test.trace` and render it through
`apps/kimono-ui/web/render.mjs` to prove the voice turn and the text turn look
the same on screen.

## What is proven where

- Host: `cd apps/voiced && cargo test` → 12 tests, 8 negative, against a fake
  agentd that speaks real HTTP and real SSE.
- Cross-app: voiced driven against the kimono replay server →
  `docs/track-a/evidence/A4-voiced-smoke.trace`, 30 gapless records, all
  rendered by the page with zero unhandled.
- Phone: both adapters verified standalone (ptts synthesised 2.4 s of speech in
  1.38 s; moonshine transcribed that same WAV back exactly), and one real turn
  reached agentd, which answered — the journal of that run is in the phone's
  `~/agentic/out/`. The full quiet loop is pending: the phone went offline
  before it could complete.
