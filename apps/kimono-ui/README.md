# Kimono UI — the open-kimono view (Track A, step A2)

A single static page that shows, live, everything the phone is doing: the
request, the model's reasoning, every tool call with its arguments and
result, the running services (with stop buttons), the power state, the mic,
and a loud banner whenever the fallback model is in use or something failed.

No build step, no CDN, no framework — the phone may be offline (PLAN A2).

## Run it against recorded traces

```sh
cd apps/kimono-ui
cargo run                       # regenerate fixtures/*.jsonl from the schema
node server.mjs --port 8790     # http://127.0.0.1:8790  (loopback only)
node server.mjs --fixture fallback_failure --speed 200
```

`server.mjs` is a zero-dependency replay server: it serves the page and
streams a recorded fixture as Server-Sent Events. It is a stand-in for
agentd, not part of the phone image.

## Tests

```sh
cargo test                      # fixtures are schema-valid, gapless, vocabulary-complete
node --test test/*.mjs          # headless render + the SSE contract
```

`test/render_test.mjs` runs the same `render.mjs`/`dom.mjs` the browser runs
against a plain-object DOM and fails if any event type in the fixtures has no
rendering, if any panel is never exercised, or if a malformed record is
forgiven. `test/sse_test.mjs` pins the wire contract agentd must satisfy.

## Layout

| file | role |
| --- | --- |
| `web/index.html` | the page shell |
| `web/style.css` | phone-width dark theme |
| `web/app.js` | browser glue: SSE feed, ask box, stop buttons |
| `web/render.mjs` | pure fold of journal records into the view model |
| `web/dom.mjs` | view model → DOM, through an injected `h` factory |
| web/modules.mjs | on-the-fly modules (protocol v1): manifest validation + card rendering |
| `server.mjs` | fixture replay server (dev/test only) |
| `src/` | `kimono-fx`: the fixture generator, built on `trace-events` |
| `fixtures/` | recorded traces (generated, committed) |
| `fixtures/bad/` | malformed records used by the negative tests |

## What the view shows

- **run / step** — which run is live and what step it is in
- **message** — the user's request and the agent's answer, streaming
- **reasoning** — the model's reasoning, verbatim (or an "encrypted value" note)
- **tool** — every tool call: name, streamed arguments, result, error state
- **span** — every `gen_ai` span with its duration and attributes
- **service** — running services as chips with **stop** buttons that send a request
- **power** — component on/off chips with the reason, plus wake locks and grants
- **mic / stt** — whether the mic is live, its level meter, partial and final transcript
- **banner** — fallback model in use, hand-off failures, dropped journal records,
  and anything the build has no rendering for (`UNHANDLED …`)

## On-the-fly modules (protocol v1)

The page renders declarative module manifests (ETA / flight / maps cards)
per docs/workbench/user-simulations/key-tests/modules/MODULE-PROTOCOL.md:
a closed block vocabulary (title, big_number, rows, action_button), the
real agentd tool list only, user_purpose required, manifest strings
rendered as text, never markup. A manifest outside the protocol renders
an explicit UNSUPPORTED MODULE state, never a partial card. An action
button never calls a tool - it sends a plain-language user request
through the same POST /request path as the ask box, so broker/confirm
rules are unchanged. For v1 the three adopted sample manifests ship as a
JSON data island in web/index.html (fixtures mirrored in
fixtures/modules/, tests in test/modules_test.mjs); the production
data path is agent-emitted manifests.
