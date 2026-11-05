# Muse Spark as the agent's brain

**Result (2026-11-04, ~3:05–3:12 PM PT, host run):** Meta's `muse-spark-1.3`, called
through the Meta Model API, drove agentd through the full water-reminder lifecycle.
The model wrote the service, the commit gate stopped it, the user confirmed, the
service ran and fired twice, and then it was stopped. Everything was in plain language.

| Step | User | Muse Spark via agentd |
|---|---|---|
| 1 | "remind me every 2 minutes to drink water" | Writes the ServiceIR, hits the commit gate, and asks: *"allow exactly `service_create svc.user.water_reminder`"* |
| 2 | taps Confirm (that exact label) | *"it's running now"*: installed, simulation passed |
| 3 | waits 260 s | 2 × `svc.effect_requested` (dry-run reminder) |
| 4 | "list my services" | *"1 service: Water Reminder, running… 2 reminders sent"* |
| 5 | "stop the water reminder" | *"Found your Water Reminder and stopped it."* |
| 6 | "list my services" | *"stopped… 2 reminders sent"* |

Evidence: `docs/hackathon/evidence/muse-brain-demo.txt` (the console output) and
`docs/hackathon/evidence/muse-brain-demo.trace`. The trace has 217 journal records:
44 `gen_ai` spans, all on model `muse-spark-1.3`; 13 tool calls; the
ask-to-commit; 2 effects; and the stop.

## How it is wired

```
kimono UI / voiced ──POST /request──▶ agentd ──HTTP──▶ muse-relay ──HTTPS──▶ api.meta.ai/v1 (muse-spark-1.3)
                                        │                                   (key injected from a file)
                                        └─▶ svcd (Lego services), broker gate, journal ─▶ kimono trace
```

- `tools/muse-relay/muse_relay.py` (stdlib Python). agentd's model client speaks
  plain HTTP over the private mesh and has no TLS. The relay adds the Meta API key
  from a file, sets the model, and streams the SSE back. It also repairs echoed
  tool-call `arguments` into valid JSON strings, because Meta's API returns HTTP 400
  on anything else.
- agentd config: `gateway_url = http://127.0.0.1:8799/v1/chat/completions`,
  `gateway_model = muse-spark-1.3`, `max_completion_tokens = 8192`. Muse is a
  reasoning model. At the default 2048 tokens the reasoning used up the budget and
  truncated the tool arguments.
- The on-phone llama.cpp brain is still the fallback when the network is down.

## Defect found by switching to Muse, and fixed

The old confirm re-ran the request, and Muse re-authored the IR with a **new service
id on every run**. The label the user approved therefore never matched, and
nothing was installed. In the first trial, three confirms produced three different ids.

Fix (`apps/agentd/src/agent.rs`, `held_commits`): agentd holds each gated
commit-class call under its label. Confirming that label installs **exactly the call the
user was shown**. A label that was never shown still confirms nothing.

- Tests: `apps/agentd/tests/broker_gate.rs`. The new test is
  `confirming_a_shown_label_installs_the_held_call_even_if_the_model_renames_it`, and the negative test is
  `negative_a_renamed_call_with_no_shown_label_stays_gated`. The suite passes 7/7, and the full
  agentd suite is green.
- Mutation check: with the replay removed, the new test fails. With it restored, the test passes.

## Run it

```sh
python3 tools/muse-relay/muse_relay.py --key-file ~/.secrets/meta-model-api.key --listen 127.0.0.1:8799 &
agentd --config agentd-muse.toml --trace-out muse.trace &     # config lines above
scripts/hackathon/muse-brain-demo.sh 260
```

Not yet done: the same binary and config running on the OnePlus 6 itself. The phone was
offline during this window. The phone run uses the same relay on the homelab end.
