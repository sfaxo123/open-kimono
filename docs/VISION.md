# Vision — a language-native, Lego-style, open phone OS

> **OPERATOR RULING 2026-11-03 — CLAUDE-ONLY RESTRICTION REMOVED (read first; supersedes the ownership fence in the 2026-11-02 ruling below).**
> - The operator removed the Claude-only rule on 2026-11-03 (Claude usage running out). Remy, root of record, now works across BOTH tracks, including apps/agentd, apps/kimono-ui, apps/voiced, apps/powerd, docs/track-a and ta/* branches.
> - The two tracks remain as an organizing description of the work, not an ownership fence. Claude's existing work stands and is built on, not discarded or rewritten for authorship.
> - Everything else in the 2026-11-02 ruling (Track B critical-path scope, device-window lines, RAM-boot envelope) is unchanged.
>
> **OPERATOR RULING 2026-11-02 — TWO TRACKS (read first; overrides older scope text below).**
> - **Track A — agentic OS now:** build the agentic layer (agent runtime, Lego service blocks,
>   open-kimono UI, power control, voice) on the postmarketOS/Linux install already on the OnePlus 6,
>   as portable Rust services behind a small capability interface so they can move to Redox later.
> - **Track B — Redox, critical path only:** continue Redox bring-up only along the shortest path
>   to a USB serial shell on the phone (P5 stages 1–3), which removes the operator from the device
>   test loop. Work off that path is parked.
> - **Parked:** the Rust rewrite of U-Boot/QEMU (including the phone slice of 2026-11-01). The
>   existing C U-Boot/QEMU are the tools. "ALL upstream" scope text below is historical.
>
> **SETTLED INTENT 2026-11-03 — see [INTENT.md](INTENT.md) (source of truth for what the
> phone should be).** In brief: a completely AI-native companion — voice/text-first, does
> exactly as directed, reuses deterministic scripts and generates blocks on the fly,
> waveform (on by default, even asleep; user can turn mic+waveform fully off) + detailed
> console only, anticipation asks first, everything synced to the homelab, cloud models
> help with complex jobs under a visible hand-off, full kernel-to-display power control
> with drain-estimated recommendations, user-authored crons, continuously evolving
> patterns. Inference: on-phone small LFM (pinned `LFM3-1.7B` baseline, newer LFMs are
> evaluation-only), Asus ROG Ally as changeable fast lane, homelab Spark/5090/3090 via
> gateway with models never changed by this project. Older "Rust migration precedes
> deployment" and "ALL upstream" text below is historical where it conflicts.


Latest clarification: Rust migration precedes deployment. The intervening request to prioritize a revised physical kernel is superseded; hardware testing remains stopped while the approved rewrite proceeds.

Operator priority, 2026-10-27 5:46 PM PT: unattended troubleshooting and human-like
operation must be first-class capabilities. Provide retrievable failure logs,
screenshots, direct command input and CLI equivalents for every screen action
(tap, swipe, key/button and text entry). The agent must observe, act and verify
the result through these interfaces. Source/mock tests do not establish a live
phone connection. Rust host contracts and fake-transport tests may proceed
independently of the brain; physical validation still waits for explicit resumption.

## Current operator decision — systematic Rust rewrite first

Initial on-device brain decision (2026-10-27): ONLY `LiquidAI/LFM3-1.7B`, https://huggingface.co/LiquidAI/LFM3-1.7B. Build a custom native Rust inference/serving path for OnePlus 6 and Redox. Do not trial, benchmark or silently fall back to another brain model; later models need a new operator decision. Quantizations must derive from this exact checkpoint with provenance. This choice does not change the fleet models serving development agents. Device-free brain work may run concurrently with the rewrite; physical execution remains STOPPED.

Specialization decision (2026-10-27): later phases MUST adapt this same model to our actual uses, starting with supervised LoRA fine-tuning. Curate real, authorized task/tool/result/correction examples from the phone OS and its host development workflow; keep an untouched base-model comparison and held-out workflows. Build the new data, training, adapter and evaluation code in Rust. Training is a bounded host GPU job admitted separately against live capacity, not automatic learning on the phone or permission to displace served development models. Promote a reproducible adapter only after measured task improvement and regression checks; retain rollback to the unchanged pinned base.

Scope answer received: ALL upstream U-Boot and QEMU features/platforms, plus every feature built in this project. All new project implementation, generators and tests are Rust. OnePlus-only parity does not complete this objective. Exact version/feature matrices, retained external dependencies and unsupported-target gaps remain explicit; nothing is silently omitted.

2026-10-27: STOP previous hardware/emulator feature progression. Rewrite the full upstream U-Boot/QEMU and custom stack in Rust systematically before resuming delivery. Includes native replacements and project tooling/tests, not wrappers. Breadth is confirmed above; firmware/assembly/toolchain contracts still need explicit inventory. Preserve device state, private sources and evidence. Current PLAN/HANDOFF supersede historical priorities below.


## Latest recovery decision — 2026-10-26 08:25 AM PT

Operator now accepts manual phone reboot/button recovery when needed during tests. Cheap non-invasive remote button hardware is optional research, not a prerequisite for physical kernel testing. Do not open the phone or wire contacts. Budget undecided; research only, no purchase. This supersedes the earlier fully-remote-before-every-test requirement below. Primary Redox target, disposable prototype permission, private off-device source preservation, diagnostic capture and stop-for-agent-review failure policy, and first boot/CPU/timer/interrupt/log milestone remain.

## Physical-device intent — operator interview, 2026-10-26

- Main target is our Redox kernel and OS on the physical OnePlus6. A minimal recovery/test environment supports loading, observing and recovering test kernels; developing custom Linux kernels is not a parallel primary target.
- This is a disposable prototype. Operator authorizes erasing/repartitioning the current installation; preserving the present Linux desktop is not a requirement. Essential device recovery information/backups and custom source remain protected.
- All custom source, patches and reproducible build configuration must be preserved off-device in private GitHub before destructive changes. Verified private repositories OWNER/redox-oneplus6 and OWNER/qemu-oneplus6 had remote heads matching local9990b4c andc51d86c7 respectively. Custom U-Boot commits48bbc236/3e15c281 are preserved as patches/u-boot/0001–0002 in the private Redox repo even though the working U-Boot checkout points at upstream. This audit is source/recipe coverage; it is not a claim every local binary/image has been uploaded.
- Manual button recovery is accepted for hung tests. Optional remote recovery hardware must attach externally without opening/wiring the phone. Budget undecided; research only, no purchase authorized.
- Failure policy: capture diagnostics, return to recovery, and stop for agent review. No automatic retry of the failed hardware test.
- First physical Redox milestone: boot plus CPU/timer/interrupt diagnostics, useful persistent logs, and reliable return to recovery. Display/touch/buttons/UFS emulator successes are later hardware acceptance gates; they are not required all at once in the first milestone.
- Two successful unattended normal Linux reboots prove the existing baseline only. They do not prove hung-kernel/bootloader/EDL recovery. Admit physical RAM-only tests one at a time, with capture/stop and manual recovery if required.


Operator intent (2026-10-24), the bar every design decision is checked against.

1. **The language model is the operating layer.** The user talks or types; models interpret intent and do
   whatever is asked — answer, act, or build. No app grid as the primary interface.
2. **Lego-style, modifiable on the fly.** Services, routines and apps are small composable blocks. The user
   creates them by asking, and changes, replaces or removes them the same way while the system runs.
3. **Open kimono — everything visible.** Like the DSH harness: the user sees that the mic is recording, the
   live transcript, the cleaned text, the model's reasoning, every tool call, every service and its state, in
   real time — and can inspect or change any of it by voice or text.
4. **Intelligent power.** Components (radios, sensors, models, services) switch on and off as needed; the system
   says what it switched and why.
5. **Honest and explained.** Says when it can't do something and why; announces homelab/cloud hand-offs and waits
   visibly; never silent.
6. **Shared and continuously improved.** Everything users build — services, routines, drivers, fixes — is
   shareable with the community; the OS improves from what people make.
7. **Reproducible on other hardware.** Hardware profiles + provisioning as code; any device can join.

8. **Minimal by design.** Run only what the user's request needs, nothing more: navigation = the map engine +
   the already-downloaded maps, nothing else. Offline/local first; no background services, no data leaving the
   device unless the task truly requires it (and then it says so). Fewest cores, least RAM, least power; everything
   else stays off. A clean, minimal view showing exactly what's happening — no clutter.

## Historical delivery approach — suspended during Rust rewrite
Build everything that QEMU can run in QEMU (kernel, userspace, the language-native layer, UI). Phone-specific
drivers can't be emulated (QEMU has no SDM845 model): develop them against register models replayed from traces
recorded on the real phone, then confirm on hardware.

## How Redox fits
Redox makes every OS service a separate user-space process behind a named scheme (`display.vesa`, `input`,
`disk.*`, …). That is the Lego substrate: the model composes, starts, stops and inspects schemes; a trace/
observability scheme exposes every event to the UI; power policy is starting/stopping scheme daemons.

## What already exists (the "taste")
Voice app on postmarketOS: waveform → live STT → cleaned text → streaming reasoning → spoken answer; mini-apps
generated and promoted into a library; native phone functions behind a broker; triggers/routines; homelab
fallback that explains itself (`~/Developer/experiments/voice-stack-tryout`).

## Parallel delivery model

Physical hardware validation and emulator/kernel development are parallel tracks. A physical recovery wait must not stop useful device-free development. One existing Perplexity Computer session on local pplx-5090 coordinates both, delegates bounded independent work to admitted5090/Spark lanes, and collects evidence. Codex supervises acceptance and serialized phone commands. Emulator proof never substitutes for physical proof.
