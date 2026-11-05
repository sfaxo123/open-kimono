# sandbox — user-built services as Wasm on wasmi (task B5)

A service the user asks for is compiled to a WebAssembly module and run on
[wasmi](https://github.com/wasmi-labs/wasmi) 2.0.0. The `op6` host functions
below are its **only** access to the OS. Each one is gated by a grant, and
every call, refusal and stop is recorded as a `HostEvent` for the open-kimono
view (VISION §3, §5). Small fuel and memory budgets keep services minimal
(VISION §8).

```rust
let sandbox = Sandbox::new();
let spec = ServiceSpec::new("user.hello", Grants::of(&[Capability::TraceEmit, Capability::Log]));
let mut svc = sandbox.load(&wasm_bytes, spec, MemKv::new())?;
svc.invoke("run")?;                 // fresh fuel budget per call
for e in svc.drain_events() { /* journal / UI */ }
```

## ABI (`op6` import module)

Pointers and lengths are `i32` offsets into the module's exported `memory`.

| Import | Signature | Grant | Returns |
|---|---|---|---|
| `log` | `(level, ptr, len)` | `log` | — (level 0 error, 1 warn, 2 info, ≥3 debug) |
| `trace_emit` | `(name_ptr, name_len, payload_ptr, payload_len) -> i32` | `trace.emit` | `0`; name must be `[a-z0-9._-]+` |
| `timer_after` | `(ms, token) -> i32` | `timer.after` | `0`, or `-2` if `max_pending_timers` are already pending |
| `kv_get` | `(key_ptr, key_len, out_ptr, out_cap) -> i32` | `kv.get` | full value length (writes `min(len, out_cap)`), or `-1` not found |
| `kv_set` | `(key_ptr, key_len, val_ptr, val_len) -> i32` | `kv.set` | `0`, or `-2` if the namespace quota would be exceeded |

Exports: `memory` (needed by any host call that passes data), entry points
`() -> ()` called with `Service::invoke`, and `on_timer(token: i32)` called
by `Service::fire_due_timers(now_ms)`.

KV is namespaced by the service id the host assigned; a guest cannot name
another namespace (`tests/grants.rs::kv_is_scoped_per_service_on_a_shared_backend`).

## Refusal and stop semantics

- **Load time**: invalid bytes, any import outside the five `op6` functions
  (WASI included), a start function, or initial memory/table over the cap are
  refused. `Service::requested_capabilities()` lists what the module could ask
  for, so the UI can show it before anything runs.
- **Contract violations trap and stop the service**: calling a function
  without a live grant (not granted, expired, revoked), a bad pointer, data
  over a size limit (checked before the host allocates), invalid UTF-8.
  The grant check runs before any side effect. A stopped service is never
  re-entered (`SandboxError::Stopped`): its state may be inconsistent.
- **A due timer is grant-gated too**: firing one re-enters the guest, so it
  needs a live `timer.after` grant. With that grant revoked or expired the
  `on_timer` callback is not delivered — the firing is refused (recorded as a
  `Denied` event) and stops the service, before any guest code runs (S-1).
  A poll with nothing due — no timers, or all future-dated — is not a
  capability use: the grant is not checked, nothing is recorded, the service
  keeps running (finding F-3; guard tests in `tests/timers.rs`).
- **Runtime conditions a correct service can meet** are return codes
  (`-1` not found, `-2` quota).
- `memory.grow` past the cap returns `-1` to the guest (spec behaviour) and
  records `ResourceDenied`; it does not stop the service.

## Limits (`Limits::default()`)

| Limit | Default |
|---|---|
| `fuel_per_call` | 1,000,000 (≈ instructions; refilled per call) |
| host call cost | 100 fuel + 1 per byte moved |
| `max_memory_bytes` | 1 MiB (16 pages); 1 memory, 1 table, 1 instance |
| `max_table_elements` | 1024 |
| `max_message_bytes` / `max_trace_name_bytes` | 1024 / 64 |
| `max_kv_key_bytes` / `max_kv_value_bytes` / `kv_quota_bytes` | 128 / 4096 / 64 KiB |
| `max_pending_timers` / `max_timer_ms` | 8 / 24 h |

The engine compiles eagerly so a call's fuel use does not depend on whether a
function was called before.

## Example service

`guests/hello-rs` is a `no_std` Rust cdylib (1.4 KB, one memory page) that
uses every import. Build: `cd guests/hello-rs && cargo build --release`
(target and stack size come from its `.cargo/config.toml`).
`tests/rust_guest.rs` builds and runs it; `examples/run_hello.rs` +
`tests/guest_host.rs` drive the same guest through the host runner flow
(full-grant run + TraceEmit-refusal assertion).

## Tests

```
cargo test --offline            # 39 tests; needs the wasm32-unknown-unknown target for rust_guest + guest_host
cargo test --offline --release  # exercises wasmi's tail-call dispatch path
cargo build --offline --no-default-features --target aarch64-unknown-uefi --release  # no_std check
```

Evidence: `docs/workbench/B5-SANDBOX-TESTS.txt`.

## What Redox needs

- **No relibc for the core.** The crate and its whole dependency tree
  (wasmi, wasmparser, spin, libm, bitflags) build `no_std` + `alloc`, checked
  on `aarch64-unknown-uefi`. Only a global allocator is required.
- **A Redox daemon would use `std`** (default feature) through relibc on
  `aarch64-unknown-redox`. That target is not installed on this host (same
  gap as K7/K9), so the Redox build itself is unproven.
- **Dispatch**: wasmi uses tail-call dispatch on aarch64 only in optimizing
  builds; keep the `auto-dispatch` feature (see GOTCHAS) and ship release
  builds.
- **Still to write**: a `sandboxd` scheme daemon that hosts services; a KV
  backend on the filesystem (one directory per service); timers driven by
  Redox's `time:` scheme; `HostEvent` → `trace-events` journal mapping; a
  grant broker that issues `Grants` from the D2 capability design
  (`docs/design/CAPS-SANDBOX.md`, not landed when this was written).
- Float results are not bit-deterministic across engines (wasmi's
  `deterministic` feature is off); irrelevant for routines, relevant if
  services are ever replayed for verification.
