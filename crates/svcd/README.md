# svcd — deterministic Service IR executor (task B4)

Implements the subset of `docs/design/SERVICE-IR.md` (D1) needed for its
countdown-timer and battery-alert worked examples. A ServiceIR is validated,
proven in a virtual-time simulator, then runs with **no LLM per trigger**.
Every step is a `trace-events` journal record.

```rust
let ir = svcd::load(json_text)?;                       // precise errors: JSON Pointer + code
let reports = svcd::sim::simulate(&ir);                // svc:simulate — embedded tests, virtual clock
let mut live = svcd::Svcd::new(my_host, svcd::Config::default());
svcd::sim::admit(&mut live, ir)?;                      // install only if its tests pass
live.utterance("timer for 5 minutes", None);           // zero-LLM template match → DAG
live.advance_to(300_000);                              // timers fire; nothing sleeps
for e in live.journal().iter() { /* open-kimono view */ }
```

CLI (dev host): `svcd validate <ir.json>...`, `svcd simulate <ir.json> [--trace out.trace]`.

## What is implemented

| D1 section | svcd |
|---|---|
| §2 schema | Typed serde IR (`deny_unknown_fields` exactly where the schema has `additionalProperties:false`) + hand-written pattern/range/length checks + semantic checks (references, DAG, expressions, capabilities, scheme deps, effect-class coherence, lifecycle guard, state schema, test events). All errors are reported, each with a JSON Pointer. `jsonschema` is a **dev-dependency oracle** only: the D1 schema and svcd agree on 21 schema-level mistakes (`tests/schema_conformance.rs`). |
| §3.1 fast path | ha-intents sentence templates (`[opt|alt]`, `(a|b)`, `{slot}`, `{list:slot}`), typed slots, priority order; no match → `ovos.intent.fallback`. Triggers: `voice_intent`, `timer_schedule` (countdown, interval), `sensor_threshold` (edge + hysteresis + debounce + cooldown), `ui_action`, `bus_message`, `lifecycle_stop`. |
| §3.2 loop | guard → cooldown → wakeup admission → duplicate occurrence → claim run → `handler.start` → DAG walk (FIFO over `next` edges with `when_cel`) → state-schema check → atomic commit → post-commit flush of irreversible effects → exactly one `handler.complete`/`handler.error`. Retries per `retry.max_retries` (immediate). |
| §4 transactions | Copy-on-write overlay + staged UI/bus outbox; `reversible_compensated` → LIFO compensation on abort; `committed_irreversible` deferred to after commit, each ledgered before the next runs. Ledger: `claimed → running → completed/failed/unknown` (terminal immutable), `UNIQUE(service, idempotency_key)`, BLAKE3 keys from `idempotency_key_template` (or trigger/occurrence/action). JSON export/import for restarts. |
| §5 simulator | Same executor, `SimHost` records effects, virtual clock from `initial_env`, checks `final_state_subset`, ordered `expected_bus_emissions`, `expected_ui_surfaces`, `budget_assertions`, and that no `gen_ai` span was journaled. `admit()` is the install gate. |
| §7 minimal | Budgets enforced: `max_wakeups_per_minute`, `memory_limit_mb`, `wasm_fuel_per_invocation`, `network_egress_bytes_max`, `max_wakelock_sec` → service stopped, reason on journal + bus (`svc.budget.exceeded`), schemes powered down, grants revoked, later triggers dropped. Power by capability lifetime: schemes on only while their grant is alive (`until_deadline` ↔ pending timer, `while_visible` ↔ surface, `demand_per_action` ↔ the call). Outputs > 4 KiB are journaled as a hash marker. |

Seams: `ActionHost` (dialogs, Wasm `logic`, compensation) and `CapsBroker`
(grants). Feature `sandbox` provides `SandboxHost`, which runs `logic`
actions on `crates/sandbox` (wasmi): one ephemeral instance per call, fuel =
`wasm_fuel_per_invocation`, args/output through the service's scoped KV
(`args` / `output`) because the sandbox ABI has no call parameters.

Journal mapping: executions are AG-UI runs (`RUN_STARTED`/`RUN_FINISHED`
with `llm_tokens: 0`/`RUN_ERROR`), actions are steps, commits are
`STATE_DELTA`, host calls are tool calls, svcd facts are `CUSTOM` `svc.*`,
bus messages `CUSTOM` `ovos.bus`, A2UI messages `CUSTOM` `a2ui`; lifecycle,
grants and power use `op6` events.

## Interpretations of D1 (where the spec was silent or inconsistent)

- `max_wakeups_per_minute` is a sliding 60 s window allowing `ceil(limit)`
  wakeups; only non-user triggers (timer, sensor, bus) count. A token bucket
  at 0.1/min would fail D1's own battery test.
- The simulator lets the longest `debounce_ms` elapse after the last event
  (D1's battery test expects an alert 5 s after its final sample).
- A dialog needs `audio.playback.tts` or `audio.playback.alarm` (critical
  prefers alarm): D1's timer speaks a normal confirmation with only the alarm cap.
- Surfaces are created on the first `ui_patch` (minimal: no UI until there is
  something to show), seeded from `ui.initial_messages`.
- `ui_action` payloads are accepted in A2UI wire shape (`surfaceId`, `name`,
  `sourceComponentId`) or snake_case.
- svcd-only rules beyond the schema: irreversible actions need an
  idempotency template; dialogs must be irreversible; `pure` only for
  read-only `logic`; compensations cannot be irreversible.

## Tests

```
cargo test --offline                    # 76 tests
cargo test --offline --features sandbox # 79 (adds real-Wasm logic actions)
cargo test --offline --release
```

Both D1 worked examples pass their embedded tests (`tests/examples.rs`).
Negative coverage: 28 malformed-IR rejections with exact pointer + code,
21 + 8 schema-conformance cases, retry/redelivery/duplicate/restart never
replaying an irreversible effect, abort-before-commit compensation, every
budget kind stopping the service, timeouts and refusals not stopping it.
Evidence: `docs/workbench/B4-SVCD-TESTS.txt`, journals `B4-timer.trace`,
`B4-battery.trace` (JSONL; `.jsonl` is gitignored).

## Gaps

- No Redox `svc:` scheme daemon yet (API is `Svcd::{install, list, stop,
  utterance, sensor_sample, ui_action, bus_message, advance_to}` + `sim`);
  needs a `time:` timer source and an event loop.
- Ledger/state are in memory (JSON export only) — no SQLite WAL `state:`.
- Not implemented (rejected as `Unsupported` at validation): `cron_wall` /
  `oneshot_iso` timers, `keyword`/`regex` intents, `sub_service`,
  `post_hook_jq`. Accepted but ignored: `retry.checks`, `state_schema.migrations`,
  `concurrency`, `catchup_policy`/`grace_window_sec` (the virtual clock is
  never late), CPU cluster/quota fields, conflict resolver (§6).
- `content_hash` is format-checked only (D1's examples carry placeholder digests).
- The idempotency key without a template omits canonical args (they are a
  pure function of occurrence + committed state before commit).
- D1's nav example is rejected for two real defects (irreversible speech with
  no idempotency key; bare `{{destination}}`), see `tests/validate.rs`.

## What Redox needs

`std` crate (daemon via relibc). Dependencies: `serde`, `serde_json`,
`serde_path_to_error`, `blake3` (no default features), `trace-events`
(19 crates in the normal tree; release binary 2.1 MB stripped on x86_64).
`aarch64-unknown-redox` is not installed on this host, so the Redox build is
unproven (same gap as B5/K7/K9).
