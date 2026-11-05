# a2ui-render

A front end for [Google's A2UI v0.9](https://github.com/google/A2UI) protocol:
an agent sends a declarative UI document, this draws it on the OnePlus 6 in the
OrbClient black+orange style and journals what the user did back as trace
events. Software rendering, one column, 480×800, touch targets ≥ 48 px.

```sh
cargo run -- --doc fixtures/deploy.json --screenshot out.png   # no display needed
cargo run -- --demo                                            # bundled fixture
cargo run -- --doc - < stream.jsonl                            # a live message stream
cargo test                                                     # 117 tests
```

## What it speaks

Messages: `createSurface`, `updateComponents`, `updateDataModel`,
`deleteSurface` (as a JSON document, an array, or one message per line).
Components: `Text` (body/caption/h1–h5), `Column`, `Row`, `Card`, `Divider`,
`List` (plain ids or a `{componentId, path}` template), `Button`
(`default`/`primary`/`borderless`), `TextField`, `Image` (placeholder), and
`Progress`/`ProgressBar`/`MaterialProgressBar` — the last is an extension, the
catalogs have no progress bar. Data bindings are JSON Pointers, absolute or
relative to a template item, with the catalog functions `formatString`,
`formatNumber`, `pluralize`, `and`, `or`, `not`, `required`, `openUrl`.

Anything unknown draws a visible `unsupported: X` chip and is named in the
diagnostics — a half-built or malformed document is never silently blank.
A child id that has not arrived renders `waiting: <id>`; a cycle renders
`cycle: <id>` instead of hanging.

## What the user does becomes a trace event

A press emits an AG-UI `CUSTOM` record whose value is the A2UI client→server
`action` envelope verbatim, with every binding in `context` already resolved.
Typing and client-side `functionCall`s are journalled locally
(`a2ui.input`, `a2ui.function`, marked `"journalOnly": true`) because the
protocol sends nothing for them. See `docs/workbench/C6-A2UI.md` for a real
excerpt and the four evidence screenshots.

Spec read at `op6-refs/a2ui @ 409ded2` (Apache-2.0, spec-only — no vendored
code); see `docs/PROVENANCE.md`. Traps that cost time are in `docs/GOTCHAS.md`.
