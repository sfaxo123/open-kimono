# KEY TEST 1 — addendum: the agent can open Maps now

Companion to `KEY-TEST-1-PHONE-RUN.md`. Operator order 2026-11-03
("help it — make the tool list") authorised an additive agentd tool
so the run sheet's gap 2 ("the agent cannot open Maps") is closed
for tonight. Everything below was built and smoked on 2026-11-03;
nothing deployed before was overwritten.

## What changed for tonight

- **Step 1 starts a different binary.** Same command as the run
  sheet, one path swapped:

  ```sh
  ssh oneplus 'export XDG_RUNTIME_DIR=/run/user/$(id -u); systemd-run --user --unit=agentd-keytest --collect ~/agentic/bin/agentd-main-keytest --config ~/agentic/agentd-kimono.toml --trace-out ~/agentic/out/keytest1-agentd.trace'
  ```

  `agentd-main-keytest` (sha256 `a759266f798a170c…`; rebuilt 2026-11-03 with BRAINFIX v4 in the
  system prompt; earlier builds kept on the phone as `agentd-main-keytest.bak-b2d3ee9c` and
  `agentd-main-keytest.bak-45a59b03`) sits next to the
  deployed `agentd-main` (sha256 `f165f7063bbad9ec…`), which is
  untouched. Health check, voice conf, and the Step 3 turn are
  unchanged.

- **Step 5 is agent-done, not assisted — if the smoke below
  generalises.** Ask for the destination out loud in the Step 3
  turn (or a follow-up turn): "open maps and give me directions to
  San Francisco." The agent calls its new `app_launch` tool itself
  and GNOME Maps opens on the phone. Score Step 5 **agent-done**.
  The run sheet's manual `gio open` command stays as the fallback
  if the tool call fails (write the tool's error next to the
  verdict). Step 5's in-app route is still the real turn-by-turn —
  gap 3 (spoken directions are model prose) has **not** changed.

## The tool contract

`app_launch` — the general shape with an allowlist, rather than a
one-off `maps_open`, because the allowlist *is* the safety boundary:
the model picks an app name, and anything off the list is refused
before it can act. Tonight the list has exactly one entry.

| field | rule |
|---|---|
| `app` | required, non-empty. Only `"maps"` is allowed; anything else is refused with `app … is not on the launch allowlist (allowed: maps)` |
| `destination` | required, non-empty, ≤ 256 chars, no control characters |
| destination = place name | opens `geo:0,0?q=<percent-encoded name>` — the form verified against GNOME Maps |
| destination = `lat,lon` | both parse as numbers, lat ∈ [-90, 90], lon ∈ [-180, 180] → opens `geo:<lat>,<lon>` |
| execution | `gio open <uri>` as an argv array via `std::process::Command`. There is no shell anywhere in the path, and the destination is percent-encoded into the query, so metacharacters or a second URI in the place name stay inert text |
| broker class | **Free** (the broker's navigate class). It displays an app on the phone itself; it sends, spends, calls, shares and deletes nothing, so — exactly like `service_stop` and unlike `service_create` — no confirmation is requested. Every call and its result are journaled like every other tool |
| success result | `{"launched": true, "app": "maps", "uri": "geo:…", "via": "gio open"}` |
| failure | the refusal or the `gio` failure (exit status + stderr) is returned to the model as a named tool error, never a crash |

Source: `apps/agentd/src/launch.rs` (engine + unit tests),
`apps/agentd/src/tools.rs` (definition + dispatch),
`apps/agentd/src/broker.rs` (class note),
`apps/agentd/tests/app_launch.rs` (refusal integration tests).
Full agentd suite: 97 tests, 0 failures. Cross-built with the
documented recipe in `docs/track-a/BUILD.md` (static musl aarch64,
`readelf -d`: no dynamic section).

## The smoke (2026-11-03, phone `oneplus-enchilada`)

The keytest binary was started with the kimono config (trace to
`~/agentic/out/keytest-smoke.trace`), health-checked, given one
request through the real API, and stopped again:

- Health: `{"brains":["homelab-gateway","on-device-llama"],"dropped":0,"events":5,"ok":true}`
- `POST /request {"text":"Open maps and show me San Francisco"}` →
  answer: "Maps is open and showing San Francisco — I sent a geo
  search for \"San Francisco\" to GNOME Maps on the phone."
  (brain: homelab-gateway)
- Journal tool result: `{"launched":true,"uri":"geo:0,0?q=San%20Francisco","via":"gio open"}`
- Dispatch evidence beyond the exit code: after the request,
  GNOME Maps was running on the phone —
  `147254 /usr/bin/gjs /usr/share/gnome-maps/org.gnome.Maps --gapplication-service` —
  the DBus-activated Maps service, started by the `geo:` handoff.
- What this does **not** prove: that a window was visible on the
  phone's screen. Process + DBus activation is the headless
  evidence; the visible leg is what tonight's run is for.

After the smoke the keytest binary was stopped and no agentd was
left running; `agentd-main`'s sha256 was re-verified unchanged.
GNOME Maps itself was left open on the phone — that is the tool
working, and it is tonight's Step 5 end state anyway.

## Roll back

Delete one file; nothing else references it:

```sh
ssh oneplus 'rm ~/agentic/bin/agentd-main-keytest'
```

The deployed `agentd-main`, the kimono config, and the run sheet's
original Step 1/Step 5 then apply exactly as written.
