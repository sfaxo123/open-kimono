# Open Kimono: an agentic phone OS, from kernel to voice

**Meta Global AI Developer Hackathon, Nov 2026.** This is a OnePlus 6 (SDM845) that does what you tell it in plain
language and shows its work. Meta's **Muse Spark** is the agent's brain, and the OS underneath is the **Redox**
microkernel being ported to the phone.

> "remind me every 2 minutes to drink water"
> The Muse Spark brain writes a timer service. The commit gate asks you to confirm, it installs, fires twice,
> and stops when you say "stop the water reminder". Every model call, tool call and power change is
> visible in the trace. [`docs/hackathon/MUSE-BRAIN.md`](docs/hackathon/MUSE-BRAIN.md)

## Start here
- [`docs/hackathon/SUBMISSION.md`](docs/hackathon/SUBMISSION.md): what it is, the architecture, and what is verified
- [`docs/hackathon/MUSE-BRAIN.md`](docs/hackathon/MUSE-BRAIN.md): Muse Spark as the brain (Meta Model API), with the evidence trace
- [`docs/hackathon/EVIDENCE.md`](docs/hackathon/EVIDENCE.md): 31 host test suites, 1,327 tests passing, and which proofs are phone or emulator
- [`docs/hackathon/DEMO-SCRIPT.md`](docs/hackathon/DEMO-SCRIPT.md): the 2-minute demo
- [`docs/VISION.md`](docs/VISION.md) and [`docs/INTENT.md`](docs/INTENT.md): where this is going

## Layers (bottom → top)
| Layer | Where | State |
|---|---|---|
| Redox kernel on SDM845, RAM-booted with `fastboot boot` (nothing flashed) | `patches/`, `crates/op6-boot/` | RAM-boots on the real phone and self-resets to postmarketOS (5/5, 3/3 cycles, [`DEVICE-RUNS`](docs/hackathon/DEVICE-RUNS.md)); serial shell in the emulator |
| Rust drivers and specs: UART, I2C, UFS, DWC3 USB gadget, touch, SPMI, rproc | `crates/`, `specs/` | host-tested; replay and emulator harnesses |
| Lego services: model-authored IR that `svcd` validates and simulates before it runs | `crates/svcd/` | host-tested |
| `agentd`: tool loop, commit gate, power policy, journal + SSE | `apps/agentd/` | on the phone (postmarketOS) and the host |
| Brain: Muse Spark via the Meta Model API, with an on-device llama.cpp fallback | `tools/muse-relay/` | host-proven end to end |
| `kimono-ui` (open trace UI) and `voiced` (wake phrase "hey phone") | `apps/kimono-ui/`, `apps/voiced/` | on the phone |

Meta models were also used to build the project. `muse-spark-1.3-contributor` ran as autonomous,
budget-capped coding contributors, with one packet and one commit each. `muse-wonder-1.0` independently reviewed new diffs.

## Running the tests
`cargo test --offline --manifest-path <crate>/Cargo.toml` for each crate. Six `crates/fdt-diff` tests (`cli`, `fdt_real_pair`)
read raw phone-partition dumps that are not published here, so they fail on a fresh clone. Every other suite needs nothing extra.

## Honesty rule
Every claim in `docs/hackathon/` names the file that proves it. Emulator results are labelled emulator.
Anything not yet proven is listed as planned.

## Licence
Crates and apps are MIT unless their `Cargo.toml` says otherwise. Files under `patches/` that derive from
U-Boot or Linux keep their upstream licences (GPL-2.0), as marked in each file.
This public tree is a snapshot of a private working repo. Internal ledgers and host-specific logs were removed,
and host paths and addresses were replaced with placeholders.
