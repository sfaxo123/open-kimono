# C4 evidence — Timeline app (`apps/timeline`)

Wave-1 task C4: minimal OrbClient open-kimono screen fed by `trace-events` JSONL.
Branch `apps/timeline`, base `4ddfde1`.

## What was built

`apps/timeline/` — one crate, one screen, black + orange (VISION §8):

- `src/model.rs` — pure event→state machine over `trace_events::Event`: mic
  level/live + stop reason, transcript (partial/final), cleaned text,
  reasoning stream (per `messageId`), tool-call lifecycle, services
  (name+generation), power + wake locks, hand-off, run error, genai span
  header. Bounded rows (6 tools / 8 services / 8 power), newest-first trim.
- `src/feed.rs` — tolerant JSONL tailer (file or stdin): partial lines held at
  the byte level, corrupt lines counted, never fatal.
- `src/view.rs` — the renderer, generic over `orbclient::Renderer`: the live
  SDL window and the off-screen `Surface` used for screenshots run the same
  draw path.
- `src/png.rs` — minimal PNG writer (RGBA8, flate2 IDAT, hand-rolled chunks)
  so evidence needs no image crate.
- `src/main.rs` — `--demo` (fixture replay paced on `ts_us`, `--speed`/`--instant`),
  `--stream <path|->` (live journal), `--screenshot <png>`, `--exit-after <ms>`.
- `fixtures/demo.jsonl` — 28-event "navigate me home" scenario in the real
  C2 schema (mic → STT → cleaned → genai span → reasoning → tool → power →
  service → wake lock → hand-off → span end → mic stop).

## Verification

```
$ cd apps/timeline && ~/.cargo/bin/cargo test
test result: ok. 15 passed  (tests/model.rs — every event family + clamping, pruning, unknown payloads)
test result: ok.  3 passed  (tests/feed.rs — bad lines counted, partial lines held, file tailing)
test result: ok.  2 passed  (tests/fixture.rs — fixture parses 28/28 against C2 schema, drives coherent screen)
test result: ok.  3 passed  (tests/png.rs — CRC vectors, full structural + decompressed round-trip)
test result: ok.  6 passed  (tests/view.rs — wrap/tail + pixel checks: orange bar on black, idle has no fill)
total: 29 passed, 0 failed; cargo clippy: 0 warnings
```

Headless runs (`SDL_VIDEODRIVER=dummy`, libsdl2-dev 2.30.0 installed):

```
$ cargo run -- --demo --instant --screenshot ../../docs/workbench/C4-TIMELINE-DEMO.png --exit-after 300
events applied: 28  bad lines: 0
$ cat fixtures/demo.jsonl | cargo run -- --stream - --screenshot ../../docs/workbench/C4-TIMELINE-STREAM.png --exit-after 300
events applied: 28  bad lines: 0
$ cargo run -- --demo --speed 8 --screenshot ../../docs/workbench/C4-TIMELINE-MID.png --exit-after 200
events applied: 9   bad lines: 0
```

Screenshots (all 480×800 RGBA, final binary):

- `C4-TIMELINE-DEMO.png` — final frame: MIC OFF + "utterance complete",
  hand-off done, YOU/CLEANED/REASONING/TOOLS ok/SERVICES·POWER with reasons.
- `C4-TIMELINE-MID.png` — mid-stream: orange mic dot, level bar partially
  filled, LIVE, dim partial transcript, `REASONING ...` streaming marker.
- `C4-TIMELINE-STREAM.png` — same final frame via the stdin live path.

## Gaps / next steps

- Not built for Redox: OrbClient's *orbital* backend exists but this crate
  links sdl2 on Linux; a Redox build needs the `orbclient` orbital path and
  an on-device trace source (C3's pipeline).
- No live mic/STT feed yet — the app consumes the journal C3 produces; wiring
  to a running `op6d` is the integration step.
- Wake-lock deadline renders as raw epoch µs — readable, but a relative
  countdown would be nicer.
- Window resize is not re-flowed mid-frame beyond the per-frame recompute
  (render reads width/height every frame, so it self-heals; untested on X11).
