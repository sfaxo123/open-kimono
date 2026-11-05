# Open Kimono: an agentic phone OS, kernel to voice

**One-line pitch:** A phone OS that does what you say, shows its work,
and proves it — from the kernel up to the voice that answers you.

## The problem

- Phone assistants today are black boxes: you speak, something happens
  somewhere, and you cannot see what it did or why
  (framing; the answer below is in `docs/track-a/`).
- On-device agents need a real OS story too: boot, drivers, power, and
  mic discipline — not just a chatbot stapled to a launcher
  (see `docs/VISION.md`, `docs/INTENT.md`).
- Voice agents that act on their own timers (reminders, routines) need
  commit gates: installing a self-firing block must ask first, and
  anything the mic hears must be truthfully reported
  (see `docs/track-a/WIRE-STATUS.md`).

## What it does

- Runs an agent daemon on the phone itself: type or say "remind me
  every 2 minutes to drink water" and it installs a real timer service,
  fires dry-run notices, and stops on command
  (`apps/agentd/`, `docs/track-a/FINAL-DEMO.md`).
- Every agent run is journaled as trace records and streamed to an
  open UI — the "kimono" view shows runs, tools, power changes, and
  grants instead of hiding them
  (`apps/kimono-ui/`, `docs/track-a/KIMONO-UI.md`).
- Voice loop with wake phrase "hey phone": mic switch file, STT/TTS
  engines as config, quiet mode that writes the answer WAV without
  playing sound and takes input from a pre-recorded WAV instead of
  the live mic
  (`apps/voiced/`, `docs/track-a/VOICE.md`,
  `docs/workbench/user-simulations/key-tests/KEY-TEST-1-WAKEWORD.md`).
- Power discipline from measured battery drain only: recommendations
  cite sysfs evidence, and radios/screen/Bluetooth stay dry-run even
  when pre-approved (`apps/agentd/src/power.rs`,
  `apps/agentd/src/power_policy.rs`, `apps/agentd/src/drain.rs`).
- Lego services: small capability-boxed blocks the model authors as IR,
  run by `svcd`, gated by a broker that demands exact-label
  confirmation for anything commit-class (`crates/svcd/`,
  `apps/agentd/src/broker.rs`, `docs/track-a/WIRE-STATUS.md`).

## Architecture, bottom to top

- **Kernel:** Redox OS microkernel ported to the OnePlus 6 / SDM845 —
  RAM-booted on the real phone: the kernel arms its own reset timer and
  returns the phone to postmarketOS unattended — 5/5 and 3/3 consecutive
  device cycles (`docs/hackathon/DEVICE-RUNS.md`)
  (`README.md`); early framebuffer boot console discovered from the
  device tree (`docs/workbench/K14-FBCON.md`, `patches/`).
- **Boot:** RAM-only `fastboot boot` envelope via `op6-boot` —
  postmarketOS is never flashed, no EDL/firehose
  (`crates/op6-boot/`, `docs/hackathon/LANE-C.md`); QEMU RAM-boot path
  with serial rescue shell for host iteration
  (`docs/workbench/K3-RAMBOOT.md`).
- **Drivers/specs (Rust):** UART, I2C, UFS, USB gadget, touch,
  remote-proc, SPMI, plus replay/diff harnesses (`crates/geni-uart/`,
  `crates/geni-i2c/`, `crates/ufs/`, `crates/dwc3-gadget/`,
  `crates/rmi4-touch/`, `crates/qcom-rproc/`, `crates/spmid/`,
  `crates/mmio-replay/`, `crates/fdt-diff/`).
- **Voice protocol:** Wyoming client crate shared by the probe and the
  voice daemon (`crates/wyoming/`, `apps/wyoming-probe/`,
  `docs/workbench/E1-WYOMING-PROBE.md`).
- **Agentic layer (Track A, on the phone today):** runs on the
  postmarketOS/Linux install already on the OnePlus 6, written as
  portable Rust services behind a small capability interface so they
  can move to Redox later (`docs/VISION.md`, `apps/agentd/`,
  `crates/svcd/`, `apps/kimono-ui/`, `apps/voiced/`).
- **agentd:** request loop, model gateway, tool broker with
  commit-class gating, journal + SSE, power policy
  (`apps/agentd/src/agent.rs`, `apps/agentd/src/broker.rs`,
  `apps/agentd/src/server.rs`, `apps/agentd/src/stream.rs`).
- **svcd:** the Lego runtime that hosts service blocks with timers
  (`crates/svcd/`).
- **kimono-ui:** renders only what the journal says; an unparseable
  frame becomes a visible banner, never an invented record
  (`apps/kimono-ui/src/`, `docs/track-a/KIMONO-UI.md`).
- **voiced:** wake-word listener, mic ownership lock, STT→agentd→TTS
  turn pipeline with per-step journaling
  (`apps/voiced/src/capture.rs`, `apps/voiced/src/turn.rs`,
  `docs/track-a/VOICE.md`).

## How Meta models are used

- `muse-spark-1.3-contributor` runs as autonomous Lane C contributor
  workers: one packet each, one commit, budget-capped, worktree
  isolated (`docs/hackathon/LANE-C.md`).
- `muse-wonder-1.0` reviews every new diff as an independent reviewer
  (`docs/hackathon/LANE-C.md`).
- Wonder was also evaluated as the phone-brain candidate in Key Test 1:
  39 turns over 16 scenarios through the user-sim shim — conversation
  works first try, destination-confirmation needs fixing
  (`docs/workbench/user-simulations/key-tests/KEY-TEST-1-RESULTS.md`).
- **Muse Spark is the agent's brain.** `muse-spark-1.3` drives agentd's
  tool loop through the Meta Model API via a 150-line relay
  (`tools/muse-relay/`): it authors the service IR, hits the commit gate,
  installs after confirmation, lists and stops the service
  (`docs/hackathon/MUSE-BRAIN.md`, trace in `docs/hackathon/evidence/`). Host run
  today; the same binary/config runs on the phone.

## What is verified

| Claim | Evidence file | Phone / emulator / host |
|---|---|---|
| Water-reminder agent demo runs end to end | `docs/track-a/FINAL-DEMO.md`, `docs/track-a/evidence/final-demo-water-reminder.trace` (153 records) | phone |
| Commit-class `service_create` asks first; kimono Confirm button sends the exact label | `docs/track-a/FINAL-DEMO.md` (2026-11-03 update), `docs/track-a/WIRE-STATUS.md` (W1, W6) | phone |
| Broker gate blocks unconfirmed and wrong-label commits | `apps/agentd/tests/broker_gate.rs`, `docs/track-a/WIRE-STATUS.md` (A1.2) | host |
| Routines are services; cron deleted; pause refused by name | `apps/agentd/tests/routines_services.rs`, `docs/track-a/WIRE-STATUS.md` (W2, A3.2) | host |
| Drain analytics + power manager; radios/screen stay dry-run | `apps/agentd/tests/drain_power_wire.rs`, `docs/track-a/WIRE-STATUS.md` (W3, A4.1, A4.2) | host |
| Mic-off blocks listening everywhere; UI fixtures render mic truthfully | `docs/track-a/WIRE-STATUS.md` (W4, A1.1), `apps/voiced/`, `apps/kimono-ui/` fixture tests | host |
| Wake phrase "hey phone" listener (file-mode proven, systemd unit; live-mic proof pending) | `docs/workbench/user-simulations/key-tests/KEY-TEST-1-WAKEWORD.md` | phone |
| Brain key test: 14/16 behavioural (11/16 mechanical), real failure is unconfirmed destinations | `docs/workbench/user-simulations/key-tests/KEY-TEST-1-RESULTS.md` | host |
| Wyoming protocol interop + aarch64 Redox cross-build | `docs/workbench/E1-WYOMING-PROBE.md` | host |
| RAM-boot Redox in QEMU, serial rescue shell, no drive | `docs/workbench/K3-RAMBOOT.md` | emulator |
| Early framebuffer boot console from device tree (diff-reviewed, no device claimed) | `docs/workbench/K14-FBCON.md` | host |
| Phone + host power traces recorded | `docs/track-a/evidence/a5-phone-power.trace`, `docs/track-a/evidence/A5-host-power.trace` | phone + host |

Planned, not claimed: the 90-second water-reminder run in
`docs/track-a/A3-PHONE-DEMO.md` is queued and **not yet run**; the
waveform widget itself is not built; anticipation and pattern-sync
items are open (`docs/track-a/WIRE-STATUS.md`).

## What is next

- Run the queued A3 phone demo and bank its trace
  (`docs/track-a/A3-PHONE-DEMO.md`).
- Measured phone + Ally benchmarks and a pinned model decision
  (`docs/track-a/WIRE-STATUS.md`, items M1.2/M1.3/M1.GATE).
- Wire anticipation to a real context source; wire pattern sync to a
  real transport (`docs/track-a/WIRE-STATUS.md`, items A2.1/A2.2).
- Sandboxed Lego generation path inside agentd
  (`docs/track-a/WIRE-STATUS.md`, items A3.1/A3.GATE).
- Build the waveform widget; run the Muse Spark brain on the phone itself (host-proven today).
- Keep marching the Redox-on-phone boot toward the agentic layer
  running on Redox itself (`docs/hackathon/LANE-C.md`).

## How to run the demo

Copied from `docs/track-a/FINAL-DEMO.md` (on the phone):

```sh
export XDG_RUNTIME_DIR=/run/user/$(id -u)
systemd-run --user --unit=agentd-demo --collect ~/agentic/bin/agentd-main --config ~/agentic/agentd-demo.toml --trace-out ~/agentic/out/demo.trace
# open http://127.0.0.1:8792/ on the phone, type the sentence; stop with: systemctl --user stop agentd-demo
```

Then: type `remind me every 2 minutes to drink water`, wait ~250 s
on the phone clock for two dry-run notices, type `stop the water
reminder`, then `list my services` to see it stopped
(`docs/track-a/FINAL-DEMO.md`).

## Repo map (30 seconds)

- `apps/agentd/` — the agent daemon; `crates/svcd/` — service runtime.
- `apps/kimono-ui/` — the open UI; `apps/voiced/` — the voice loop.
- `crates/` — drivers, protocol crates, harnesses.
- `docs/track-a/` — demos, wiring status, voice docs, phone traces.
- `docs/workbench/` — boot, probe, and simulation records.
- `docs/hackathon/` — submission, evidence, device runs, Muse brain.
