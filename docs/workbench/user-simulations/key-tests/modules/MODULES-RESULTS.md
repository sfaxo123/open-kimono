# Modules — agent-emitted manifest path: results

Run 2026-11-04 on the 3090, scratch clone `~/work/modules-wonder`
(base `main @ bb40adb`). Host-side only: nothing deployed to the
phone, nothing committed — the root session adopts. Proof was the
suites plus one live local `agentd` instance on the 3090 against a
scripted fake brain and a stub `gio` (details below).

## What was built

The agent-emitted path that did not exist: `agentd` now emits
module manifests (MODULE-PROTOCOL v1) as journal events driven by
real state, and kimono renders emitted manifests when present.
This follows the CV1 pattern exactly — kimono renders follow-up
state only from journaled events the listener posts; here kimono
renders modules only from journaled events agentd emits. There is
no side channel, no fetch, and no page data island.

### Event shape

One new journal vocabulary, reusing the existing CUSTOM envelope
(no new trace-events kind, no schema change):

```json
{
  "payload": {
    "kind": "ag_ui",
    "type": "CUSTOM",
    "name": "module_manifest",
    "value": { "manifest": { "...protocol v1 manifest..." }, "inputs": { "...agent-filled values..." } }
  }
}
```

- `manifest` is a complete protocol-v1 object: `id`, `protocol:
  "1"`, `user_purpose` (always present, one sentence), `trigger`,
  `inputs`, `layout` (only `title` / `big_number` / `rows` /
  `action_button`), `actions` (real tool names only), `lifetime`
  with a positive `max_ttl_minutes`. It contains data, never code.
  Production manifests carry no `sample_inputs`.
- `inputs` is the agent-filled value map, keyed by the manifest's
  input names. Values ride beside the manifest, not inside it, so
  the manifest stays a reusable definition and the values are the
  run's real state (the tool result, the session snapshot).
- Kimono side (`render.mjs`): `view.modules[id] = { manifest,
  inputs }`, `view.moduleOrder` keeps first-emission order, latest
  per id wins — a card updates in place when agentd re-emits it.
  This branch is the only writer of `view.modules` (like
  `follow_up_window` is the only writer of `view.followUp`).
  Rendering reuses the existing `renderModuleCard`, so an invalid
  emitted manifest renders the explicit UNSUPPORTED card and all
  strings render as text nodes — unchanged render-side rules.

### When each card is emitted

**Destination card (`destination-card`)** — emitted exactly once
per successful `app_launch` tool execution, from inside the tool
loop after `TOOL_CALL_RESULT`, only when the call was not broker-
gated and the tool actually succeeded. Built from the actual tool
result: `destination` and `app` from the call arguments, `uri`
from the result content (`launch.uri`, the `geo:` URI that was
opened), `run_id` from the current run — the link back to the run.
A refused launch (non-`maps` app, bad arguments) or a gated call
emits nothing; the Rust suite proves the refusal case. Its one
action is `open_maps` → `app_launch { app: "maps", destination }`,
so its button rides the existing plain-language request path
(`open maps and give me directions to <destination>`) through the
broker/confirm rules, never a direct tool call.

**Session card (`session-card`)** — display-only (no actions),
emitted from `SessionStore` state at two moments:

1. After every recorded session turn (`Agent::run`, after
   `record` + `set_repair`): inputs are the snapshot's real
   `session_id`, `turn_count`, `follow_up_window`
   (`open`/`closed`), and `pending` (the stored pending list
   joined, or `none`). Turn 1 emits `turn_count: 1`; the next turn
   of the same session emits `turn_count: 2`, and kimono replaces
   the card in place. A sessionless run emits no session card.
2. On `POST /follow-up` (the listener's window report): the
   report is mirrored into the store (`SessionStore::set_follow_up`,
   new) and the card is re-emitted with the new window state and
   the unchanged turn count — the window change is a session
   change. The `follow_up_window` journal record itself is
   untouched and still drives the mic chip.

Both builders are guarded at emission: the manifest is checked by
`validate_manifest` + `missing_required_inputs` (below) and an
invalid or value-less card is not journaled at all. Session expiry
emits nothing: kimono keeps the last emitted card (TTL/lifetime
enforcement is the lifetime field's declared contract; see Open
items). On session expiry the next turn starts a fresh session and
its first recorded turn re-emits with `turn_count: 1` and the new
session's state.

### Empty state

The static sample island (`eta-card` / `flight-card` / `maps-card`
JSON in `index.html`) is deleted. Before the first
`module_manifest` record, the modules area renders one honest
empty state — `renderModuleEmptyState`: “No modules yet — cards
the agent composes will appear here when it emits them.” Sample
data is never substituted: `app.js` no longer reads any page
island (`parseModuleData` is retained in `modules.mjs` as a pure
parser, unused by the page), and the kimono suite asserts the page
contains neither `module-data` nor the sample card ids.

## Conformance proof

Which method: **reuse of the real validator in a node-side test
against captured emitted manifests** (the brief's first option),
plus an emission-side replica as a guard. Specifically:

- Captured fixtures
  `apps/kimono-ui/fixtures/modules/emitted-destination-card.json`
  and `emitted-session-card.json` are verbatim `module_manifest`
  values agentd journaled during a host proof run (fake brain,
  stub `gio`, session `sess-capture-1`, destination San Francisco
  — the KEY TEST 1 smoke destination). The node test
  `captured agent-emitted manifests validate with the real
  validator` runs the actual `validateModule` from
  `web/modules.mjs` over them, asserts a non-empty
  `user_purpose`, and asserts every required input has a value.
  The neighbouring test renders both captured cards and asserts
  their real values and the destination button's request text.
- `apps/agentd/src/modules.rs::validate_manifest` replicates
  `validateModule` check-for-check (id, protocol, purpose,
  trigger, input types/sources, action tools + `app_launch`
  `args.app == "maps"`, closed block vocabulary, action-button
  references, positive TTL), with matching error wording. Rust
  unit tests pin the builders against it, and the agent checks it
  before journaling any card. The replica is a guard, not the
  proof — the proof is the JS validator over the captures above.

## Example manifests actually emitted (live proof)

Live local instance proof, 2026-11-04 on the 3090: the built
`agentd` binary (scripted fake brain, stub `gio` exit 0, static
dir = `apps/kimono-ui/web`) served `GET /health` 200, `GET /`
200 with no `module-data` in the page; `POST /request`
(“open maps…”, session `sess-live-1`) 200; `POST /follow-up`
(`open`) 200; second session request 200. Reading the journal
back over `GET /events` gave exactly 4 `module_manifest` records,
in this order. Compact form (manifest layout/inputs specs are in
the fixture files; inputs shown in full):

1. `destination-card` — inputs:
   `{"app":"maps","destination":"San Francisco","run_id":"run-1793783117455958-0","uri":"geo:0,0?q=San%20Francisco"}`
2. `session-card` — `{"session_id":"sess-live-1","turn_count":1,"follow_up_window":"closed","pending":"none"}`
3. `session-card` (after the follow-up report) — same, with
   `"follow_up_window":"open"`, turn count still 1.
4. `session-card` (after turn 2) — `"turn_count":2`,
   `"follow_up_window":"open"`.

One full emitted value, verbatim from that journal:

```json
{"manifest":{"id":"destination-card","protocol":"1","user_purpose":"Show the user the destination just opened so they can confirm it and reopen it in one tap.","trigger":{"when":"app_launch_succeeded","context":["last_destination","run"]},"inputs":[{"name":"destination","type":"string","required":true,"source":"context"},{"name":"app","type":"string","required":true,"source":"agent"},{"name":"uri","type":"string","required":true,"source":"tool"},{"name":"run_id","type":"string","required":true,"source":"agent"}],"layout":[{"block":"title","text":"destination"},{"block":"rows","items":[{"label":"App","value":"app"},{"label":"Link","value":"uri"},{"label":"Run","value":"run_id"}]},{"block":"action_button","label":"Open in Maps","action":"open_maps"}],"actions":[{"id":"open_maps","tool":"app_launch","args":{"app":"maps","destination":"destination"}}],"lifetime":{"appears_when":"a destination is opened in Maps","expires_when":"the destination is opened again, changed, or the conversation moves on","max_ttl_minutes":30}},"inputs":{"app":"maps","destination":"San Francisco","run_id":"run-1793783117455958-0","uri":"geo:0,0?q=San%20Francisco"}}
```

The session-card manifest: `id: "session-card"`, purpose “Show
the user the current conversation session state at a glance.”,
`big_number` on `turn_count` + rows for window/pending, no
actions, TTL 120 min. Full text in the captured fixture file.

## Files changed

| Area | File | Change |
|---|---|---|
| agentd | `apps/agentd/src/modules.rs` (new) | Card builders (`destination_card`, `session_card`), `validate_manifest` replica, `missing_required_inputs`, unit tests |
| agentd | `apps/agentd/src/kimono.rs` | `Recorder::module_manifest` CUSTOM writer |
| agentd | `apps/agentd/src/agent.rs` | Destination emission after ungated `app_launch` success; session emission after each recorded turn |
| agentd | `apps/agentd/src/session.rs` | `SessionSnapshot`, `snapshot()`, `set_follow_up()` (+ stored window bit) |
| agentd | `apps/agentd/src/server.rs` | `POST /follow-up` mirrors window into store + re-emits session card |
| agentd | `apps/agentd/src/lib.rs` | `pub mod modules` |
| agentd | `apps/agentd/tests/modules_emit.rs` (new) | 5 emission tests (below) |
| kimono | `apps/kimono-ui/web/render.mjs` | `view.modules`/`moduleOrder`, `moduleEntries()`, `module_manifest` branch (only writer) |
| kimono | `apps/kimono-ui/web/modules.mjs` | `renderModuleEmptyState()`, `renderEmittedModules()` |
| kimono | `apps/kimono-ui/web/app.js` | Modules painted from the journal in `paint()`; island path deleted |
| kimono | `apps/kimono-ui/web/index.html` | Sample island deleted (−238 lines) |
| kimono | `apps/kimono-ui/web/dom.mjs` | `module-manifest` tag label |
| kimono | `apps/kimono-ui/web/style.css` | `.module-empty` (one rule; no redesign) |
| kimono | `apps/kimono-ui/test/modules_test.mjs` | Island test replaced; +5 tests (below) |
| kimono | `apps/kimono-ui/fixtures/modules/emitted-*.json` (new) | Captured emitted values (conformance fixtures) |

Not touched: `brainfix.rs` and the system prompt (BRAINFIX stays
byte-identical — the suite asserts it), `tools.rs` / `launch.rs`
behavior, `trace-events` schema, voiced, any UI layout/styling
beyond the one empty-state rule.

Overlap note (brief's streaming worker): this diff touches the
agent tool loop and the recorded-turn tail in `agent.rs`, the
`/follow-up` handler in `server.rs`, and `session.rs`. If the
other worker's streaming work edits those three files in its
clone, expect textual overlap there on adoption; `modules.rs`,
`kimono.rs` (+12 lines), `lib.rs` (+1) and the whole kimono side
should merge clean. No `tools.rs`/`broker.rs` changes, so their
dispatch work does not collide with the destination emission.

## Tests

New tests:

- agentd `tests/modules_emit.rs` (5): destination card built from
  the tool result (uri + run link asserted) and valid; refused
  `app_launch` emits no card; session card updates across two
  turns (1 → 2, pending `answer_question` after a question);
  sessionless run emits destination but no session card;
  `POST /follow-up` over real HTTP re-emits the session card
  (`closed` → `open`, turn count unchanged).
- agentd `modules.rs` unit tests (3): both builders validate and
  carry every required value; the replica validator refuses
  missing purpose, unknown block, unknown tool by name.
- kimono `modules_test.mjs`: island test **replaced** by “page
  ships no sample island”; new tests for the empty state, captured-
  fixture conformance with the real validator, captured rendering
  (incl. the button request text), journal-driven view with
  latest-per-id replacement, and invalid/markup emitted values
  (unsupported card; markup stays one text node).

Suite counts (all run 2026-11-04 on the 3090):

| Suite | Base | Now | Command |
|---|---|---|---|
| agentd | 118 | **126** (14 lib incl. 3 new unit + 112 integration incl. 5 new) | `cargo test --manifest-path apps/agentd/Cargo.toml` |
| voiced | 56 | **56** (unchanged, untouched) | `cargo test --manifest-path apps/voiced/Cargo.toml` |
| kimono (node) | 32 | **37** (modules 7 → 12) | `node --test apps/kimono-ui/test/*.mjs` |
| kimono (cargo fixtures) | 7 | **7** (unchanged) | `cargo test --manifest-path apps/kimono-ui/Cargo.toml` |

All green, 0 failures. `cargo clippy` on agentd shows no warnings
in the new code (pre-existing lib warnings in `services.rs` etc.
untouched). `cargo fmt --check` was already unclean at base across
the crate, so it was not run as a gate and no formatting sweep was
applied.

## Deviations

- The one existing test that asserted the sample island
  (`the page data island ships exactly the three adopted
  manifests`) directly contradicts goal 3, so it was replaced,
  not kept — its replacement asserts the island's absence plus
  the empty state. Every other pre-existing test is unmodified
  and green.
- `parseModuleData` / `renderModules` remain exported in
  `modules.mjs` (the parser is a pure helper; deleting exported
  API was out of scope) but the page no longer feeds them.
- No deviation on BRAINFIX, broker rules, block vocabulary,
  tool list, or text-node rendering: those paths were reused,
  not edited.

## Open items

- **Lifetime/TTL enforcement on the render side**: manifests
  declare `expires_when`/`max_ttl_minutes`, but kimono does not
  yet expire a card when its session dies or TTL passes (agentd
  also emits no retraction). Protocol-conformant next step;
  deliberately not invented here.
- **Only two card kinds exist.** ETA/flight cards from the
  workbench prototype still have no live data source (no route /
  calendar state in agentd); they were not faked onto this path.
- **Phone deployment is untested by design** (another worker owns
  the phone tonight): the kimono web files must be mirrored to
  the phone's static dir through the normal deploy path before
  this renders on device, and `gio` success there is still the
  KEY-TEST-1-proven path, not re-proven here.
- **`follow_up_window` in the session snapshot is agentd's mirror
  of the listener's last report**; if the listener dies without
  posting `closed`, the card can show `open` until session expiry.
  The mic chip has the same property today (it reads the same
  reports) — one source, same honesty, noted not fixed.
