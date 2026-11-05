# WY-PYINTEROP — close()-EOF fix + Python-reference interoperability gate

Worker: WAVE2-WY (lane 5090), branch `wy/close-eof-and-pyinterop`, based on main `9e0645b`.
Run date: 2026-10-24 ~8:43 PM PDT (2026-10-25T03:43Z).
Scope: Luna review follow-up — (1) `Client::close()` EOF regression, (2) real-Python
interoperability gate closing the acceptance gap from `WY-CLIENT.md` (golden fixtures
alone were not enough). Ownership honored: only `crates/wyoming/**` + this file.

## 1. close() EOF regression — red then green

**Bug** (`src/client.rs:136` pre-fix): `close()` only flipped `state = Closed` and kept
the `Framed`/socket alive, so the peer observed EOF only when the `Client` was dropped.

**Red** — commit `5e0b0b3`, test `close_signals_eof_to_peer_while_client_lives`
(`tests/loopback.rs`): bounded (server-side read timeout 1.5 s), asserts the peer sees
`Ok(0)` EOF while the Client is still alive. Raw tail:

```
$ cargo test --locked --manifest-path crates/wyoming/Cargo.toml --test loopback close_signals_eof
peer did NOT observe EOF while client lives: Resource temporarily unavailable (os error 11)
test result: FAILED. 0 passed; 1 failed; ... finished in 1.53s
```

**Fix** — commit `1232100`: `Sink` gains `shutdown()` (default no-op for in-memory
mocks); `TcpStream`/`UnixStream` implement it with `Shutdown::Both`. `Client::close()`
now flushes + shuts the transport down before returning the `Closed` transition; the
clean-EOF path in `recv()` releases our side too. Public API semantics preserved:
`close()` keeps its signature, journalable `Transition` return, and idempotence
(`closed -> closed`); closed/out-of-order rejection unchanged. Trade-off noted: `Sink`
drops the blanket impl (explicit one-line impls for mocks now) — no downstream users.

**Green** — companion unit tests `close_shuts_down_transport` and
`eof_also_shuts_down_transport` (MockSink records `shutdown_called`):

```
$ cargo test --locked --manifest-path crates/wyoming/Cargo.toml
unit 15 | framing 18 | golden 2 | loopback 3 | typed 39 | doc 0  = 77 passed, 0 failed
```

## 2. Python-reference interoperability gate

**Reference**: `$HOME/Developer/forks/op6-refs/wyoming` @ `08a0c25` (wyoming 1.11.0),
imported via `PYTHONPATH` — the fork is read-only policy, so no `pip install -e` into it.
The reference's `wyoming/version.py` calls `importlib.metadata.version("wyoming")`, so a
minimal committed dist-info (`tests/pyref_meta/wyoming-1.11.0.dist-info/METADATA`) is
committed to satisfy the version lookup under plain PYTHONPATH. Protocol code is the
untouched reference.

**Environment**:
- `python3 --version` → **Python 3.12.3** (system)
- server cmd: `python3 crates/wyoming/tests/pyinterop_server.py`
  with `PYTHONPATH=$HOME/Developer/forks/op6-refs/wyoming:$HOME/Developer/experiments/redox-oneplus6-wy/crates/wyoming/tests/pyref_meta`
- server behavior: reference `read_event`/`write_event`; `ping` → reference `Pong`
  (text copied, like `PingHandler`); every other event echoed byte-faithfully.

**Gate test**: `crates/wyoming/tests/pyinterop.rs` — spawns the Python server on
127.0.0.1, connects the Rust `Client`, sends all 59 golden-fixture events, asserts each
comes back logically identical (ping→pong with text copy), asserts ≥59 exchanged per
direction, then `close()` → Python `read_event` sees clean EOF → `SERVED 59` → exit 0.

### Transcript (cargo test --test pyinterop -- --nocapture, 2026-10-24)

```
pyinterop: python3 Python 3.12.3
pyinterop: server cmd: python3 ".../crates/wyoming/tests/pyinterop_server.py" (PYTHONPATH=$HOME/Developer/forks/op6-refs/wyoming:.../tests/pyref_meta)
pyinterop: sent audio-start            -> got audio-start
pyinterop: sent audio-chunk            -> got audio-chunk
pyinterop: sent audio-chunk            -> got audio-chunk
pyinterop: sent audio-stop             -> got audio-stop
pyinterop: sent voice-started          -> got voice-started
pyinterop: sent voice-stopped          -> got voice-stopped
pyinterop: sent ping                   -> got pong
pyinterop: sent ping                   -> got pong
pyinterop: sent pong                   -> got pong
pyinterop: sent error                  -> got error
pyinterop: sent error                  -> got error
pyinterop: sent timer-started          -> got timer-started
pyinterop: sent timer-started          -> got timer-started
pyinterop: sent timer-updated          -> got timer-updated
pyinterop: sent timer-cancelled        -> got timer-cancelled
pyinterop: sent timer-finished         -> got timer-finished
pyinterop: sent user-event             -> got user-event
pyinterop: sent user-event             -> got user-event
pyinterop: sent played                 -> got played
pyinterop: sent transcript             -> got transcript
pyinterop: sent transcript             -> got transcript
pyinterop: sent transcribe             -> got transcribe
pyinterop: sent transcribe             -> got transcribe
pyinterop: sent transcript-start       -> got transcript-start
pyinterop: sent transcript-chunk       -> got transcript-chunk
pyinterop: sent transcript-stop        -> got transcript-stop
pyinterop: sent detection              -> got detection
pyinterop: sent detection              -> got detection
pyinterop: sent detect                 -> got detect
pyinterop: sent detect                 -> got detect
pyinterop: sent not-detected           -> got not-detected
pyinterop: sent synthesize             -> got synthesize
pyinterop: sent synthesize             -> got synthesize
pyinterop: sent synthesize             -> got synthesize
pyinterop: sent synthesize-start       -> got synthesize-start
pyinterop: sent synthesize-chunk       -> got synthesize-chunk
pyinterop: sent synthesize-stop        -> got synthesize-stop
pyinterop: sent synthesize-stopped     -> got synthesize-stopped
pyinterop: sent recognize              -> got recognize
pyinterop: sent intent                 -> got intent
pyinterop: sent intent                 -> got intent
pyinterop: sent not-recognized         -> got not-recognized
pyinterop: sent intents-start          -> got intents-start
pyinterop: sent handled                -> got handled
pyinterop: sent not-handled            -> got not-handled
pyinterop: sent handled-start          -> got handled-start
pyinterop: sent handled-chunk          -> got handled-chunk
pyinterop: sent handled-stop           -> got handled-stop
pyinterop: sent run-pipeline           -> got run-pipeline
pyinterop: sent run-pipeline           -> got run-pipeline
pyinterop: sent run-satellite          -> got run-satellite
pyinterop: sent pause-satellite        -> got pause-satellite
pyinterop: sent streaming-started      -> got streaming-started
pyinterop: sent streaming-stopped      -> got streaming-stopped
pyinterop: sent satellite-connected    -> got satellite-connected
pyinterop: sent satellite-disconnected -> got satellite-disconnected
pyinterop: sent describe               -> got describe
pyinterop: sent info                   -> got info
pyinterop: sent audio-chunk            -> got audio-chunk
pyinterop: exchanged 59 sent / 59 received; python EOF + exit 0
test python_reference_interop_roundtrip ... ok
test result: ok. 1 passed; 0 failed; ... finished in 0.08s
```

59 sent / 59 received (≥59 required); both `ping`s answered with `pong` copying `text`;
every other event round-tripped logically identical through the Python codec; the
Python server exited 0 after observing EOF from `Client::close()` (which also proves
the §1 fix against a real peer).

## 3. Acceptance gates

```
$ cargo test --locked --manifest-path crates/wyoming/Cargo.toml                # debug
unit 15 | framing 18 | golden 2 | loopback 3 | pyinterop 1 | typed 39 | doc 0 = 78 passed, 0 failed
$ cargo test --locked --release --manifest-path crates/wyoming/Cargo.toml      # release
unit 15 | framing 18 | golden 2 | loopback 3 | pyinterop 1 | typed 39 | doc 0 = 78 passed, 0 failed
```

Full workspace build (all 12 crates, host): **all OK**. Note: 7 crates have no committed
Cargo.lock, so `--locked` refuses to build them (pre-existing repo state, not caused by
this branch); they build green with `--offline` alone. `crates/wyoming` itself is
`--locked` clean in debug and release.

Test counts: 74 (main `9e0645b`) → 77 (close fix) → 78 (+pyinterop gate).

## 4. Commits (red-then-green evidence)

- `5e0b0b3` test(wyoming): RED regression — close() must signal EOF to peer while Client lives
- `1232100` fix(wyoming): GREEN — close() shuts the transport down so the peer sees EOF
- (this commit) test(wyoming): pyinterop gate vs Python reference + this evidence doc
