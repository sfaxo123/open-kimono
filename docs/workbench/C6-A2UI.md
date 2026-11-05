# C6 — A2UI renderer (`crates/a2ui-render`)

Branch `apps/a2ui-render`. Renders a subset of Google's A2UI v0.9 declarative
UI protocol on the phone in the same black+orange OrbClient style as
`apps/timeline`, and journals every interaction as an AG-UI `CUSTOM` record on
the `trace-events` ring (C2). Spec read at
`op6-refs/a2ui @ 409ded29ccac3eebc541979ae5320771703248e8` (Apache-2.0,
**spec-only** — nothing vendored, see `docs/PROVENANCE.md`).

## Reproduce it

```sh
# one document, drawn off-screen (no display needed — the app probes for one)
~/.cargo/bin/cargo run -q -- --doc fixtures/deploy.json --screenshot out.png

# the real event loop with no screen (evidence runs)
SDL_VIDEODRIVER=dummy cargo run -q -- --doc fixtures/broken.json \
    --screenshot out.png --exit-after 150

# a scripted, reproducible interaction, journalled to stdout
cargo run -q -- --doc fixtures/deploy.json --focus note --text "ship it" \
    --click 240,300 --trace 0123456789abcdef0123456789abcdef --events -

# the whole suite
~/.cargo/bin/cargo test -q && ~/.cargo/bin/cargo clippy -q --all-targets
```

## Evidence images (rendered by the binary, 480×800)

| File | What it proves |
| --- | --- |
| `C6-deploy.png` | heading + rule, caption, divider, a `List` **template** over the data model (`checkout - done` …), a determinate progress bar reading `/percent` = 62 %, a text field, a filled primary button and an outlined default button — all full-width, ≥ 48 px tall |
| `C6-booking.png` | a `Card` (frame drawn), three `TextField`s with their bound values (`2026-11-02`, `4`, `101`), an image placeholder with its description, a `Row`, one primary action |
| `C6-progress.png` | both bar kinds — determinate at 25 % and indeterminate (three blocks) — a paragraph wrapped to the column, and a second template list |
| `C6-broken.png` | the failure path: `unsupported: Calendar` chip, `waiting: not_yet`, `cycle: cyc`, an inert button drawn dim, a missing binding left as a hole, and the `MORE BELOW` marker with its orange bar because 26 log lines overflow the screen |

The goldens in `crates/a2ui-render/fixtures/golden/` are byte-identical frames
of these; `tests/golden.rs` decodes them and compares **pixels** against what
the library draws, and separately asserts that the **binary's** screenshot is
pixel-identical to both — so the evidence images are the app's real frame, not
a mock-up.

## A real journal excerpt

`--focus note --text "ship it" --click 240,300` on `fixtures/deploy.json`
(the press lands on the primary button):

```json
{"seq":0,"ts_us":1792889782463783,"trace_id":"0123456789abcdef0123456789abcdef",
 "payload":{"kind":"ag_ui","type":"CUSTOM","name":"a2ui.input","value":
  {"journalOnly":true,"path":"/note","sourceComponentId":"note","surfaceId":"deploy",
   "timestamp":"2026-10-25T00:56:22.463Z","value":"ship it","version":"v0.9"}}}
{"seq":1,"ts_us":1792889782463853,"trace_id":"0123456789abcdef0123456789abcdef",
 "payload":{"kind":"ag_ui","type":"CUSTOM","name":"a2ui.action","value":
  {"action":{"context":{"env":"staging","note":"ship it"},"name":"deploy.confirm",
   "sourceComponentId":"go","surfaceId":"deploy","timestamp":"2026-10-25T00:56:22.463Z"},
   "version":"v0.9"}}}
```

The second record is the A2UI client→server `action` envelope **verbatim**
(`name`, `surfaceId`, `sourceComponentId`, ISO-8601 `timestamp`, `context` with
every data binding already resolved — including the note the user just typed).
The first is marked `"journalOnly": true` because the protocol sends **nothing**
to the server for a keystroke; the local trace still records it, and the value
was written into the model at `/note` before the press.

## Tests

`~/.cargo/bin/cargo test -q` → **117 passed, 0 failed**; `cargo clippy -q
--all-targets` silent.

| Suite | Tests | Covers |
| --- | --- | --- |
| `tests/doc.rs` | 13 | message envelope, `version` required, JSONL, rejection of malformed messages |
| `tests/pointer.rs` | 7 | RFC 6901 get/set/remove, escapes |
| `tests/surface.rs` | 10 | surface lifecycle, component map, field writes into the model |
| `tests/value.rs` | 17 | bindings, `formatString` `${…}`, `formatNumber`, `pluralize`, `and/or/not/required`, depth guard |
| `tests/scene.rs` | 33 | layout of every supported type, templates, cycles, overflow, hit testing |
| `tests/png.rs` | 9 | PNG writer + decoder round-trip, CRC and filter rejection |
| `tests/view.rs` | 12 | palette discipline (five colours only), button variants, bar fill, caret, chips, overflow marker |
| `tests/events.rs` | 8 | ISO-8601 vectors, action/error envelopes, journal-only input, gapless `seq`, JSONL round-trip |
| `tests/golden.rs` | 8 | the four fixtures vs goldens, per-fixture semantics, binary-vs-library pixels, scripted tap journal |

## Decisions worth knowing

- **Headings are uppercase + orange + a rule (h1), not larger.** orbclient
  draws unifont at a fixed 8×16 with no scaling; that is the only emphasis this
  screen has. Pinned by a test so it cannot regress into an invisible change.
- **`Progress` is an extension, not a spec claim.** The minimal catalog is
  exactly Text/Column/Row/Button/TextField; neither catalog has a progress bar.
  The community `MaterialProgressBar` shape is accepted under three names.
- **Unknown components are never silent**: an `unsupported: X` chip in orange,
  plus a diagnostic naming the id. A child id that has not arrived yet renders
  `waiting: <id>` (progressive rendering is part of the protocol), a cycle
  renders `cycle: <id>` instead of hanging.
- **Interactions are AG-UI `CUSTOM` records carrying A2UI JSON**, so the same
  journal can be replayed to an agent without translation.
- **The theme is recorded but not applied** — this renderer owns the palette
  (black + `rgb(255,140,0)` + three greys), which is what the OrbClient style asks for.

## Gaps

- No scrolling: content past the screen shows `MORE BELOW`, but there is no
  touch drag or scrollbar yet (the phone's touch driver is not in Redox).
- `Row` is laid out but a horizontal `List` degrades to a stacked column at
  480 px; no measurement-based wrapping inside a row.
- Images are placeholders only — no fetch, no decode of a real URL.
- `CheckBox`, `Tabs`, `Modal`, `Video`, `AudioPlayer` and the rest of the basic
  catalog are not implemented; they render as `unsupported: X`.
- Client-side `functionCall` actions (e.g. `openUrl`) are journalled
  (`a2ui.function`, journal-only) but not executed — no browser to open.
- Keyboard is text-input only (no IME, no backspace handling in the live loop).
- Not run on the phone: no Redox build target for this crate yet, so the
  evidence is the SDL dummy-driver frame, not a device screenshot.
