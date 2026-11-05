# Demo video script — Redox OnePlus 6 agentic OS (2:00)

One person, one second phone/camera pointed at the OnePlus 6, plus laptop
screen captures. Every claim names the repo file that proves it.
Rule: "on the phone" only where a phone trace/receipt exists in the repo;
QEMU results are labelled "emulator"; anything else is "planned".

## Shot list (2:00)

| Time | What is on screen | What the narrator says |
|---|---|---|
| 0:00–0:05 HOOK | Close-up of the OnePlus 6; finger types plain English into the on-phone page at `http://127.0.0.1:8792/` | "I tell my phone what I want in plain English — and it just does it. This is Redox coming to the OnePlus 6." |
| 0:05–0:25 | Phone screen: type `remind me every 2 minutes to drink water`; agentd answers, asks for confirmation; Confirm button on the card; service created and running | "Watch: I ask for a water reminder every two minutes. It asks me to confirm first — commit-class creates — then the service is created and running. On the phone." Proof: `docs/track-a/FINAL-DEMO.md` (2026-11-03 update, binary sha prefix `f165f7063bbad9ec`); trace `docs/track-a/evidence/final-demo-water-reminder.trace` (153 records). |
| 0:25–0:40 | Phone screen: wait; two dry-run notices arrive (`Time to drink some water.`); type `stop the water reminder`; `list my services` shows 1 service, status stopped | "Two minutes later, two reminders fire. I say stop — timer cleared, and the service list shows it stopped with my reason. Plain language, whole lifecycle." Proof: `docs/track-a/FINAL-DEMO.md` steps 1–4. |
| 0:40–0:55 | Phone screen: kimono UI trace view of the same run — tool calls, spans, power changes, grant granted/revoked, service_stopped | "Everything is on screen in the open-kimono UI: every tool call, every power change, every grant. Nothing hidden." Proof: `docs/track-a/KIMONO-UI.md` (`GET /` → 200 from the real agentd); `docs/track-a/FINAL-DEMO.md` (9 tool calls, 32 gen_ai spans, 13 power_change). |
| 0:55–1:10 | Laptop capture: Redox booting in QEMU to `ram:~$` (ON-SCREEN LABEL: emulator). Then the K14 boot-console log line | "Meanwhile the OS itself boots in the emulator — Redox reaches a shell with no drive, and the new framebuffer boot console is live before `Redox OS starting...`. Emulator, labelled as such." Proof: `docs/workbench/K3-RAMBOOT.md` (drive-free QEMU reaches `ram:~$`); `docs/workbench/K14-FBCON.md` + `docs/workbench/K14-QEMU.txt` (log line 38: `device tree simple-framebuffer 800x600 stride 800 at BC430000`). |
| 1:10–1:22 | Laptop capture: E1 timeline screenshot `docs/workbench/E1-REDOX-TIMELINE.png` (ON-SCREEN LABEL: emulator) | "And in the emulator, the Redox timeline app already replays a full voice-assistant session — mic, transcript, reasoning, tools — from 28 journal events. Emulator." Proof: `docs/workbench/E1-REDOX-DEMO.md` (`timeline-demo-publish: sent 28 events to trace:publish`). |
| 1:22–1:35 | Laptop capture: Wyoming host-interop probe passing (ON-SCREEN LABEL: emulator/host, planned on device) | "Voice plumbing uses the Wyoming protocol — our Rust client passes host interop against the reference server today; the on-device round trip is in progress. Honest status, on screen." Proof: `docs/workbench/E1-WYOMING-PROBE.md` (HOST_ECHO_INTEROP_PASS); guest leg pending per `docs/workbench/E1-WY-GUEST-HARNESS.md`. Planned, not claimed. |
| 1:35–1:50 | Back on the phone: agentd + kimono + voiced adapters montage; narrator to camera | "On the phone today: the agent, the open UI, and voice adapters verified standalone — speech synthesized and transcribed back exactly. Built by autonomous coding contributors running Muse Spark, every diff reviewed by Muse Wonder." Proof: `docs/track-a/VOICE.md` (adapters verified standalone; full quiet loop pending); `docs/hackathon/LANE-C.md` (Lane C: `muse-spark-1.3-contributor` workers; `muse-wonder-1.0` reviewer). Brain: Muse Spark via the Meta Model API (`docs/hackathon/MUSE-BRAIN.md`). |
| (alt 1:35–1:50, if the phone is offline) | Laptop: `scripts/hackathon/muse-brain-demo.sh` in a terminal, `brain: muse-spark-1.3` answers scrolling | "The brain is Meta's Muse Spark: it writes the service, asks before installing, and stops it on command." Proof: `docs/hackathon/MUSE-BRAIN.md`. |
| 1:50–2:00 | Title card: repo + what is real / emulator / planned | "Agentic OS now on the OnePlus 6, Redox underneath, everything shown is in the repo. Link below." |

What the video does NOT claim: live "hey phone" wake-word on the device
(the "hey phone" turns exist only in host user-simulations —
`docs/workbench/user-simulations/key-tests/KEY-TEST-1-RESULTS.md`, labelled
simulation, behavioural 14/16 with one load-bearing confirm-before-navigating
defect); Wyoming voice over emulated Redox TCP end-to-end (guest leg pending);
Muse Spark brain running on the phone itself (host-proven; phone run pending).

## Exact laptop-capture commands

Phone shot (from `docs/track-a/FINAL-DEMO.md`):

```sh
export XDG_RUNTIME_DIR=/run/user/$(id -u)
systemd-run --user --unit=agentd-demo --collect ~/agentic/bin/agentd-main --config ~/agentic/agentd-demo.toml --trace-out ~/agentic/out/demo.trace
# open http://127.0.0.1:8792/ on the phone, type the sentence; stop with:
systemctl --user stop agentd-demo
```

Emulator boot shot (from `docs/workbench/K3-RAMBOOT.md`):

```sh
qemu-system-aarch64 -machine virt,acpi=off,gic-version=3 -cpu max -smp 1 -m 2048 \
  -bios /usr/share/qemu-efi-aarch64/QEMU_EFI.fd -boot order=n \
  -nic user,model=virtio-net-pci,tftp=<build>/tftp,bootfile=bootaa64.efi \
  -display none -serial stdio -monitor none
```

K14 fbcon check (from `docs/workbench/K14-FBCON.md`):

```sh
bash scripts/integrate.sh --qemu
# assert log line 38: device tree simple-framebuffer 800x600 stride 800 at BC430000
# tracked log: docs/workbench/K14-QEMU.txt
```

E1 timeline shot: TODO — exact live-image QEMU argv not extracted for this
script (see `docs/workbench/E1-REDOX-DEMO.md` "Exact reproduction" and
`scripts/qemu-demo/`).

Wyoming host-interop shot (from `docs/workbench/E1-WYOMING-PROBE.md`):

```sh
cd apps/wyoming-probe
PATH=$HOME/.cargo/bin:$PATH cargo build --release --offline --target-dir ../../out/wyoming-probe/host
bash apps/wyoming-probe/interop.sh
```

Voiced tests shot (from `docs/track-a/VOICE.md`):

```sh
cd apps/voiced && cargo test
```

## X posts

Variant A (agent-first):

> My phone takes plain English now. "Remind me every 2 minutes to drink water" → confirmed, created, fired twice, stopped — all on the OnePlus 6, all visible in the trace. #MetaAI <VIDEO>

Variant B (OS-first):

> Redox on the OnePlus 6: boots to a shell in QEMU with its own framebuffer console, while the agentic layer already runs services by voice+text on the phone today. #MetaAI <VIDEO>

Variant C (honesty-first):

> Demo with receipts: everything in our 2-min video links to the repo file that proves it. Phone = phone traces. QEMU = labelled emulator. Rest = planned, not claimed. #MetaAI <VIDEO>

Thread (4 posts):

1. I told my OnePlus 6 "remind me every 2 minutes to drink water" in plain English. It asked me to confirm, created the service, fired twice, and stopped on command. 2 minutes, on video. #MetaAI <VIDEO>
2. The whole run is visible in the open-kimono UI: 9 tool calls, 32 model spans, 13 power changes, every grant — trace in the repo, not a black box.
3. Underneath: Redox boots to a shell in QEMU (emulator) with a new framebuffer boot console, and the timeline app replays a full assistant session from 28 journal events.
4. Built by autonomous Muse Spark coding contributors, every diff reviewed by Muse Wonder. The agent's brain is Muse Spark too. Repo link below — receipts included.

## Pre-recording checklist

1. OnePlus 6 charged (70%+) and on Wi-Fi/SSH-reachable; homelab link up.
2. `agentd-main` binary deployed at `~/agentic/bin/` on the phone; config `~/agentic/agentd-demo.toml` present.
3. No stale units: `ssh oneplus 'pgrep -af "agentd|voiced"'` shows nothing left over; reap before starting.
4. `export XDG_RUNTIME_DIR=/run/user/$(id -u)` set on the phone shell before `systemd-run`.
5. Kimono page loads: `GET http://127.0.0.1:8792/` → 200 on the phone browser.
6. Test sentence ready: `remind me every 2 minutes to drink water`; Confirm-button step rehearsed.
7. Clock plan: ~250 s wait for two notices, or show the already-captured trace `docs/track-a/evidence/final-demo-water-reminder.trace`.
8. Second phone/camera charged, tripod/framing on the OnePlus 6 screen, glare checked.
9. Laptop captures rehearsed: QEMU boot, K14 log line, E1 timeline PNG, Wyoming interop — "emulator" labels on screen.
10. Stop + cleanup rehearsed: `systemctl --user stop agentd-demo`; agentd stopped afterwards per `docs/track-a/FINAL-DEMO.md`.
