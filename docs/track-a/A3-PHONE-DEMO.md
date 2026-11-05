# A3 phone demo — queued, ready to run

Status: **not yet run.** The phone left the mesh at 7:05 AM PT (mesh timeout, home-Wi-Fi IP dead from the
Mac, no USB on the Mac; spark / macbook / 5090 all answered, so the 3090 end is healthy). Lane 2
independently saw the phone drop off Meshnet at 7:58 AM PT, which points at suspend. See STATUS L1-D9.

Everything the demo needs is already built and host-tested:

- binary: `apps/agentd/target/aarch64-unknown-linux-musl/release/agentd` — static musl, 3 768 240 bytes,
  `sha256` prefix `29378188bbe253bf` (built at HEAD `dd7df08`). After the `mv` in step 1, check the hash on
  the phone: `sha256sum ~/agentic/bin/agentd` — a truncated transfer over the mesh is a real failure mode.
- host proof: `cd apps/agentd && cargo test --offline` → 45 passed, 10 of them the Lego layer
- phone config already on the phone: `~/agentic/agentd.toml` (`gateway_model = "5090"`, listen on
  `127.0.0.1`), key at `~/agentic/litellm.key` (0600 — read by agentd only, never copied or printed)

## Rules this run must obey

Read-only plus dry-run plans. Install only under `~/agentic/`. Run as `user`. Record every PID. Kill what
you started. Never touch radios, DNS, the container config, or anything outside `~/agentic/`.

## Run

```bash
# 0. reachability + reap anything a previous lane left behind
ssh oneplus 'pgrep -af "agentd|voiced" || echo none'
# Lane 2 left agentd 109896 and voiced 109901 when the link dropped. Kill only the agentd one
# (voiced belongs to Lane 2 and its worker should decide), then confirm:
ssh oneplus 'kill 109896 2>/dev/null; sleep 1; pgrep -af agentd || echo agentd-gone'

# 1. deploy the current binary
scp apps/agentd/target/aarch64-unknown-linux-musl/release/agentd oneplus:~/agentic/bin/agentd.new
ssh oneplus 'mv ~/agentic/bin/agentd.new ~/agentic/bin/agentd && chmod +x ~/agentic/bin/agentd'

# 2. start the server in the background and RECORD the pid (screen stays off)
ssh oneplus 'cd ~/agentic && setsid nohup ./bin/agentd --config ~/agentic/agentd.toml \
  --trace-out ~/agentic/out/a3-water-reminder.trace >~/agentic/out/a3.log 2>&1 < /dev/null & \
  echo AGENTD_PID=$!; sleep 1; curl -s http://127.0.0.1:8791/health'

# 3. ask for the service. The prompt is deliberately explicit: the model has both examples in its
#    system prompt, so it should copy example 1 and change every_sec to 90.
ssh oneplus 'curl -s -X POST "http://127.0.0.1:${PORT:-8791}/request" \
  -H "content-type: application/json" \
  -d "{\"text\":\"Create a service that reminds me to drink water every 90 seconds. Use service_create with the full ServiceIR, then tell me the service id and whether it is running.\"}"'

# 4. let the interval come due twice, then ask whether it fired
sleep 200
ssh oneplus 'curl -s -X POST "http://127.0.0.1:${PORT:-8791}/request" \
  -d "{\"text\":\"List my services and tell me how many water reminders have fired.\"}"'

# 5. stop it through the tool, then stop the server
ssh oneplus 'curl -s -X POST "http://127.0.0.1:${PORT:-8791}/request" \
  -d "{\"text\":\"Stop the water reminder service.\"}"'
ssh oneplus 'pkill -f "bin/agentd --config" ; sleep 1; pgrep -af agentd || echo agentd-gone'
```

## What counts as evidence

The trace file must contain, in order:

1. `TOOL_CALL_START` for `service_create` with the IR in the args,
2. svcd's own records mirrored in (`"service":"svcd"`, `svc.timer.armed`),
3. at least one `svc.effect_requested` with `"performed":false,"dry_run":true` and the reminder text —
   proof the timer fired on the real clock and the effect was recorded rather than performed,
4. `service_list` / `service_stop` results, with the stop reason,
5. no `gen_ai` record inside a deterministic service run.

Then:

```bash
scp oneplus:~/agentic/out/a3-water-reminder.trace docs/track-a/evidence/A3-phone-water-reminder.trace
# validate every line (L1-D3: `.trace`, because `*.jsonl` is gitignored)
./target/debug/smoke --validate docs/track-a/evidence/A3-phone-water-reminder.trace
```

If a screen probe is missing on the phone, fix the two paths in
`apps/agentd/src/power.rs` to the ones that exist and re-run — an `unknown` that
could have been an `on` is a reporting bug, not a fact about the phone.

## What actually happened when it ran (11-02, 7:40–7:52 AM PT)

The script above is the plan; three things turned out to need changing, and all
three are now in the code, not just in this document.

1. **An `ssh`-launched daemon dies with the session.** `setsid nohup … &` printed
   a PID and then the process was gone. What survives is a transient user unit:
   `systemd-run --user --unit=agentd-a3 --collect
   --property=StandardOutput=append:$HOME/agentic/out/a3.log
   $HOME/agentic/bin/agentd-a3 --config … --trace-out …`
   with `XDG_RUNTIME_DIR=/run/user/$(id -u)` exported. The PID is
   `systemctl --user show agentd-a3 -p MainPID --value` (it was 8233, then 8902),
   and the clean shutdown is `systemctl --user stop agentd-a3`.
2. **Deploy under your own name.** Lane 2 already had `bin/agentd`,
   `agentd-kimono.toml` and port 8770 on the phone. Lane 1 used `bin/agentd-a3`,
   `agentd-a3.toml` and port 8791 so neither lane guessed the other's files.
3. **The first two runs failed for reasons that were ours**, and the trace said
   so: the model answered "I'll build the service…" without calling the tool
   (the prompt now forbids narrating instead of acting), and then the gateway
   spent its whole 512-token completion budget on reasoning and emitted nothing
   (`RUN_ERROR` code `empty_reply`, `completion_tokens: 512`). `max_tokens` is now
   `max_completion_tokens`, default 2048.

Result: the service was created, fired three times on the real clock, and was
stopped by a later request. Evidence: `evidence/a3-water-reminder.trace`
(170 lines, all valid) and `evidence/a5-phone-power.trace` (35 lines). The phone
was left with no agentd running.

A negative worth capturing too: create a service whose own test asserts something false and show the
refusal text coming back to the model (`refused before install — the service does not pass its own
simulation: …`) with `service_list` still empty.
