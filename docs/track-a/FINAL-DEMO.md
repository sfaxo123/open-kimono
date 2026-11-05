# Track A final demo — run by the reviewer, 2026-11-02 ~9:26–9:31 AM PT

Binary: agentd built from main `36a2bdc` (both lanes merged), static musl, sha256 prefix `b1f9570e53c9d59b`,
deployed as `~/agentic/bin/agentd-main`, config `~/agentic/agentd-demo.toml` (loopback `127.0.0.1:8792`,
`static_dir` = kimono), run as transient user unit `agentd-demo`, stopped afterwards.

| Step | Request | Result |
|---|---|---|
| 1 | `remind me every 2 minutes to drink water` | created `svc.demo.water_reminder_2min`, passed simulation, running |
| 2 | (wait ~250 s on the phone clock) | 2 × `svc.effect_requested` (dry-run "Time to drink some water.") |
| 3 | `stop the water reminder` | stopped; "had sent 2 notices, timer cleared" |
| 4 | `list my services` | 1 service, status **stopped** with the user's reason |

Kimono view: `GET /` → 200 from the same agentd. Trace: `evidence/final-demo-water-reminder.trace`
(153 records: 9 tool calls, 32 gen_ai spans, 13 power_change, grant granted/revoked, service_stopped).

Run it yourself (phone):
```sh
export XDG_RUNTIME_DIR=/run/user/$(id -u)
systemd-run --user --unit=agentd-demo --collect ~/agentic/bin/agentd-main --config ~/agentic/agentd-demo.toml --trace-out ~/agentic/out/demo.trace
# open http://127.0.0.1:8792/ on the phone, type the sentence; stop with: systemctl --user stop agentd-demo
```

Gap vs PLAN wording: "stop" leaves the service listed as *stopped* rather than removing it from the list.

## Update 2026-11-03 ~7:00 AM PT — confirm step (after ta/wire, main e7e8189)
Creating a service is now commit-class. On the phone (binary sha256 prefix `f165f7063bbad9ec`):
1. "remind me every 2 minutes to drink water" → not created; answer asks for confirmation,
   trace `ask_to_commit` label `service_create svc.demo.water_reminder`.
2. Same text + `"confirm": "service_create svc.demo.water_reminder"` → created and running.
3. "stop the water reminder" → stopped. agentd stopped afterwards; listen_server.py untouched.
On the kimono page step 2 is the Confirm button on the card.
Caveat: confirming re-runs the request, so it relies on the model choosing the same service id; if it picks a
different one, it asks again rather than installing something unconfirmed.
