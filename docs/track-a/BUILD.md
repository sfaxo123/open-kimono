# Track A — cross-build recipe (A0, verified 2026-11-02)

The working recipe for building Track A binaries for the OnePlus 6 and running
them there. Everything below was run on the 3090 (`hostname c`) against the
phone at `ssh oneplus`, and the outputs are the real ones.

## The phone

| fact | value |
|---|---|
| kernel | `7.1.0-rc1-sdm845` (`/proc/sys/kernel/osrelease`) |
| libc | **musl** — `/lib/ld-musl-aarch64.so.1`, distro `Nura edge` |
| arch | aarch64 (`qcom,sdm845`) |
| user | `user` (uid 10000), no root needed for anything here |
| Rust toolchain | none — binaries are cross-built here and copied over |
| install root | `~/agentic/` only (`bin/`, `fixtures/`, `out/`) |

Because there is no toolchain on the phone, the binary must be **statically
linked**: a dynamically linked musl binary built against a different musl
version dies there with `not found` or a symbol error.

## Recipe (host: 3090, Ubuntu 24.04)

```bash
# one-time
rustup target add aarch64-unknown-linux-musl

# every build: rust-lld from the toolchain, static musl, no C cross-compiler needed
export PATH="$HOME/.cargo/bin:$HOME/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/lib/rustlib/x86_64-unknown-linux-gnu/bin:$PATH"
export RUSTFLAGS="-C linker=rust-lld -C target-feature=+crt-static"
cargo build --release --offline --target aarch64-unknown-linux-musl
```

Why each piece:

- `-C linker=rust-lld` — the host `cc` links with host `ld`, which does not
  understand the aarch64 musl link flags and dies with
  `unrecognized option '--fix-cortex-a53-843419'`. `rust-lld` ships inside the
  toolchain (`…/lib/rustlib/x86_64-unknown-linux-gnu/bin/rust-lld`), so no
  cross-gcc and no zig are needed.
- `-C target-feature=+crt-static` — no dynamic section at all; verified with
  `readelf -d` → *"There is no dynamic section in this file."*
- `--offline` — the fleet's crates are already in `~/.cargo/registry`; keeps
  builds reproducible.

### The one dependency that needs special treatment: `blake3`

`svcd` depends on `blake3`, whose build script compiles C and looks for
`aarch64-linux-musl-gcc`, which this host does not have:

```
error occurred in cc-rs: failed to find tool "aarch64-linux-musl-gcc"
```

Fix without touching `crates/svcd` (Track A may not edit existing crates):
select blake3's pure-Rust path.

```bash
cargo build --release --offline --target aarch64-unknown-linux-musl --features blake3/pure
```

For crates that depend on `svcd` (like `apps/agentd`), the same feature is
pinned in that crate's own `Cargo.toml` as a direct `blake3` dependency, so
feature unification carries it — no per-command flag to forget.

## Verified runs on the phone

### 1. `apps/agentd` smoke binary

```bash
cd apps/agentd
cargo build --release --offline --target aarch64-unknown-linux-musl
scp target/aarch64-unknown-linux-musl/release/smoke oneplus:~/agentic/bin/smoke
ssh oneplus '~/agentic/bin/smoke'
```

output (real):

```json
{"seq":0,"ts_us":1793628221536781,"service":"agentd-smoke","payload":{"kind":"ag_ui","type":"CUSTOM","name":"track_a.smoke","value":{"arch":"aarch64","hostname":"oneplus-enchilada","libc":"musl","os_release":"7.1.0-rc1-sdm845","target":"aarch64-unknown-linux-musl"}}}
```

Back on the host, the same line is proved to be a journal record:

```bash
ssh oneplus '~/agentic/bin/smoke' | cargo run --offline --bin smoke -- --validate
# valid: 1 journal line(s)
```

Evidence: `docs/track-a/evidence/A0-phone-smoke.trace`.

### 2. `crates/svcd` CLI, unmodified, on the phone

```bash
cd crates/svcd
cargo build --release --offline --target aarch64-unknown-linux-musl --features blake3/pure
scp target/aarch64-unknown-linux-musl/release/svcd oneplus:~/agentic/bin/svcd
scp tests/fixtures/svc.time.countdown_timer.json oneplus:~/agentic/fixtures/
ssh oneplus '~/agentic/bin/svcd simulate ~/agentic/fixtures/svc.time.countdown_timer.json \
  --trace ~/agentic/out/countdown.trace'
```

output (real, tail):

```
svc.time.countdown_timer.json: OK (svc.time.countdown_timer v1.0.0, 3 triggers, 5 actions)   # svcd validate
PASS svc.time.countdown_timer :: five_minute_timer_lifecycle (virtual end 300000 ms, 50 journal events, 6 bus messages)
1 test(s), host time 3621 µs
```

The 50 journal events the phone produced are in
`docs/track-a/evidence/A0-phone-countdown.trace`, and they validate:

```bash
cargo run --offline --bin smoke -- --validate < docs/track-a/evidence/A0-phone-countdown.trace
# valid: 50 journal line(s)
```

### 3. Negative case on the phone: a bad ServiceIR is refused

```bash
python3 - <<'PY'
import json
ir = json.load(open('crates/svcd/tests/fixtures/svc.time.countdown_timer.json'))
ir['triggers'][0]['kind'] = 'telepathy'
json.dump(ir, open('/tmp/bad-ir.json', 'w'), indent=1)
PY
scp /tmp/bad-ir.json oneplus:~/agentic/fixtures/bad-ir.json
ssh oneplus '~/agentic/bin/svcd validate ~/agentic/fixtures/bad-ir.json'; echo "exit=$?"
```

output (real):

```
$HOME/agentic/fixtures/bad-ir.json: ServiceIR rejected (1 error):
  /triggers/0/kind: Enum: unknown variant `telepathy`, expected one of `voice_intent`, `bus_message`, `timer_schedule`, `sensor_threshold`, `ui_action`, `lifecycle_stop`
exit=1
```

## Host tests for A0

```bash
cd apps/agentd && cargo test --offline     # 9 tests, 4 of them negative cases
```

`tests/smoke_jsonl.rs` — the emitted line round-trips into `trace_events::Event`;
truncated line, invented payload kind, missing `payload`, and nameless `CUSTOM`
are each rejected with a named serde error.
`tests/smoke_bin.rs` — runs the real binary: one line out, it validates; a
corrupted line exits 1 with `not a valid journal event` + `unknown variant`;
empty input exits 1 instead of passing vacuously.

## What is NOT proven here

- Nothing about the phone's radios, screen or power — A0 only ran a print and a
  virtual-clock simulation, both under `~/agentic/`, both exited by themselves.
- No TLS: the phone talks to the homelab gateway over plain HTTP on Meshnet, so
  the cross-build needs no TLS stack. If a future step needs HTTPS, this recipe
  has to grow (ring/aws-lc need a C cross-compiler or `pure`-style features).
