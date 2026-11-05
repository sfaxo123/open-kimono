# Track A wiring status (2026-11-03)

Overnight items A1.1–M1.GATE were reopened: they had passed on suites that
already passed, or on code nothing called. An item is wired only when the
behaviour is reachable through `POST /request` (or voiced / the kimono
fixtures for UI items), proven by its own test with a negative case, and
the load-bearing line was broken once and seen red.

| Item | State | Own test | Mutation seen red |
|---|---|---|---|
| A1.1 mic/waveform shell | wired (mic half; waveform UI itself not built) | `cargo test --offline --manifest-path apps/voiced/Cargo.toml --test pipeline mic_` (3 tests) + `cargo test --offline --manifest-path apps/kimono-ui/Cargo.toml --test fixtures_valid the_voice_fixture_shows_mic_on_and_mic_off_truthfully` | `"off"` parsed as `MicState::On` in voiced capture.rs: mic-off test went red, restored green |
| A1.2 broker gate | wired | `cargo test --offline --manifest-path apps/agentd/Cargo.toml --test broker_gate` (5 tests) | `confirmed = true` in agent.rs gate: unconfirmed + wrong-label tests went red, restored green |
| A1.GATE shell+runtime gate | wired (runtime half; shell half is A1.1/W4) | same broker_gate suite: generated-block (`service_create`) commit attempts gated | same mutation as A1.2 |
| A2.1 pattern store + sync | open — no sync transport or reset-survival path exists to wire to today; the store in crates/caps-check is unreferenced by agentd | — | — |
| A2.2 anticipation | open — no live context source (car Bluetooth etc.) reaches agentd today, so prompts would be fabricated | — | — |
| A2.GATE learning gate | open — depends on A2.1/A2.2 producing a learned change to gate | — | — |
| A3.1 Lego generator | open — no sandboxed generation path is wired into agentd today; services are model-authored IR, not generated blocks | — | — |
| A3.2 routines (was cron) | wired — cron deleted, routines are services | `cargo test --offline --manifest-path apps/agentd/Cargo.toml --test routines_services` (2 tests, incl. negative pause) | `hub.create` replaced with a fixed error in tools.rs: both routines tests went red, restored green |
| A3.GATE generation gate | open — depends on A3.1 existing to prefer a reused block over regeneration | — | — |
| A4.1 drain analytics | wired | `cargo test --offline --manifest-path apps/agentd/Cargo.toml --test drain_power_wire` (6 tests) | discharge check flipped (`<` → `>`) in power.rs sampling: measured-drop + pre-approved tests went red, restored green |
| A4.2 power manager | wired | same drain_power_wire suite (recommendation, pre-approval, dry-run) | same mutation as A4.1 |
| A4.GATE power gate | wired | W3 suite (no-baseline refusal) + W4 mic tests (mic-off blocks listening) | W3 and W4 mutations above |
| M1.1 candidate inventory | open — promotion/pin changes need an operator decision; inventory alone decides nothing | — | — |
| M1.2 phone benchmark | open — needs a measured run on OnePlus 6 hardware; host runs would be invented numbers | — | — |
| M1.3 Ally benchmark | open — needs a measured run on the Ally; host-only rule forbids it here | — | — |
| M1.GATE evaluation gate | open — needs the M1.2/M1.3 measurements plus a new operator pin decision (no auto re-pin) | — | — |

## Decisions

- W1 classes: `service_create` is the only commit-class tool today —
  installing a service commits a (possibly generated) block that fires
  on its own timers. `service_stop` is free: svcd stop keeps the IR and
  state under a `stopped` status and deletes no user data (proof:
  `service_stop_is_free_because_it_deletes_nothing`). All other tools
  read, report, or dry-run only, so they are free. Confirmation is the
  exact label (`service_create <id>`, or `service_create` when the IR
  has no id yet); any other string confirms nothing.
- W2 pause: svcd has no paused state (only running/stopped), so agentd
  offers no pause tool and refuses `service_pause` by name. The svcd
  patch that would add it is proposed, not applied, in
  `docs/track-a/proposed/svcd-pause-resume.md`. `src/cron.rs` and
  `tests/cron_engine.rs` are deleted; routines are services.
- W3 drain scope: only the overall battery rate is measurable from
  sysfs, so `Drain` records `battery` samples (percent drop ÷ elapsed,
  discharging only) and recommendations cite that evidence while
  naming the component proposed from the existing plan logic.
  Per-component drain attribution is not measured and is not invented.
  Pre-approvals come from `power_preapproved` in agentd.toml; even
  pre-approved radios/screen/Bluetooth stay dry-run.
- W4 mic truth: `apps/kimono-ui/src/shell.rs` is deleted. It duplicated
  mic on/off as a second state machine with no caller beyond its own
  tests. The single source is now voiced's mic switch file (one word,
  `on`/`off`; missing = on; anything else fails closed) plus the
  `PowerChange` mic records voiced journals, which are what the kimono
  fixtures render. The waveform widget itself is still not built; what
  is wired is that mic-off really blocks listening everywhere.

## Leftovers

- `crates/lfm3-brain/tests/ally_bench.rs` and `ondevice_bench.rs`:
  deleted. Both asserted constants against themselves and reported
  NOT MEASURED by construction; they could never go red on a real
  regression. `docs/workbench/LFM-EVAL.md` stays as the plan of record
  for the measurements that replace them.
- `tools/user-sim/src/anticipation.rs`: kept, still unwired. It is
  referenced by its own crate (`mod anticipation` in user-sim's main)
  and sits outside this pass's editable paths; deleting it would break
  that crate for no wired gain. It wires only when a real context
  source exists (see A2.2).
- `crates/caps-check` (pattern store): kept, still unwired. Nothing in
  agentd references it and there is no sync transport to wire it to
  (see A2.1); it is outside this pass's editable paths.
- Deleted in this pass: `apps/agentd/src/cron.rs`,
  `apps/agentd/tests/cron_engine.rs`, `apps/kimono-ui/src/shell.rs`
  (see W2/W4 decisions). Nothing else was deleted.

## W6 — confirm from the kimono page

W1 left the phone demo stuck: `service_create` needs
`"confirm": "<label>"`, but the page had no way to send it. The
`ask_to_commit` CUSTOM event now renders as a confirm card (panel
`confirm`, kind `ask-to-commit`) showing the label and the gate's
reason, with a Confirm button that re-POSTs the SAME request text
with `confirm` = the label — exactly `{text, confirm}`, no other
keys — and a Cancel button that only dismisses. The original text is
the one the run itself carried: `render.mjs` keeps
`lastRequestText` from `RUN_STARTED` `input.text`, the card stamps
it onto the button, and `confirmRequestBody` in
`apps/kimono-ui/web/render.mjs` builds the body (the headless test
pins its exact keys; `app.js` posts that builder's output). The
replay server (`server.mjs`) accepts `confirm` the way agentd's
`parse_request` does and refuses a non-string confirm by name.
Fixture: one `ask_to_commit` line generated into
`fixtures/vocab_coverage.jsonl` by `scene_vocab.rs` (label
`service_create svc.demo.water_reminder`, agentd's own reason
string), so the Rust vocabulary tests and the node render test both
exercise the card.

| Item | State | Own test | Mutation seen red |
|---|---|---|---|
| W6 confirm card | wired | `node --test test/confirm_test.mjs` in apps/kimono-ui (6 tests: card from fixture, Confirm posts exactly {text, confirm:label}, negatives — no card for other CUSTOM, Cancel posts nothing, label-less gate confirms nothing, server accepts/refuses confirm) | `ev.name === 'ask_to_commit_MUTATED'` in render.mjs CUSTOM branch: card + Confirm + Cancel tests went red (3 fail), restored green (6/6) |

WIRE DONE
