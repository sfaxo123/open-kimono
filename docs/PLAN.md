# PLAN — full upstream U-Boot/QEMU Rust rewrite before delivery resumes

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
> **SETTLED INTENT 2026-11-03 — [INTENT.md](INTENT.md) / [SPEC.md](SPEC.md) SC8–SC12 govern
> Track A.** Phases A1–A5 below are the active Track A plan; Track B remains the P5
> serial-shell gate (STATUS.md TONIGHT chain). Historical Rust-rewrite phases 0–8 below
> are parked/superseded where they conflict. Fleet rule: never change models on dual
> Spark, 5090, or 3090; Ally and on-phone models are the changeable evaluation surfaces.

## Track A — agentic OS on postmarketOS (portable Rust, capability interface)

**Goal:** INTENT.md live on the phone already in service. **E2E:** voice/text request → deterministic-or-generated block → observed result in console, with homelab sync.

- [ ] **A1.1** — Waveform+console shell with mic/waveform control: idle shows only waveform+console, waveform truthfully reflects listening (incl. asleep), user off/on stops/starts listening — verify: `cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/apps/kimono-ui/Cargo.toml` [lane:5090] (reopened 11-03: unwired/vacuous — see docs/track-a/WIRE-STATUS.md)
- [ ] **A1.2** — Agent runtime + capability broker: voice/text intent dispatch, deterministic-block lookup before generation, brief co-pilot replies with detail in console — verify: `cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/apps/agentd/Cargo.toml` [lane:5090] (reopened 11-03: unwired/vacuous — see docs/track-a/WIRE-STATUS.md)
- [ ] **A1.GATE** — Shell+runtime gate: ask-first and ask-to-commit enforced in tests incl. generated-block commit attempts — verify: `bash -c 'cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/apps/agentd/Cargo.toml && cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/apps/kimono-ui/Cargo.toml'` (reopened 11-03: unwired/vacuous — see docs/track-a/WIRE-STATUS.md)

- [ ] **A2.1** — Pattern store with homelab sync: routines/routes/taste versioned, synced, survivable across reset, forget-pattern reversal — verify: `cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/crates/caps-check/Cargo.toml` [lane:5090] (reopened 11-03: unwired/vacuous — see docs/track-a/WIRE-STATUS.md)
- [ ] **A2.2** — Anticipation engine: context triggers (e.g. car Bluetooth) produce ask-first prompts only; dismissal/correction re-fits thresholds — verify: `cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/tools/user-sim/Cargo.toml` [lane:spark] (reopened 11-03: unwired/vacuous — see docs/track-a/WIRE-STATUS.md)
- [ ] **A2.GATE** — Learning gate: learned change is console-logged with evidence and reversible; no silent behavior change — verify: `bash -c 'cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/tools/user-sim/Cargo.toml'` (reopened 11-03: unwired/vacuous — see docs/track-a/WIRE-STATUS.md)

- [ ] **A3.1** — Lego generator: sandboxed on-the-fly block creation via tiered inference, console-logged build, promote-on-keep to deterministic library — verify: `cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/apps/sandbox-demo/Cargo.toml` [lane:5090] (reopened 11-03: unwired/vacuous — see docs/track-a/WIRE-STATUS.md)
- [ ] **A3.2** — Cron/customization engine: user-authored schedules/triggers by voice/text, inspectable cron list, everything customizable in conversation — verify: `cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/apps/agentd/Cargo.toml --test cron_engine` [lane:5090] (reopened 11-03: unwired/vacuous — see docs/track-a/WIRE-STATUS.md)
- [ ] **A3.GATE** — Generation gate: reused deterministic block preferred over regeneration in tests; cloud hand-off announced before any cloud call — verify: `bash -c 'cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/apps/sandbox-demo/Cargo.toml'` (reopened 11-03: unwired/vacuous — see docs/track-a/WIRE-STATUS.md)

- [ ] **A4.1** — Drain analytics: per-component (mic/waveform, radios, models, display) drain measured from previous usage with time-to-next-charge estimate; measure-and-report until baseline exists — verify: `cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/apps/agentd/Cargo.toml --test drain_analytics` [lane:5090] (reopened 11-03: unwired/vacuous — see docs/track-a/WIRE-STATUS.md)
- [ ] **A4.2** — Power manager: aggressive explained switching of radios/models/services/display, recommendations cite measured history, auto-switch only within pre-approval — verify: `cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/apps/agentd/Cargo.toml --test power_manager` [lane:5090] (reopened 11-03: unwired/vacuous — see docs/track-a/WIRE-STATUS.md)
- [ ] **A4.GATE** — Power gate: recommendation without measured baseline is refused; mic-off state blocks all listening in tests — verify: `bash -c 'cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/apps/agentd/Cargo.toml --test drain_analytics && cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/apps/agentd/Cargo.toml --test power_manager'` (reopened 11-03: unwired/vacuous — see docs/track-a/WIRE-STATUS.md)

### Conversational voice workstream (adopted 2026-11-03)

Goal: make the phone conversational, not command-and-control. Research and
full integration plan: docs/workbench/conversational-voice/RESEARCH.md and
INTEGRATION.md. Test sketch: docs/workbench/user-simulations/key-tests/
KEY-TEST-2-CONVERSATION.md. Operator decision points (INTEGRATION.md S3) are
presented one at a time. Decision 1 DECIDED 2026-11-03: follow-up window about 10 s. Decision 2 DECIDED 2026-11-03: both, cascade first (duplex pilot last, gated).

- [x] **CV0** — Kill the dead air: warm STT/TTS processes, instant ack clip, VAD endpointing on the phrase-only path
- [x] **CV1** — Conversation session: follow-up window (~10 s, operator-decided 2026-11-03) + agentd session state
- [x] **CV2** — Smart Turn v3 endpointing on-device + ASR-confidence repair strata composed with BRAINFIX
- [x] **CV3** — Streaming pipeline: token-streamed brain, sentence-chunked TTS; preemptive start as tightening
- [ ] **CV4** — Barge-in lite (duck-and-confirm, echo guard); real AEC as the tightening pass
- [ ] **CV5** — Kokoro-82M vs Piper measured on this phone + text-side prosody rules
- [ ] **CV6** — Gated homelab duplex pilot (split-brain, 3090-served), explicit kill criterion
## Phase M — on-device LFM evaluation (Ally + phone; fleet lanes immutable)

**Goal:** fast on-device backup inference if achievable, decided by measurement. **E2E:** same held-out tasks scored on pinned baseline vs candidates, on phone and Ally.

- [ ] **M1.1** — Candidate inventory: current/new LFM models vs pinned `LFM3-1.7B` config/tokenizer provenance; Roomba transplant harness/results imported for comparability — verify: `bash -c 'test -f $HOME/Developer/experiments/redox-oneplus6/docs/workbench/LFM-EVAL.md && grep -q LFM3-1.7B $HOME/Developer/experiments/redox-oneplus6/docs/workbench/LFM-EVAL.md'` [lane:spark] (reopened 11-03: unwired/vacuous — see docs/track-a/WIRE-STATUS.md)
- [ ] **M1.2** — Phone benchmark harness: tokens/s, time-to-first-token, RAM, thermal, drain on OnePlus 6 hardware for baseline+candidates (measure and report; no threshold before baseline) — verify: `cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/crates/lfm3-brain/Cargo.toml --test ondevice_bench` [lane:5090] (reopened 11-03: unwired/vacuous — see docs/track-a/WIRE-STATUS.md)
- [ ] **M1.3** — Ally benchmark + serving: same harness on Asus ROG Ally (models changeable), phone consumes Ally as fast lane — verify: `bash -c 'cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/crates/lfm3-brain/Cargo.toml --test ally_bench'` [lane:5090] (reopened 11-03: unwired/vacuous — see docs/track-a/WIRE-STATUS.md)
- [ ] **M1.GATE** — Evaluation gate: report compares baseline vs candidates on held-out actual-use tasks; fleet immutability proven by read-only probe before/after; promotion requires new operator decision (no auto re-pin) — verify: `bash -c 'fleet-model-status --map > /tmp/fleet-before.txt && cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/crates/lfm3-brain/Cargo.toml --test ondevice_bench'` (reopened 11-03: unwired/vacuous — see docs/track-a/WIRE-STATUS.md)


Monitor checkpoint, 2026-11-01 1:23 PM PT: ff235b9 stages the SAME rejected
P4 image c626d16e... without a build repair. P4-FEATURE-01 remains open;
attended pinning does not close compiled-hook proof. See current HANDOFF.

Monitor checkpoint, 2026-11-01 9:46 AM PT: P4 feature-on acceptance REJECTED
(P4-FEATURE-01): Make reused a kernel across changed feature env; saved logs
show forwarding/reset but no PON hook. Existing root owner must repair build
identity/invalidation and supply guarded-code proof (HANDOFF). No phase tick.

Monitor checkpoint, 2026-11-01 8:50 AM PT: operator ruling0735 keeps0047 with
Remy/root; ownership question resolved.0047 independently13PASS with alignment/
overflow mutations RED and restoration GREEN, source guard closed narrowly.
New Python packager adaptations cb6538e do not satisfy Rust-only generator
acceptance; native Rust parity remains needed. Full phase gates remain OPEN.

Monitor checkpoint, 2026-11-01 4:15 AM PT:0047 still undelivered; no running
DSH implementation. Live atomic-paused capability is true; the execution
blocker is parked admission/direct-root ownership, not missing capability.
Next packet and unchanged acceptance gates are in HANDOFF; no expired revival.

Monitor checkpoint, 2026-11-01 1:34 AM PT: saved P3 gate serial hashes audited,
watchdog pure layer9PASS with readback mutation RED/restoration GREEN. New
WDT-BOUNDS-01 source boundary needs owner review (HANDOFF); no phase checkbox
advanced, physical execution remains stopped for this monitor.

Monitor checkpoint, 2026-11-01 12:39 AM PT:0041 correction +0044 PON glue fix
independently reviewed; host26PASS plus actual target/extent mutations RED,
restored GREEN. Host findings closed narrowly; full kernel-build/glue/runtime
gates remain separate and root-owned. No full Rust phase advanced (HANDOFF).

Monitor checkpoint, 2026-10-31 11:44 PM PT: root2ed6430 accepts P4 findings,
holds0040 unbuilt pending0041 address/span validation. Added independent host
write/readback denial controls and error-discard mutation proof (HANDOFF/TESTS);
no production source/gate change. Existing owner retains correction ownership.

Monitor review, 2026-10-31 9:54 PM PT: new kernel0040 host controls reject its
malformed-address acceptance (2FAIL despite14 original PASS). Existing P4 owner
must reconcile range checks and unchecked MMIO spans; findings in HANDOFF/TESTS.
No phase gate advanced or physical action authorized by this source review.

Monitor checkpoint, 2026-10-31 8:59 PM PT:0.1.3 and1.GATE stay unchecked.
Main loader18 tests pass; held6cc561f again fails both root error-spoof controls
and the asserted malformed-flag allocation gate. Next tokenizer correction
specification and admission/ownership hold are in canonical HANDOFF. No expired
worker revival or additional correction round follows from this checkpoint.

## Priority addition — observability and CLI operation (2026-10-27 5:46 PM PT)

Root review update8:07PM PT: accepted envelope/log/screenshot CODECS now65PASS,
not full0.2.1 or actual device control. Existing55minute heartbeat now explicitly
requires source-diff review, tests, correction/rejection, push and next bounded
ready task each wake. The same local native queue continues independently of
chat, with original8:23:25PM PT cutoff; new waves need new finite admissions,
never extension of expired goals. Host partial repair currently does not
compile and command result codec is missing; neither is accepted. Preserve
their dirty work, finish smaller slices after ownership/admission review.

Management correction, 2026-10-27 7:25 PM PT: a completed finite packet must
not cause passive idle reporting when useful approved work remains. Keep the
SAME local coordinator executing a concrete bounded queue of reviewed packets,
with live child handles/route/progress and collection receipts. Root reviews
integration. Current accepted device-control suite52PASS includes envelope and
log-body producer/codec only; full0.2.1 stays OPEN. Current active queue and
exact cutoffs live in HANDOFF. The55minute cloud heartbeat is safety supervision,
not proof of continuous execution. Ordinary Codex allowance was unavailable;
the brief experiment with a5minute cloud cadence was reverted, no credits/reset
or new cloud worker admitted. Local execution must persist independently of chat.

Robust retrievable failure logs, screenshots, command execution and CLI-driven
screen actions are priority delivery requirements, independent of the LFM3
brain. The new ready host-contract work below takes scheduling priority over
further brain expansion; preserve the existing tokenizer partial work. Device
implementation still requires relevant source/ABI contracts. No phone action is
admitted: Rust-first/hardware STOP persists. Last direct phone proof remains the
Redox RAM shell; current Linux/screen/USB state is unverified, with no evidence
of a new failure and no operator intervention required now. Historical wording
about recovered Linux does not establish present liveness.

## Latest operator correction — Rust migration before deployment (2026-10-27 1:02 PM PT)

The operator clarified that the revised kernel does NOT need to be deployed first: systematically rewrite the approved stack in Rust, then put it on the prototype phone. This supersedes the intervening immediate-kernel-delivery request and earlier physical parallel-path reopening. Physical execution is STOPPED: no phone commands, restart, RAM boot, installation or new photo request during migration. A completed migration gate does not automatically authorize physical resumption; reconcile the applicable reviews and device state then.

Preserve the successful physical kernel/shell evidence and current recovered Linux. The newly built full31 candidate at $HOME/artifacts/handoff/redox-perplexity/physical-revised31-20261027T1955/package/op6-embedded-efi-redox.img (SHA256 1385d2d1d48a4b88fbcb9b51e424a1badddbf42d9d8c5ff23b9e3ddd6dc78be5) is an UNTRIED, parked legacy-bootstrap artifact, not a completed Rust replacement or admitted next boot. Its package build exited0; no Mac staging or boot occurred. Grok and Perplexity responses exist but contain concrete qualifications to reconcile; AGY returned quota errors and delivered no accepted review. These do not clear the previous three-review requirement.

Continue independent ready Rust inventory, tooling parity and LFM3 foundations in bounded disjoint packets. Full upstream U-Boot/QEMU parity remains the approved scope. Keep the same Perplexity local-E4oCbKIhq98oGmfGXUVMy on pplx-5090, maximum TWO Spark and TWO5090 slots including it, and NO3090 inference/GPU use. Earlier paused workers remain historical, not active work. Rust prevents many memory-safety errors but does not guarantee freedom from leaks, unsafe-code errors or hardware/driver bugs: require allocation/resource accounting, malformed-input tests, unsafe-boundary review and repeated-workload memory measurements at relevant gates. Do not suppress existing entropy/SMP warnings as a migration fix.


Updated 2026-10-27 PT. Operator explicitly stopped the previous phone/emulator delivery plan and made this rewrite the prerequisite. Historical plan/evidence: Git commit1263b74. This is the ONE active plan, with [SPEC](SPEC.md), [acceptance map](TESTS.md), [handoff](HANDOFF.md) and [dependency ledger](workbench/RUST-MIGRATION.md).

## Execution rules

PHYSICAL LOGGING CHECK (2026-10-27 12:52 PM PT): current photos/source receipts support initial warning triage and manual recovery, not complete boot/persistent logging. Before a changed-image test, record exact image/stage identifiers, a bounded whole-boot capture route (proven host sink or operator video fallback), acceptance/failure markers and recovery. Do not assume USB serial, attached durable log storage or post-reboot retention. Preserve the original photos; use device-free source regressions first. No extra reboot solely to recreate existing evidence and no unreviewed phone-storage writes for logs.

3090 OPERATOR RESERVATION (2026-10-27 12:27 PM PT): the3090 inference/GPU lane is reserved exclusively for the operator. Do not dispatch project agents, inference, training, benchmarks or other GPU work to it, even when it appears free. No3090 aliases or auto/pair/unpinned fallback routes that could land there; require a verified5090/Spark route. This overrides older skill suggestions to use spare3090 capacity. Existing gateway/control-plane hosting on the3090 does not authorize inference on its GPU; keep the same native coordinator routed to pplx-5090. Do not change/unload the operator's served3090 model or interrupt their jobs. Project limits remain TWO Spark and TWO5090 slots total, including the coordinator.

PROJECT CONCURRENCY CAP (operator correction, 2026-10-27 12:21 PM PT): at most TWO Spark work slots and TWO 5090 work slots for this entire project, regardless of larger fleet capacity. Count native Perplexity coordinator work, DSH workers/subagents and all aliases on their actual GPU against the same cap;5090-nothink/pplx-5090 do not create extra seats. Count queued/admitted active goals until complete or verified paused/idle, so autonomous wakeups cannot exceed the cap. An active coordinator consumes ONE of the TWO5090 slots, leaving at most ONE5090 worker alongside it; a verified idle coordinator with no queued work may release its slot. Do not spill into3090 or another local lane. Live shared availability/leases may reduce these maxima; unrelated workers are preserved. This supersedes the preceding ONE5090 limit and earlier wider parallel waves, and applies across Rust, LoRA preparation and physical-path preparation. Parallelize only useful ready packets within these limits.

LATEST OPERATOR CHANGE (2026-10-27 12:18 PM PT): reopen physical assessment/preparation in parallel with Rust migration and later LoRA specialization. Operator is returning to photograph the phone. First inspect that current screen and reconcile the ALREADY consumed167e6bda RAM-boot receipt; do not repeat the test to obtain evidence. This supersedes the blanket physical STOP below for assessment/preparation, not automatic execution. Root exclusively verifies current state, ownership, exact image and applicable reviews before any ONE serialized device action. Monitors/Perplexity prepare and record only. No device command is admitted by this readiness message; no blind reboot, flash/partition/slot/EDL/U-Boot reset or protected VM replay. All new implementation remains Rust. This path need not wait for unrelated inventory work and does not complete any Rust rewrite gate or authorize destructive installation.

GLOBAL PARALLELISM POLICY (operator confirmed 2026-10-27): applies to the ENTIRE plan, not only the brain. Maintain a readiness/dependency graph and admit useful independent packets concurrently across live 5090/Spark capacity, with disjoint worktrees/files, bounded goals and one integration owner. Stagger admission RPCs if session-history scanning is slow; execution may overlap after verified admission. Never equate phase numbering with a global execution barrier: independent source inventory, host-tool parity and brain contracts may overlap. Boot/emulator implementation still requires its actual phase1 contracts; pretrained execution requires correct tensor/tokenizer/operator dependencies; final integration/retirement depends on all relevant receipts. Do not parallelize a true dependency, share a writable tree, duplicate a healthy worker, consume reserved3090 or invent quota/capacity. Each local worker uses next/verify for its assigned current-item subpacket; root banks only the complete accepted item. If the serial runner cannot express a separate ready item, keep its evidence unbanked and report the scheduling constraint rather than falsely ticking prerequisite work.

| Work family | Actual prerequisite | May overlap with |
|---|---|---|
| Diagnostics and CLI control (0.2) | current source/interface inventory; bounded Rust host contracts and fake transports | brain and full-scope inventories; real device tests require separately resumed hardware |
| Full source/dependency/feature inventories (1) | verified pinned inputs and ownership | host tooling (2), brain foundations (0), independent architecture reviews |
| Host-tool Rust parity (2) | old behavior captured for the owned slice | other disjoint tools, inventories, brain work |
| Brain config, numerical kernels, service boundary (0.1.1) | pinned LFM3 reference and stable module boundaries | each other, inventories and host tools |
| Brain tensor loader / tokenizer-template (0.1.2/0.1.3) | their respective format/identity contracts | each other and independent kernel/service work |
| Full pretrained inference / daemon wiring (0.1.4/0.1.5) | real tensor/tokenizer/operator/backend dependencies | independent tests, memory accounting and other ready plan packets |
| Rust boot and emulator families (3–5) | accepted phase1 inventory/ABI/architecture contracts | independent architectures, boards, devices and boot/tool families with frozen shared interfaces |
| Reproducibility / retirement (6) | relevant working implementations and receipts | independent artifact/dependency audits; final retirement waits for complete parity |
| Redox brain integration (7) | required Redox transport/runtime interfaces and brain backend | device-free integration preparation; physical tests still require rewrite gates and explicit resumption |
| Actual-use LoRA specialization (8) | reviewed actual-use data, pinned tensor/template contracts, working native Rust training/backend and base evaluation | schema/curation/evaluation design may overlap inventories and inference development; real training waits for validated gradients, data and live GPU admission; phone promotion remains separately gated |

Operator priority 2026-10-27 11:00 AM PT: repair DSH atomic paused-goal execution FIRST. RESOLVED at 11:11 AM PT: rebuilt existing source 2025a03e, restarted the shared server under explicit operator authority, restored all four interrupted sessions and verified no prior session IDs missing. Normal :8771 now advertises the capability. Owned worker dac53156 passed paused creation, guard readiness, exact CAS resume, checked host-file output, stale-CAS rejection and complete/idle cleanup. Source tests: 188 PASS. Full receipts and limitations are in HANDOFF. Rust work may resume through freshly probed admission. Any later restart requires capture/resume of interrupted sessions and pending human instructions. No model change, shared-store second writer or fixed-pin change as a shortcut.

Old physical and feature tracks are STOPPED. Old delivery heartbeat paused and replaced in place with Rust-only supervision; redox-supervisor timer disabled. Do not request another phone photo or issue device commands under this plan. Keep one existing Perplexity local5090 coordinator; no replacement task, no automatic old-task resumption. Root integrates/reviews; bounded workers have disjoint ownership. Probe live lane seats/leases/context before each local admission; preserve reserved3090 and foreign workers. DSH children require live atomic-paused capability and ready/CAS; if absent, repair the execution path first. Shared restarts use explicit operator snapshot/restart/resume authority recorded above.

Scope answer RECEIVED: ALL upstream U-Boot and QEMU features/platforms, plus every custom feature we built. All new project implementation/generator/test code is Rust; frozen non-Rust comparison oracles are temporary and cannot count as replacements. Host candidates and parity fixes may proceed while phase1 inventory/contracts are open; no production caller switch before applicable gates. Full bootloader and native emulator replacement are required; wrapping C executables is not completion. Unsupported architectures or backend/tooling gaps stay explicit blockers, not scope reductions. No performance target or ETA is invented; budgets require measured baselines.

The following commands name real required future test targets. Missing targets are RED/NOT IMPLEMENTED, never PASS. Later phase items must be decomposed into small file-owned implementation packets after phase1 contracts; the milestones below are gates, not permission for an unbounded worker. Local PLAN execution uses `plan-runner.py next $HOME/Developer/experiments/redox-oneplus6/docs` and verify/bank against that same docs directory; commands use absolute manifest paths because runner cwd is docs. Do not create a competing root PLAN.

## Phase 0B — priority diagnostics and CLI control

These are new requirements; only partial envelope framing is implemented.
Source inventory, host contracts
and fake-transport tests are ready independently of brain inference. No new
boot/emulator authoring before phase1 acceptance; no device execution under
this phase. Keep one canonical plan, with disjoint owned subpackets.

- [ ] **0.2.1** — Freeze Rust diagnostics/control protocol and current source/caller inventory: boot/build/action/frame identities, resource bounds, transport pairing, error/unknown-outcome states and a CLI for logs, screenshots, commands and all UI input. Require honest unsupported-transport reporting — verify: `cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/crates/device-control/Cargo.toml --test protocol_contract` [lane:spark]
- [ ] **0.2.2** — Rust bounded log capture/host retention and failure-bundle pipeline with explicit loss/gap/reconnect/panic behavior; no invented persistence guarantee — verify: `cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/crates/device-control/Cargo.toml --test logging_contract` [lane:5090]
- [ ] **0.2.3** — Rust screenshot/command/input CLI and broker contracts over fake transport: framebuffer dimensions/stride/format/freshness, bounded command results, tap/swipe/press/release/text/key semantics, coordinate transforms, cancellation and release of held input — verify: `cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/crates/device-control/Cargo.toml --test interaction_contract` [lane:5090]
- [ ] **0.2.4** — Device-free observe–act–verify integration with fault injection, disconnect/unknown-result/no-blind-replay controls, log/frame/action correlation and repeated-workload allocation accounting; record distinct future real-phone acceptance — verify: `cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/crates/device-control/Cargo.toml --test troubleshooting_workflow` [lane:5090]
- [ ] **0B.GATE** — Phase 0B gate: diagnostics/control contracts green incl. mutation controls — verify: `bash -c 'cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/crates/device-control/Cargo.toml --test protocol_contract'`

The protocol_contract target now tests ONLY partial framing, not the complete
0.2.1 scope:24crate tests pass, but body schemas, callers, pairing/session state
and CLI contracts remain OPEN. workbench/DEVICE-CONTROL-CONTRACT.md records
the wire layout, source evidence and limits. Do not bank0.2.1 from this partial
suite. The other three targets remain absent/NOT IMPLEMENTED/RED. Actual Redox device
transport and end-to-end phone tests remain separate unproved delivery gates.
An ACK or mock screenshot is not evidence of human-like use on the phone.

## Phase 0 — independent LFM3 brain foundation

Operator addition 2026-10-27: the sole initial phone brain is LiquidAI/LFM3-1.7B. This independent Rust service work is explicitly authorized concurrently with migration inventories; it does not relax boot/emulator gates or hardware STOP. Root owns integration; bounded workers may implement disjoint submodules of the same next item, each running next/verify against this canonical plan in its isolated worktree. Only root banks the combined item after reviewing all modules; partial module tests do not complete the item. Do not substitute other model trials or alter fleet inference models.

Milestone0.1.1 banked asf3e92c4 after complete source evidence, scoped dependency review, runner verify PASS and validator COMPLETE (fresh5090 self-lane review, explicitly degraded). Combined suite31PASS/0ignored. This accepts foundation components only; no pretrained inference, full-model parity or phone fit. Numerical math now uses pinned cached Rust libm with architecture/intrinsic paths disabled. Next ordered item0.1.2; independent phase1/host-tool preparation may still overlap through actual readiness gates.

- [x] **0.1.1** — Implement three independent Rust foundations in crates/lfm3-brain: pinned LFM3 config/model policy, scalar numerical reference primitives, and bounded service request/backend contracts; all-features unit/negative tests and audited dependencies — verify: `cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/crates/lfm3-brain/Cargo.toml --all-features -- --quiet` [lane:5090]
- [x] **0.1.2** — Rust safetensors/quantized tensor reader with shape/hash/provenance/bounds validation; bounded metadata parsing and rejection before tensor materialization or inference — verify: `cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/crates/lfm3-brain/Cargo.toml --test tensor_artifacts --test tensor_views --test tensor_adversarial --test tensor_identity --test tensor_quantized --test tensor_quantized_limits --test tensor_quantized_identity` [lane:5090]
- [ ] **0.1.3** — Rust tokenizer and exact pinned chat/tool template port, special-token/stop behavior and streaming tool-call parser with differential fixtures — verify: `cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/crates/lfm3-brain/Cargo.toml --test tokenizer_template` [lane:spark]
- [ ] **0.1.4** — Assemble all LFM3 layer operations, tensor loading, attention/recurrent caches, sampling and real CPU prefill/decode; independent tiny full-model fixtures plus pinned pretrained numerical/token generation proof — verify: `cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/crates/lfm3-brain/Cargo.toml --test pretrained_inference` [lane:5090]
- [ ] **0.1.5** — Wire executable Rust streaming daemon to the actual backend, broker interface, cancellation/budgets and offline artifacts; request-level integration tests reject fake/unavailable inference as success — verify: `cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/crates/lfm3-brain/Cargo.toml --test service_integration` [lane:5090]
- [ ] **0.GATE** — Pinned real-model host generation, numerical/template/tool parity, complete process memory/latency/cache measurements and native-dependency audit; host result is not phone fit or hardware proof — verify: `cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/crates/lfm3-brain/Cargo.toml --test brain_host_gate`

Accepted partial 0.1.2, 2026-10-27 4:01 PM PT: a038119/70f6fb5 provide structural safetensors validation; 94162da/4d6dfa6 add pinned artifact identity, strict expected SHA256, filename/length/format checks and bounded manifest strings. A trusted out-of-band manifest is a precondition, not authenticity/signature proof. Combined suite: 80 PASS, 0 ignored. Header-byte resource policy now precedes hashing after independently reproduced RED. Cached exact sha2 0.10.9 uses force-soft; reachable unsafe Rust casts and compiled target-specific cpufeatures/libc remain explicit audit boundaries, not silently exempted. Release brain/SHA objects had no getauxval imports; ARM cpufeatures did. A tiny 10,000-cycle identity workload measured 3264 KiB process RSS / 0.29 seconds, not exact heap accounting, leak freedom or model fit. tensor_quantized is still NOT IMPLEMENTED/RED, so the combined gate stays OPEN. Next: freeze an explicit quantized format/version, scale/block/shape/hash/base-provenance contract, then implement its native Rust reader and negative tests. Production manifest acquisition, secure file opening, weights, pretrained inference and trainer remain unimplemented. Do not reimplement accepted structure/identity code or bank from a green partial suite.

Latest accepted tensor-reader milestone (2026-10-27 5:24 PM PT): 0.1.2 banked9f5bf1b after actual runner verification. New typed quantized binding f03784e/765f92d requires a private proof from actual validated base bytes and binds model/revision, file/tensor/shape/version, length/hash, recipe and base-manifest identity. Generic ArtifactFormat::Quantized remains rejected; callers use the explicit Q8 typed API. Combined suite109PASS/0ignored, required reader targets78PASS. Root actual-source hash/link mutations separately RED, restored GREEN. Automatic5090 validator saw a truncated evidence snapshot and is only partial/degraded review evidence; root reviewed actual complete source/diffs. Separate Spark review timed out without a verdict: no full independent audit is claimed. Trusted out-of-band typed manifests are an API precondition; authenticity/acquisition, secure file opening, quantizer accuracy, weights, inference and whole-native retirement are NOT accepted. Next0.1.3 tokenizer/template: config/template and the exact17,908,635-byte tokenizer.json are now cached; its SHA matches the pinned Hub LFS digest. workbench/LFM3-TOKENIZER-CONTRACT.md records actual BPE/merges/ByteLevel/lookahead/sentinel requirements. Rust indexing, exact encoding/decoding and template parity remain NOT IMPLEMENTED. Independent full-scope phase1 inventories continue through real readiness gates.

## Phase 1 — stop, inventory and freeze contracts

Goal: every non-Rust component and retained dependency is accounted for. E2E: inventory and operator scope decision produce a complete compatibility/ownership ledger.

- [x] **1.1.1** — Stop old autonomous work and preserve accepted baseline; root verifies automation PAUSED, supervisor inactive/disabled and native task idle. Recorded in HANDOFF; verify: `bash -c 'test "$(systemctl --user is-enabled redox-supervisor.timer)" = disabled && ! systemctl --user is-active --quiet redox-supervisor.timer'` [lane:inline]
- [ ] **1.1.2** — Rust inventory validator enumerates tracked Python/shell/embedded C/FFI/native dependencies and reconciles ledger entries without omissions — verify: `cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/tools/rust-migration/Cargo.toml --test inventory_contract` [lane:5090]
- [ ] **1.1.3** — Validate the CONFIRMED full-upstream scope against exact upstream/downstream refs, every architecture/platform/feature and explicit firmware/assembly/toolchain boundaries; reject omitted or silently exempt components — verify: `cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/tools/rust-migration/Cargo.toml --test scope_contract` [lane:inline]
- [ ] **1.GATE** — Inventory, negative omissions, scope contracts and independent architecture review accepted; no boot/emulator author until this gate — verify: `cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/tools/rust-migration/Cargo.toml --test phase1_gate`

## Phase 2 — replace host tools and test infrastructure

Goal: Rust CLI implementations preserve observable behavior. E2E: owned callers execute Rust binaries with old languages retained only as explicit test oracles.

Accepted bounded numeric subpacket (2026-10-27 1:12 PM PT): bc0cc3b fixes positive ASCII base-0 prefix/separator/leading-zero handling after root RED reproduction and full-diff review. Canonical trace suite17PASS/0ignored, including13 new grammar cases and rejected-output preservation. Full2.1.1 stays OPEN for remaining bounds/signed CLI/whitespace/Unicode/digit-limit/regex/diagnostic contracts; frozen oracle/callers stay unchanged.

- [ ] **2.1.1** — First bounded slice: tools/rust-migration Rust trace converter and differential fixtures for scripts/oracle/trace/convert.py; no legacy delegation in runtime — verify: `cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/tools/rust-migration/Cargo.toml --test python_parity -- --include-ignored` [lane:5090]
- [ ] **2.1.2** — Convert host-only validators, fixture generators and source/patch verifiers in separate reviewed inventory batches — verify: `cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/tools/rust-migration/Cargo.toml --test host_tool_parity` [lane:5090]
- [ ] **2.1.3** — Convert build/packaging/emulator-control/remote-recovery tools using fake transports; preserve bounds/ownership/failure handling and prove no device contact in tests — verify: `cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/tools/rust-migration/Cargo.toml --test control_tool_parity` [lane:5090]
- [ ] **2.1.4** — Convert remaining project tests/generators and wire callers; reject Python/shell fallback in active project paths — verify: `cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/tools/rust-migration/Cargo.toml --test host_retirement` [lane:5090]
- [ ] **2.GATE** — Full owned tooling inventory parity and runtime language/dependency audit — verify: `cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/tools/rust-migration/Cargo.toml --test phase2_gate`

## Phase 3 — Rust boot stage replaces U-Boot

Goal: native Rust replacement of the complete upstream U-Boot surface, not a Rust EFI app hosted by C U-Boot. E2E: architecture/board/driver/command/service/host-tool matrix and custom Redox loading contracts all pass. One Redox boot is a milestone, not full parity.

- [ ] **3.1.1** — Freeze boot ABI and feature contracts: Android container, ARM64 entry, memory, DTB, framebuffer, console, EFI alternative, loading and recovery — verify: `cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/tools/rust-migration/Cargo.toml --test boot_contract` [lane:spark]
- [ ] **3.1.2** — Implement Rust boot primitives and image packer in isolated modules; malformed images/bounds/relocation/handoff negative cases fail — verify: `cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/tools/rust-migration/Cargo.toml --test boot_primitives` [lane:5090]
- [ ] **3.1.3** — Replace every upstream architecture/board/driver/service/command/host-tool interface plus custom SDM845 behavior, using a separately owned packet per inventory family; every unsupported row blocks parity — verify: `cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/tools/rust-migration/Cargo.toml --test boot_feature_parity` [lane:5090]
- [ ] **3.GATE** — Rust boot artifact has no C U-Boot linkage and passes bounded handoff/diagnostic/guest contracts; no phone execution — verify: `cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/tools/rust-migration/Cargo.toml --test phase3_gate`

## Phase 4 — Rust-native multi-architecture execution and machine foundation

Goal: replace QEMU/libqemu/TCG execution rather than C board glue. E2E: every inventoried upstream system/user-mode architecture, execution/acceleration contract, MMU/exception/interrupt/timer model and custom workload has explicit Rust coverage. AArch64/OnePlus is one family, not the final gate.

- [ ] **4.1.1** — Audit Rust execution engines/dependencies for every upstream target, system/user ABI and accelerator; record ISA/privilege/MMU/atomics/vector coverage and missing compiler/host support — verify: `cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/tools/rust-migration/Cargo.toml --test cpu_contract` [lane:spark]
- [ ] **4.1.2** — Implement memory/bus, deterministic clock/events, reset and trace contracts in independent modules — verify: `cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/tools/rust-migration/Cargo.toml --test machine_core` [lane:5090]
- [ ] **4.1.3** — Implement CPU execution, translation, exceptions and required architectural state; independent instruction/MMU/exclusive-access litmus and negative fixtures — verify: `cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/tools/rust-migration/Cargo.toml --test cpu_architecture` [lane:5090]
- [ ] **4.1.4** — Implement timer/GICv3/interrupt delivery and ordering; no unreviewed SMP shortcuts — verify: `cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/tools/rust-migration/Cargo.toml --test interrupt_architecture` [lane:5090]
- [ ] **4.GATE** — Execution foundation passes coverage ledger and native-dependency audit; no C/QEMU subprocess fallback — verify: `cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/tools/rust-migration/Cargo.toml --test phase4_gate`

## Phase 5 — all upstream platforms/devices/interfaces and custom guest compatibility

Goal: every upstream board/device/host/tool feature and every custom capability replaced, with stubs/unsupported cases blocking final parity. E2E: complete frozen upstream matrix plus custom guest/command/input/storage regressions pass on Rust implementations.

- [ ] **5.1.1** — Implement OnePlus map/DTB/boot/PSCI, UART and framebuffer with reset/IRQ/trace parity — verify: `cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/tools/rust-migration/Cargo.toml --test board_console` [lane:5090]
- [ ] **5.1.2** — Replace TLMM/PDC, GENI I2C/touch, SPMI/RPMh, UFS and every remaining inventory device with separate state/reset/IRQ/negative tests — verify: `cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/tools/rust-migration/Cargo.toml --test board_devices` [lane:5090]
- [ ] **5.1.3** — Replace all upstream machines/devices, CLI/serial/display/block/network/control/migration/snapshot/plugin/tool interfaces and host backends, plus custom harness integration, as individually gated owned packets — verify: `cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/tools/rust-migration/Cargo.toml --test emulator_interface_parity` [lane:5090]
- [ ] **5.GATE** — New Rust emulator runs the complete approved guest/device workload; compare traces and fault controls, not banners alone — verify: `cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/tools/rust-migration/Cargo.toml --test phase5_gate`

## Phase 6 — audit, reproducibility and retirement

Goal: systematic rewrite completed against approved scope, with no hidden old-language dependency. E2E: clean build and full parity suite use Rust production paths.

- [ ] **6.1.1** — Clean-environment reproducible artifacts and dependency/unsafe/FFI/license audits; unresolved DTB/firmware provenance remains explicit — verify: `cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/tools/rust-migration/Cargo.toml --test reproducible_stack` [lane:spark]
- [ ] **6.1.2** — Retire old active paths only after receipts, preserve source history/oracle records, verify no runtime C/Python/shell delegation and all callers wired — verify: `cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/tools/rust-migration/Cargo.toml --test stack_retirement` [lane:5090]
- [ ] **6.GATE** — Scope-complete Rust parity, independent review and rollback evidence; report completion and await explicit resumption before any physical work — verify: `cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/tools/rust-migration/Cargo.toml --test phase6_gate`

## Phase 7 — brain integration after rewrite

Goal: custom native Rust LFM3-1.7B brain integrated with Redox userspace, observability, power/lifecycle and the audited action broker. E2E: real host/Redox compatibility first; phone measurements only after a separate explicit physical resumption decision.

- [ ] **7.1.1** — Redox userspace service/transport integration, deterministic offline artifact provisioning and trace/power/broker regression tests — verify: `cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/crates/lfm3-brain/Cargo.toml --test redox_brain_integration` [lane:5090]
- [ ] **7.GATE** — Complete SC7 receipts, distinguish host/Redox/phone levels; physical RAM/ISA/temperature/latency/recovery remain NOT RUN until explicitly admitted, with no alternative-model fallback — verify: `cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/crates/lfm3-brain/Cargo.toml --test brain_delivery_gate`

## Phase 8 — specialize the selected brain from actual uses, LoRA first

Operator addition 2026-10-27: later phases must fine-tune ONLY the pinned LFM3-1.7B for the actual phone OS use case, starting with supervised LoRA. This phase is mandatory. Its numbering does not delay independent data-contract/evaluation preparation; real training requires its actual data/backend prerequisites. No training job, data collection from the stopped phone, model switch or device resumption is admitted by this plan edit. Root integrates; data/evaluation, adapter math and artifact loading are disjoint future worker families. All new implementation/tests are Rust, including training; Python/TRL examples are reference-only. Existing base inference remains an evaluation baseline, not final specialization acceptance.

- [ ] **8.1.1** — Rust actual-use episode schema, provenance/redaction/consent/outcome validation, private manifest and grouped train/validation/test splitting; reject fabricated outcomes, secrets and related-session leakage — verify: `cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/crates/lfm3-brain/Cargo.toml --test specialization_data` [lane:5090]
- [ ] **8.1.2** — Freeze real-workflow base-model evaluation, rubric and predeclared improvement/regression thresholds from measured baselines; tool/schema/clarification/broker outcomes and retention tests use untouched cases — verify: `cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/crates/lfm3-brain/Cargo.toml --test specialization_eval` [lane:spark]
- [ ] **8.1.3** — Native Rust LoRA tensor targeting, forward/backward/update and checkpoint/resume primitives; analytic gradient, frozen-base, zero-adapter and wrong-target/rank negative controls pass on tiny independent fixtures — verify: `cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/crates/lfm3-brain/Cargo.toml --test lora_training` [lane:5090]
- [ ] **8.1.4** — Implement bounded native Rust single-GPU training against curated actual-use data and pinned full-model backend; real run receipts include module map, seeds/config/data/base hashes, frozen weights, loss, measured peak memory, cutoff and clean lease release — verify: `cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/crates/lfm3-brain/Cargo.toml --test lora_host_training` [lane:5090]
- [ ] **8.1.5** — Rust adapter load/export/optional merge and rollback in the actual inference path; reject wrong base/hash/shape/rank, re-evaluate exact merged/quantized artifacts and distinguish fixture tests from real adapter generation — verify: `cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/crates/lfm3-brain/Cargo.toml --test lora_artifacts` [lane:5090]
- [ ] **8.GATE** — Compare pinned base and LoRA on frozen actual-use held-out tasks; accept only predeclared improvement/regression and resource gates with reproducible adapter/rollback/native-dependency receipts; target and physical proof stay separate — verify: `cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/crates/lfm3-brain/Cargo.toml --test specialization_gate`

## Amendments

2026-10-27: operator replaces hardware-first execution with systematic Rust prerequisite, explicitly selects ALL upstream U-Boot/QEMU platforms/features, and requires all custom features carried over and all new implementation/tests written in Rust. Old unchecked delivery tasks are suspended, not completed. Full inventories, dependency/assembly contracts and unsupported-target blockers remain phase1 work; parity breadth is no longer an open question.
