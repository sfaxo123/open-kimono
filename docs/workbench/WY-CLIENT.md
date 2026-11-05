# WY-CLIENT — Wyoming socket transport + client state machine (WAVE2 §1.1 steps 1–3)

Worker: WAVE2-WY (lane 5090), branch `net/wyoming-client`, based on clean main `33f288d`.
Run date: 2026-10-24 8:02 PM PDT (2026-10-25T03:02Z).
Scope executed: `docs/design/WAVE2-NETSANDBOX.md` §1.1 steps **1–3 only** (transport adapter,
client state machine, loopback host test). **Step 4 (QEMU user-net gate) deliberately NOT
started** — it is a later gate. No phone, no push, no QEMU.

## 1. What was built

| File | Change |
|---|---|
| `crates/wyoming/src/transport.rs` | NEW — `Sink` trait (blanket `Read + Write`), `Framed<S>` codec wrapper (`read_frame`/`write_frame`), `connect()` TCP adapter (`TCP_NODELAY`). Codec itself untouched. |
| `crates/wyoming/src/client.rs` | NEW — `ClientState` enum, `Transition`/`TransitionKind` journal records, `Client<S>` state machine (ping/pong, audio start/chunk/stop, transcript, speech audio, wake detection, auto-pong for service pings, EOF→Closed). |
| `crates/wyoming/src/error.rs` | +1 variant `InvalidState(String)` (out-of-order command / closed-connection action), with Display/Clone/PartialEq arms. |
| `crates/wyoming/src/lib.rs` | module wiring + re-exports (`Client`, `ClientState`, `Transition`, `TransitionKind`, `connect`, `Framed`, `Sink`). |
| `crates/wyoming/tests/loopback.rs` | NEW — loopback gate over `127.0.0.1` against a minimal **in-Rust** fake Wyoming server (thread + `TcpListener`). |

No new dependencies (Cargo.toml/Cargo.lock untouched — offline build stays hermetic).

## 2. Design notes

- **Codec stays pure.** `encode_into`/`decode` were already generic over `Write`/`BufRead`,
  so `transport.rs` only *wraps* them: `Framed<S>` holds a `BufReader<S>` (frame reads mix
  line + exact-length block reads, so buffering is mandatory) and writes straight through
  `get_mut()`. Nothing in `event.rs`/`events/` changed — the golden cross-check in
  `wyoming-crosscheck.md` remains the byte-identity proof.
- **`Sink` is a trait alias** (`trait Sink: Read + Write` + blanket impl), so `TcpStream`,
  `UnixStream`, and the in-memory duplex mocks in tests all flow through the identical
  `Framed`/`Client` code path.
- **State machine is explicit**: `Idle | AwaitingPong | StreamingAudio | AwaitingResponse |
  ReceivingSpeech | Closed`. Out-of-order commands (`audio-chunk` before `audio-start`,
  double `audio-start`, anything after `Closed`) are rejected with
  `Error::InvalidState` and write nothing to the wire. Unknown / out-of-vocabulary frames
  never kill the connection: they are journaled `Received` with no state change.
- **Every transition returns a journalable record** (`Transition { from, to, kind }` with
  the full `Event` in `kind`) — the B1 tracejournal hook. `Transition::summary()` gives a
  one-line form (`idle -> awaiting-pong: sent ping`) for cheap journaling. A service `ping`
  yields **two** transitions (`Received` + `AutoPong`) since the client answers keepalives
  automatically, copying `text` like the Python handler.
- **Loopback gate is Python-free by design** (per the WAVE2-WY brief): the fake server
  speaks with the same crate codec (`encode_into`/`decode`), whose byte-identity to Python
  `write_event`/`read_event` is already proven by the 59 golden fixtures
  (`tests/golden_fixtures.rs`, `docs/workbench/wyoming-crosscheck.md`). The loopback test
  asserts the full 14-step transition journal, payload integrity (2048-byte speech chunk,
  1024 bytes streamed up), transcript text, auto-pong text copy, and clean EOF→Closed.

## 3. Commands + raw tails

### Before (baseline at `33f288d`, before any edit)

```
$ cargo test --offline
     Running unittests src/lib.rs
test result: ok. 0 passed; 0 failed; ...
     Running tests/framing.rs
test result: ok. 18 passed; 0 failed; ...
     Running tests/golden_fixtures.rs
test result: ok. 2 passed; 0 failed; ...
     Running tests/typed_events.rs
test result: ok. 39 passed; 0 failed; ...
   Doc-tests wyoming
test result: ok. 0 passed; 0 failed; ...
```
Baseline total: **59 passed, 0 failed**.

### After (this commit)

```
$ cargo test --offline
     Running unittests src/lib.rs
test result: ok. 13 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
     Running tests/framing.rs
test result: ok. 18 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
     Running tests/golden_fixtures.rs
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
     Running tests/loopback.rs
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.04s
     Running tests/typed_events.rs
test result: ok. 39 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
   Doc-tests wyoming
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```
After total: **74 passed, 0 failed** (+15: 10 client mock-sink unit tests, 3 transport unit
tests incl. a mock-sink and a TCP round-trip, 2 loopback integration tests).

New test names (unit): `client::tests::{ping_pong_cycle, audio_stream_then_transcript,
speech_audio_stream, wake_detection_journaled_without_state_change, service_ping_gets_auto_pong,
out_of_order_commands_rejected, clean_eof_closes, close_is_idempotent,
unknown_and_error_frames_journal_only, transitions_are_journalable_summaries}`,
`transport::tests::{mock_sink_roundtrip, mock_sink_multiple_frames, tcp_loopback_roundtrip}`.
Integration: `loopback::loopback_client_against_fake_server`,
`loopback::loopback_wire_bytes_match_golden_encoder`.

### Build

```
$ cargo build --offline
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.01s
```

### Flake check (loopback over real TCP)

```
$ for i in $(seq 10); do cargo test --offline --test loopback; done
test result: ok. 2 passed; 0 failed; ...   (×10)
```
10/10 green — the fake server is lock-step (blocking `recv` per expected frame), so there
is no WouldBlock race.

## 4. Boundaries honored

- Touched only `crates/wyoming/**` and this file, in this worktree only.
- `$HOME/Developer/forks` not touched (read-only respected).
- No phone, no push, no QEMU. Step 4 (QEMU user-net serial gate) left for its own gate.

## 5. Next (not done here)

- QEMU user-net gate (§1.1 step 4): guest client → host fake Wyoming server at `10.0.2.2`,
  one Ping/Pong round-trip asserted in the e1 QEMU serial log.
- Redox TCP maturity for outbound connections is still unproven (design doc risk note);
  the loopback gate only proves the host-side stack path.
