# E1 — Wyoming probe (host interop + offline aarch64 Redox cross-build)

**Initial verdict: HOST_ECHO_INTEROP_PASS + REDOX_BUILD_PASS.** Shared cache write exclusion was not established in the initial command. One real
gap in the shared cross-build recipe is documented under *Recipe gap* (not patched).

- Branch: `wyoming-probe`, forked from `main` @ `77a441e2c8508d8c83a2c4788279bf319f30ad07`
  (local `main` == `origin/main`; worktree was clean at `c115846` = `codex/wy-harness-modes`).
- Worktree: `$HOME/Developer/experiments/redox-oneplus6-wy` (all git calls with
  `git -c core.excludesFile=/dev/null`).
- Session window: 5:35–5:45 AM PT (12:35–12:45 UTC), 2026-10-25.

## What was added (only these)

| File | Lines | Purpose |
| --- | --- | --- |
| `apps/wyoming-probe/Cargo.toml` | 13 | standalone package, path dep on `crates/wyoming` |
| `apps/wyoming-probe/src/main.rs` | 142 | minimal native Wyoming client probe |
| `apps/wyoming-probe/interop.sh` | 46 | host smoke driver against the EXISTING Python reference server |
| `apps/wyoming-probe/.cargo/config.toml` | 12 | hermetic vendored-source replacement (relative path, same pattern as `apps/sandbox-demo/.cargo/config.toml`) |

Scratch (untracked, under `out/wyoming-probe/`): host build, vendor dir, cargo-home
shims, logs, the aarch64 binary. No binaries committed.

The app reuses the existing crate API only — `wyoming::connect`, `Client::new`,
`Client::send`, `send_ping`, `recv`, `close`, and the typed events
`Describe` / `Info` / `WakeProgram` / `Pong` from `crates/wyoming::events`.
`crates/wyoming` was **not** modified.

Probe flow (against whatever Wyoming service is on the given address):
`describe` → peer echo re-parsed with `Describe::from_event`; typed `info`
(one wake program) → echo re-parsed with `Info::from_event` and compared;
`ping {text: <nonce>}` → reference `pong` must copy the nonce back verbatim;
then `Client::close()` so the peer sees EOF.

## 1. Branch setup

```
git -c core.excludesFile=/dev/null checkout -b wyoming-probe 77a441e
# Switched to a new branch 'wyoming-probe'
# head 77a441e2c8508d8c83a2c4788279bf319f30ad07   (12:35:39 UTC)
```

## 2. Host interop smoke — PASS

Host build (offline, local registry cache only):

```
cd apps/wyoming-probe
PATH=$HOME/.cargo/bin:$PATH cargo build --release --offline --target-dir ../../out/wyoming-probe/host
# Finished `release` profile [optimized] target(s) in 0.36s
```

Interop driver `apps/wyoming-probe/interop.sh` starts the **existing** proven
reference server `crates/wyoming/tests/pyinterop_server.py` exactly the way
`crates/wyoming/tests/pyinterop.rs` starts it — `PYTHONPATH = <reference checkout>:
crates/wyoming/tests/pyref_meta` (the `pyref_meta` dist-info is required because
`wyoming/version.py` reads it through `importlib.metadata`; a bare PYTHONPATH fails
with `PackageNotFoundError: No package metadata was found for wyoming`).
Reference checkout: `$HOME/Developer/forks/op6-refs/wyoming` @ 08a0c25,
wyoming 1.11.0. No new server was written.

```
bash apps/wyoming-probe/interop.sh        # rc 0
```

Actual output (`out/wyoming-probe/interop.log`):

```
REF $HOME/Developer/forks/op6-refs/wyoming/wyoming/__init__.py 1.11.0
PROBE connect ok 127.0.0.1:35443
PROBE frame idle -> idle: sent describe
PROBE frame idle -> idle: received describe
PROBE describe ok (round-tripped through the peer)
PROBE frame idle -> idle: sent info
PROBE frame idle -> idle: received info
PROBE info ok (typed Info round-tripped, 1 wake program)
PROBE frame idle -> awaiting-pong: sent ping
PROBE frame awaiting-pong -> idle: received pong
PROBE nonce ok (nonce wy-probe-1792931864-1638340 came back in pong)
PROBE frame idle -> closed: closed
PROBE PASS addr=127.0.0.1:35443 nonce=wy-probe-1792931864-1638340
REF $HOME/Developer/forks/op6-refs/wyoming/wyoming/__init__.py 1.11.0
PORT 35443
SERVED 3
interop: client rc=0 python rc=0 nonce=wy-probe-1792931864-1638340 port=35443
```

**Nonce exchanged: `wy-probe-1792931864-1638340`** (out in `ping.text`, back in
`pong.text`). Python server served 3 frames and exited 0 on clean EOF, which also
proves `Sink::shutdown` delivers EOF.

Note on the echo semantics: `pyinterop_server.py` is an echo server — it answers
`ping` with a real `Pong` (text copied) and echoes every other frame. So the
`describe`/`info` legs prove the typed codec round-trips through the *Python
reference encoder/decoder*, not that a real service answers `describe` with `info`.
The nonce leg is a genuine request/response exchange.

## 3. Offline aarch64-unknown-redox cross-build — PASS

`cargo vendor --offline` (host, no network):

```
cd apps/wyoming-probe
cargo vendor --offline ../../out/wyoming-probe/vendor   # rc 0, 0.107s
# vendored: equivalent foldhash hashbrown indexmap itoa memchr proc-macro2 quote
#           serde serde_core serde_derive serde_json syn unicode-ident zmij
```

Worktree-local `CARGO_HOME` shims (copied from the fork's podman build dir, as the
qemu-demo recipe does):

```
cp -a $HOME/Developer/forks/redox/redox/build/podman/.cargo/bin out/wyoming-probe/cargo-home/bin
```

Container build — **`--network none`**, toolchain and sysroot mounted read-only,
all outputs under `out/wyoming-probe/`:

```
podman run --rm --network none --workdir /mnt/project/apps/wyoming-probe --pids-limit=1024 \
  --volume $HOME/Developer/forks/redox/redox:/mnt/redox:ro \
  --volume $HOME/Developer/forks/redox/redox/build/podman/.rustup:/root/.rustup:ro \
  --volume $HOME/Developer/forks/redox/redox/build/podman/.redoxer:/root/.redoxer \
  --volume out/wyoming-probe/cargo-home:/root/.cargo \
  --volume $PWD:/mnt/project \
  --env PATH=/root/.cargo/bin:/mnt/redox/prefix/aarch64-unknown-redox/sysroot/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
  --env TARGET=aarch64-unknown-redox \
  --env COOKBOOK_TOOLCHAIN=/mnt/redox/prefix/aarch64-unknown-redox/sysroot \
  --env COOKBOOK_SYSROOT=/mnt/redox/prefix/aarch64-unknown-redox/sysroot \
  redox-base /mnt/redox/target/release/cookbook_redoxer build --release --offline \
  --target-dir /mnt/project/out/wyoming-probe/redox --bins
# rc 0 — Finished `release` profile [optimized] target(s) in 11.87s
# wyoming v0.1.0 and wyoming-probe v0.1.0 compiled for aarch64-unknown-redox
```

Image `localhost/redox-base` = `9ca3618050a4` (present). Logs:
`out/wyoming-probe/redox-build.log` (first attempt, see gap below) and
`out/wyoming-probe/redox-build2.log` (passing attempt).

Artifact evidence:

```
out/wyoming-probe/redox/aarch64-unknown-redox/release/wyoming-probe
  ELF 64-bit LSB executable, ARM aarch64, version 1 (SYSV), statically linked, not stripped
  sha256 5048c585d30ceb1fc9df6cada60a88bbfa49ba1a638e5a0cafa8c180b2be3728   (1,191,520 bytes)
```

Host binary (for the interop run, for contrast):
`out/wyoming-probe/host/release/wyoming-probe`, x86-64 PIE,
sha256 `ef0400e3cf8188ad0f35886bafee613f55cd7e826082aacb2b01d43f29aefd60`.

No ISO staging, no mounting, no guest boot, no phone: the aarch64 binary is a real
cross-compiled artifact, **not executed on Redox** — that is the next gate, not this one.

## Recipe gap (shared file — NOT patched, reported for separate review)

`scripts/qemu-demo/build.sh` (shared, untouched) mounts `.rustup`, the `.cargo/bin`
shims and the sysroot, but **not** the redoxer toolchain cache
`$HOME/Developer/forks/redox/redox/build/podman/.redoxer`. Reproduced exactly:

```
# first attempt, without the .redoxer mount, --network none:
redoxer: downloading toolchain from "https://static.redox-os.org/toolchain/x86_64-unknown-linux-gnu/aarch64-unknown-redox"
curl: (6) Could not resolve host: static.redox-os.org
redoxer cargo: unable to init toolchain
```

The cached toolchain is at `build/podman/.redoxer/aarch64-unknown-redox/toolchain`;
mounting it at `/root/.redoxer` makes the build fully offline (rc 0, 11.87s). So the
qemu-demo recipe as written is only offline-capable if the container image itself
carries `/root/.redoxer` — this image does not (`/root/.cache` and `/root/.redoxer`
are absent in a bare `redox-base` run). Suggested follow-up for whoever owns
`scripts/qemu-demo/build.sh`: add `--volume "$redox/build/podman/.redoxer":/root/.redoxer`.
No bug was found in `crates/wyoming` itself.

## Reproduce

```
cd apps/wyoming-probe
PATH=$HOME/.cargo/bin:$PATH cargo build --release --offline --target-dir ../../out/wyoming-probe/host
bash interop.sh
cargo vendor --offline ../../out/wyoming-probe/vendor
# then the podman run block above (with the .redoxer mount)
```

## Round-2 re-verification (5:41–5:44 AM PT / 12:41–12:44 UTC)

Re-ran the interop from the committed tree — reproduced independently:

```
bash apps/wyoming-probe/interop.sh        # rc 0
PROBE PASS addr=127.0.0.1:33401 nonce=wy-probe-1792932087-1698244
SERVED 3
interop: client rc=0 python rc=0 nonce=wy-probe-1792932087-1698244 port=33401
```

Second nonce exchanged: `wy-probe-1792932087-1698244` (log `out/wyoming-probe/interop-rerun.log`).
Redox artifact re-hashed from disk, unchanged:
`5048c585d30ceb1fc9df6cada60a88bbfa49ba1a638e5a0cafa8c180b2be3728`.

Branch state at close: `wyoming-probe` @ `ecb5863d` == `origin/wyoming-probe`, tree clean,
no binaries committed. Note for whoever integrates: local `main` advanced during this
run to `1970b3b` ("docs: record active Wyoming and Spark owners after verified TCP
gate", 2 commits ahead of this branch's base `77a441e`, which is still an ancestor —
fast-forward/merge path is clean). `main` was never pushed from this worktree.

## Codex review closure — 6:27 AM PT

Luna reviewed full code and identified missing bounds, echo-fixture scope and writable
shared cache exposure. Original .redoxer mount was writable; no-fork-write claim is
withdrawn for that run. The local branch is divergent from main, NOT fast-forward.

The 10-minute repair worker stopped at its exact6:24:18AM PT deadline, leaving
uncommitted fixes. Codex completed the narrow repair: negative fixtures now exec
Python rather than tracking an exited intermediate shell; ownership checks inspect
only this test's PIDs. No unrelated workers were killed. The initial failing-client
negative had waited120s; it was not accepted as a prompt bounded result.

Client now uses numeric bounded connect and mandatory per-operation read/write
timeouts, plus frame budget and cooperative deadline. Unsupported primitives fail
closed. A hard whole-process/VM watchdog remains required for partial-frame stalls.
Interop timeout has kill-after, bounds server exit/reap and requires both statuses
to pass. It is specifically an echo-fixture codec/nonce probe, not a normal service
describe->info handshake.

Final root verification under out/wyoming-probe/: host-final-root.log (offline host
compile), interop-final-root.log (real fixture, client0/server0), negatives-final-root.log
(silent peer3s failure, EOF failure, failed client exits promptly, no owned orphans).
server-failure-root.log additionally injects an owned reference server exit143 with
a successful client; the harness rejects rc143. The initial malformed JSON attempt
was ignored by the reference and is not counted as server-failure proof.

One final explicitly Codex-authorized source-matching cross-build after the worker's
earlier readonly-cache build: redox-final-root.log, rc0,1.95s, --network none and
--offline with .rustup, sysroot and .redoxer ALL read-only. Private cargo-home/target.
Final aarch64 ELF sha256 a0ddc407ef8e85aff0260ccea2788b429ce5d22f44158e639e38555817167fcb.
Still no image staging, mount, guest execution or physical phone proof.
