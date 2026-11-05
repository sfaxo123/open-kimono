# INTENT — settled operator intent for the OnePlus 6 agentic OS

> Settled 2026-11-03 from operator interview (7 decisions) plus fine points
> confirmed the same day. This file is the source of truth for *what the
> phone should be*. VISION.md explains why, SPEC.md states the testable
> contract, PLAN.md states the executable phases. Where older text below
> this intent disagrees, this intent wins; history is preserved, not deleted.

## One line

A completely AI-native digital companion on the OnePlus 6: you talk or
type what you want, it does exactly that — reusing a deterministic script
when one exists, generating one on the fly when it does not — while it
learns your patterns, anticipates with permission, and controls the whole
device from kernel to display so only what you need is ever running or shown.

## Interaction contract

- **Voice or text first.** No app grid as the primary interface. Speech and
  typed chat are the same channel: intent in, action out.
- **Does exactly as directed.** The instruction is the spec for that task.
  Clarify only when acting on a guess could commit something irreversible
  (see Safety line); otherwise act and show the result.
- **Deterministic first.** If a tested script, routine, or Lego block
  already does the job, run it. Generation is the fallback, not the default.
- **Generate on the fly.** When nothing exists, the agent builds a small
  composable block (service/routine/mini-app) using the inference tiers
  below, runs it sandboxed, logs the build in the console, and — when the
  user keeps it — promotes it into the deterministic library so next time
  it is reused, not regenerated.
- **Brief co-pilot voice.** Spoken replies are short and warm, like a good
  passenger. Detail lives in the console; voice never narrates the log.

## The shell: waveform + console

- Idle screen is only two things: a **waveform** and a **console**.
- The waveform is the listening indicator. It is **on by default, including
  while the phone sleeps**, so the user can see at a glance whether the
  phone is listening. It shows state, not content.
- The console shows everything, open-kimono: live transcript, cleaned
  text, model reasoning summary, every tool call, every service and its
  state, every power switch and why — inspectable and changeable by voice
  or text in real time.
- Requested content (map, video, message, photo) takes the full screen
  while it is needed, then clears back to waveform + console. Nothing else
  is displayed. Only what the user desires at the time is shown.
- **The user can turn the mic and the waveform fully off at any time** —
  by voice, text, or control. While off, the phone does not listen. Turning
  them back on is equally direct. This right overrides every default below.

## Anticipation: ask first, then act

- The agent continuously analyzes usage patterns (routines, routes,
  contacts, media taste, component drain) synced to the homelab.
- A prediction is a **question**, not an action: car Bluetooth connects →
  "Going to Tabby's house, the owner?" Only on yes does it start directions
  and play the YouTube video the user would like, in the background,
  conversationally.
- Wrong predictions are data: dismissal, correction, and "forget that
  pattern" all feed the next estimate. Learned behavior is versioned,
  console-logged, and reversible — nothing learned is silent or permanent.

## Inference tiers (where thinking runs)

1. **On-phone small model — fast backup inference.** Goal: instant replies
   and continued operation when homelab and cloud are unreachable. The
   pinned baseline is `LiquidAI/LFM3-1.7B` (SPEC.md). Newer LFM models are
   **evaluation candidates**: benchmark them on phone hardware (speed,
   RAM, thermal, drain, held-out task quality) against the pinned
   baseline. A candidate replaces the pin only by a new operator decision
   — never silently, never as a fallback.
2. **Asus ROG Ally — fast nearby inference.** A changeable test surface:
   models on the Ally may be swapped freely for evaluation and fast
   inference serving the phone.
3. **Homelab (3090 / 5090 / dual Spark) — hard and build tasks.** Consumed
   through the existing LiteLLM gateway only. **This project never changes
   the models served on Spark, the 5090, or the 3090** — not for tests,
   not for convenience. Fleet-lane state is verified by read-only probe.
4. **Cloud models — complex jobs.** Used to help with complex work (app
   generation, hard reasoning, research). Every cloud hand-off is
   announced visibly ("sending this to cloud") before data leaves; the
   user can forbid it per task.

Coordination with the Roomba brain transplant: share the evaluation
harness, tokenizer/serving contracts, and results format so measurements
are comparable. Each device keeps its own pin decision; a result on one
device never re-pins the other.

## Sync: everything to the homelab

- Learned patterns, routines, generated blocks, crons, settings, logs,
  and evaluation results are **synced to the homelab** and survive a phone
  reset or replacement.
- Synced data is visible and deletable from the console. Sync is not a
  black box: what synced, when, and what it changed is in the log.
- Offline/local first: a task runs on-device unless it truly needs a
  higher tier, and then it says so (see tiers).

## Power: full control, aggressively explained

- The agent manages battery, radios, sensors, models, services, and
  display — kernel to UI — turning each on only for the task that needs
  it and off again after.
- It **recommends** power moves from measured evidence: per-component
  drain estimated from previous usage, current pattern, and estimated
  time to next charge. Example shape: "Mic + waveform is costing about
  what it cost last night and you usually charge at 7 AM — turn listening
  off till 6?" The numbers cite the measured history they came from; a
  recommendation with no baseline is not made.
- Recommendations are advice. Auto-switching happens only for moves the
  user pre-approved, and every switch — recommended, approved, or
  automatic — is announced with what changed and why.

## Customization and crons

- Everything is customizable by asking: routines, blocks, thresholds,
  voice, what is learned, what is synced, what may auto-run.
- **Crons run from user intent.** "Every Saturday…", "when I get in the
  car…", "remind me…" create, edit, pause, and delete scheduled and
  triggered routines in conversation. The schedule is user-authored, not
  vendor-default, and the full cron list is inspectable in the console.

## Continuous evolution

Pattern analysis is a standing loop, not a one-time profile: routines,
drain estimates, and anticipation thresholds are re-fit from new usage.
Every behavior change that results is versioned in the homelab sync,
logged in the console with its evidence, and reversible. The safety line
below never evolves.

## Safety line (never evolves, never learned away)

- Free without asking: read, play, navigate, draft, build, and run
  sandboxed generated blocks.
- **Ask to commit, always:** sending messages, placing calls, spending
  money, sharing location, deleting data, or any action that affects
  other people. A generated block that wants to do one of these hits the
  same gate — "build and run silently" never includes committing.
- Honest and explained: says when it cannot do something and why, names
  homelab/cloud hand-offs before they happen, never silent.

## Minimal by design

Run only what the request needs — navigation is the map engine plus the
already-downloaded maps, nothing else. Fewest cores, least RAM, least
power; everything else stays off. The easiest phone to use because it
analyzes usage and adapts by coding what the user wants.

## Two tracks (unchanged ruling, 2026-11-02)

- **Track A — agentic OS now:** this intent is built first as portable
  Rust services on the postmarketOS/Linux install already on the phone,
  behind a small capability interface so it moves to Redox later.
- **Track B — Redox critical path:** shortest path to a USB serial shell
  on the phone (P5 stages 1–3) to remove the operator from the device
  test loop. Off-path Redox work, including the Rust U-Boot/QEMU rewrite,
  stays parked; existing C U-Boot/QEMU are the tools.
