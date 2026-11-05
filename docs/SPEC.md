# SPEC — full upstream U-Boot/QEMU and project Rust migration

Status: operator selected ALL upstream U-Boot and QEMU features/platforms on 2026-10-27 and reinforced that all features we built must carry over and all new code must be Rust. Architecture freeze still requires complete inventories and dependency/ABI contracts. Existing evidence is preserved, not invalidated.

## Objective

Settled intent (2026-11-03, [INTENT.md](INTENT.md) is source of truth): build a completely AI-native companion on the OnePlus 6 — voice/text-directed, deterministic-script reuse with on-the-fly generation, waveform+console shell, ask-first anticipation, homelab-synced learning, tiered inference (on-phone LFM, Ally, homelab gateway, cloud with visible hand-off), and full device power control. Track A delivers this now on postmarketOS as portable Rust services; Track B continues Redox bring-up only to a USB serial shell (P5). Historical objective below (systematic Rust rewrite of all upstream U-Boot/QEMU before delivery) is superseded where it conflicts and retained as history.

Replace the project's non-Rust implementations systematically before resuming Redox feature delivery or physical experiments. The operator explicitly chooses the rewrite over the previous incremental approach. A Rust wrapper around C QEMU, U-Boot, Python, or shell is not a completed replacement.

## Features

### Priority: diagnostics and remote operator interfaces

Implement this work in native Rust alongside other ready migration packets,
before claiming an unattended usable phone. Host-side contracts, source
inventory and Rust fake transports may proceed now; device implementation
obeys relevant inventory/ABI gates and all physical operations remain stopped.

- Structured boot/kernel/userspace logs with build/image/boot identity,
  monotonic sequence/time, severity, subsystem and explicit gaps/drops. Bound
  buffers, expose health/counters and retain failure evidence on the host
  across reconnects. Define early-boot/panic capture and flush limitations;
  do not promise hard-power-loss persistence without actual proof.
- A CLI screenshot operation returns current framebuffer pixels with format,
  dimensions, stride, sequence/time and build/boot identity. Detect stale or
  unavailable frames; never substitute a synthetic picture for device evidence.
- A CLI command channel returns bounded stdout/stderr, exit status, timeout,
  cancellation and an explicit unknown-outcome state after disconnect. No
  blind replay of a possibly executed destructive command.
- Every UI action is addressable through CLI: tap, press/release, long press,
  swipe/drag, text and key events with explicit coordinate/rotation handling.
  Route UI and CLI actions through the same broker/input dispatch so CLI can
  reproduce human operation; do not create an unrelated fake interaction path.
- Correlate action IDs with completion/acknowledgement and a subsequent frame
  or state observation. A transport ACK alone does not prove the intended UI
  effect. Held inputs must release on cancellation/disconnect by defined policy.
- Paired authorized host access should avoid repeated interactive password
  entry after provisioning, while retaining device identity, authorization,
  command/input bounds and private log retention/redaction. No open remote shell
  or unauthenticated input endpoint as a convenience shortcut.

Physical acceptance must independently prove logs and command/screenshot/input
on the real Redox phone, then an observe–act–verify workflow and recovery of
evidence after failure. This does not prove remote power-button recovery.

### In scope

- Keep and improve existing Rust kernel, drivers, loader and userspace; audit unsafe boundaries rather than translating Rust again.
- Replace every project-owned Python/shell executable, test harness and generator with Rust implementations and Rust tests.
- Replace the C U-Boot stage and custom C/assembly glue with a Rust boot stage, including loading, memory/DTB/framebuffer handoff and required recovery interfaces.
- Replace QEMU execution/device dependencies with a Rust-native emulator: CPU execution, MMU, exceptions, virtual clock, GIC, board, devices and host interfaces. Porting only board glue does not satisfy this.
- Classify embedded C inside patch files, generated code, FFI, build scripts, transitive native libraries, host tools and shared homelab dependencies.
- Retain old implementations as read-only comparison oracles during migration, then retire active call sites at tested gates. An oracle does not count as the new implementation.
- Preserve private source, fixtures, provenance, device backups and accepted receipts. No new repository.
- Full U-Boot architecture, board, driver, command, boot/security, sandbox, build-tool and test coverage; full QEMU system/user emulation, execution engines, architectures, machines, devices, accelerators, host backends, protocols, tools and tests. No OnePlus-only completion gate.
- Every custom capability built in this project remains a separate traceable acceptance row, including loader, console/UART, interrupts/timers, input/touch/buttons, storage, networking/protocols, userspace, harnesses and recovery safeguards.
- All NEW project implementation, generators and test logic must be Rust. Existing non-Rust code may remain as frozen comparison oracles during migration, not new production logic. No new Python/shell/C rewrite helpers or test implementations.

This also covers transient/inline fixture-generation or project test logic: do not introduce Python hashing/generation snippets as a shortcut. Use Rust fixture generators. Existing, unchanged non-Rust comparison oracles must be identified with their versions and kept outside runtime/test implementation claims; tool invocation does not make an oracle a Rust replacement.

### Boundaries requiring explicit resolution

RESOLVED: the operator selected every upstream U-Boot/QEMU platform and feature, not merely all features currently used on OnePlus 6. Pin complete upstream and downstream revisions, enumerate their entire compatibility surface, and track later upstream deltas separately so the baseline does not move during a worker packet. The current local snapshots are starting inventory inputs, not proof that newer upstream features have been inventoried.

Qualcomm PBL/XBL/ABL firmware, minimal startup/exception assembly, Rust's compiler/linker/system libraries, host OS, and shared vendor/homelab agents are recorded dependencies, not claimed rewritten. Their retention is a proposed boundary, not an approved Rust-only exemption. Resolve this during phase 1 before freezing completion claims or changing boot/emulator architecture.

## Tech Stack

### OnePlus 6 brain — single initial model and native Rust runtime

Only `LiquidAI/LFM3-1.7B` is admitted for the initial brain. Pinned source/config reference: Hugging Face revision `40ebe0aa6662c9d40389c2f5d565b6a2c6aebacd`, observed 2026-10-27; future weight/tokenizer artifacts must carry exact revision, filenames, hashes, format and quantization provenance. No alternate model or remote fallback without a later operator decision. Keep existing fleet inference lanes unchanged.

Implement native Rust weight parsing, tokenizer/chat-template behavior, CPU execution and serving, with audited Rust dependencies. A Rust HTTP wrapper around llama.cpp/Ollama/ONNX Runtime, Python/Transformers, an FFI engine or remote inference does not meet this requirement. Frozen upstream implementations can supply differential reference fixtures only, not production execution. First establish portable CPU arithmetic; optional ISA/GPU acceleration requires explicit detection, numerical parity and measured benefit. Linux-host compatibility and Redox userspace compatibility are separate targets.

The pinned official config requires 24 layers (13 gated short-convolution, 5 sliding-window attention, 6 full attention), hidden size 2048, 16 query/4 KV heads, head dimension 128, intermediate size 7680, convolution cache 7, sliding window 256, sandwich normalization and layer-specific/partial RoPE. Implement and test those actual operations, tensor shapes, state/cache resets, tokenizer, stop IDs and tool-call serialization; do not substitute a generic transformer. Official config allows 262,144 context positions; that is NOT the phone's serving budget. Select a bounded context only after measuring weights, recurrent/KV state, scratch allocations and the complete service stack. No RAM-fit or latency claim from parameter count, quantized file size or vendor benchmarks alone.

Serve through an explicit Rust backend interface and bounded streaming request/response protocol, with cancellation/deadlines, context/output limits, admission/backpressure, observable load/error state and traceable tool proposals. Only an audited broker may execute tools; generated text must never execute commands directly. Future model extensions may use the interface, but only the initial model ID is accepted now. A missing/incomplete backend returns an explicit unavailable error; synthetic fixtures, mocks and arithmetic tests never count as pretrained-model generation. Default operation stays offline/local. No listener or model service starts merely by running contract tests.

Authoritative inputs: [model card](https://huggingface.co/LiquidAI/LFM3-1.7B/blob/40ebe0aa6662c9d40389c2f5d565b6a2c6aebacd/README.md), [configuration](https://huggingface.co/LiquidAI/LFM3-1.7B/blob/40ebe0aa6662c9d40389c2f5d565b6a2c6aebacd/config.json), reference model/conv sources, tokenizer/template and LFM Open License v1.0. Metadata/source were retrieved to artifacts/handoff/redox-perplexity/lfm3-source; no weights downloaded or upstream Python executed. Preserve notices and review distribution terms before packaging derived artifacts.

### Later specialization — actual-use supervised LoRA first

Operator addition 2026-10-27: specialize the SAME pinned LFM3-1.7B using LoRA as the first adaptation method. Do not begin with full-parameter fine-tuning, substitute another model, silently add preference/RL training, or start a training run from this planning change. The unmodified checkpoint remains the reproducible baseline. Derived adapters are versions of this selected model, not authorization for alternative brain trials.

Capture only authorized actual task episodes: user intent, model/tool-schema versions, observable inputs, proposed calls, broker decisions, actual tool results, verified outcome and explicit corrections. Examples include composing/inspecting services, structured tool arguments, lifecycle/power requests, error handling and follow-up clarification when those workflows actually exist. A model's claimed success is not an outcome label. Do not fabricate phone outcomes, collect hidden reasoning, or train directly on unreviewed raw transcripts. Redact secrets and unnecessary personal data, record provenance/consent and deletion lineage, and store private off-phone manifests with content hashes. Read-only data/schema/evaluation preparation may overlap other independent work; physical collection remains STOPPED.

Curate supervised demonstrations and corrected failure cases with versioned chat/tool templates and loss masks. Split by task/session/workflow lineage BEFORE augmentation; deduplicate related examples across training, validation and an untouched test set. Freeze actual-use evaluation cases and the scoring protocol before tuning, covering task success from real tool results, tool/schema validity, required clarification, broker refusals and general conversation retention. Training examples cannot serve as held-out acceptance evidence. Data sufficiency and acceptance thresholds must be explicitly recorded from measured baselines before a run, not invented from a vendor example or tuned after seeing test scores.

All NEW dataset processing, LoRA forward/backward/update logic, checkpointing, adapter loading/merge/export and test/evaluation implementation is native Rust with audited dependencies. Python/TRL/PyTorch/PEFT/C/FFI/remote training are not an implicit exemption. The official linked [TRL guide](https://docs.liquid.ai/lfm/fine-tuning/trl), inspected 2026-10-27, recommends LoRA but currently illustrates LFM2 and Python; it is reference material, not proof of an LFM3-compatible Rust trainer. Native Rust autodiff, GPU kernels/backend and whole-process memory feasibility are explicit implementation/admission dependencies; vendor single-GPU claims do not prove our trainer fits a lane.

Freeze base weights and train only the declared adapter tensors. Resolve LoRA module names/shapes against the pinned tri-hybrid LFM3 tensor map: attention q/k/v/o projections, attention gate, MLP w1/w2/w3 and short-convolution in/out projections are candidate families, not an approved blanket target list. Norms, tied embeddings/head and recurrent/cache semantics require explicit treatment. Compare small justified target/rank/alpha/precision/learning-rate/sequence-budget choices on validation data; preserve exact seeds, configuration, optimizer state, data hashes, base/tokenizer revision and trainable/frozen tensor checksums. Do not copy LFM2 defaults as accepted LFM3 hyperparameters. First prove analytic/finite-difference gradient checks, frozen-base invariance, zero-adapter equivalence and exact resume on tiny fixtures before real-model training.

Run finite training on one safely admitted host GPU; preserve live inference leases and served models. Report actual time, loss, allocation peaks and failures. Evaluate base versus adapter on identical held-out workflows, required guardrails and resource budgets; record regressions even if training loss falls. Native Rust inference must load the adapter with strict base/hash/shape/rank checks, isolate state between requests and support rollback. If merging/quantizing for deployment, re-evaluate the exact resulting artifact. Host improvement, Redox compatibility and phone latency/RAM/thermal/recovery remain separate receipts. Promotion requires measured benefit and predefined regression bounds; physical deployment still needs explicit resumption. No trained adapter, dataset or generation proof exists yet.

Rust and Cargo for replacement software. Audit and pin each existing toolchain before choosing a common workspace toolchain; kernel currently pins nightly-2026-05-24. Prefer safe Rust; every unsafe/FFI boundary needs a documented lifetime, aliasing, ordering and platform contract. Do not assume crate language implies native-free transitive dependencies. Dependencies must be available within authorized allowance/storage policies; offline first.

## Commands

Current independent first slice:

- build: `cargo build --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/tools/rust-migration/Cargo.toml`
- test: `cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/tools/rust-migration/Cargo.toml`
- legacy oracle, test-only: `python3 -m unittest discover -s scripts/oracle/trace/tests`

Future phase-specific Rust test targets are named in PLAN and TESTS. Until implemented, they must fail or be reported NOT IMPLEMENTED; do not substitute documentation greps, empty test filters, or exit-zero stubs. Existing lifecycle Python is transition-only orchestration and is itself included in the tooling inventory where project-owned.

## Project Structure

Canonical docs remain docs/VISION.md, PLAN.md, HANDOFF.md, SPEC.md and TESTS.md. Dependency/ownership ledger: docs/workbench/RUST-MIGRATION.md. New host Rust crate: tools/rust-migration/. Proposed Rust boot/emulator modules remain in this existing repository until their ownership/dependency architecture is reviewed; no new Git repository or destructive fork edits. Old plan and handoff are retrievable at main commit1263b74.

## Testing Strategy

Characterize each old implementation first. Differential fixtures compare outputs, exit status and side effects, including malformed/truncated input and missing resources. Prove a regression before each bug fix. Mutation-verify every acceptance gate: deliberately wrong mapping, injected C/Python delegation, missing feature, corrupted image, or false receipt must make the gate fail. No empty filtered test suite counts as parity. Keep source checks, host tests, Rust-emulator execution and physical-phone proof distinct.

Define explicit compatibility ledgers and unsupported behavior; a Rust stub does not inherit an old driver's acceptance. CPU/MMU tests precede guest boot. Register/IRQ/reset traces precede device parity. Image format/bounds tests precede boot handoff. Clean-environment rebuild and native-dependency audits precede final switch-over. Old executables remain comparison oracles until parity; production use is retired in a separate reviewed change.

## Boundaries

Settled-intent boundaries (2026-11-03, INTENT.md) govern product behavior; migration boundaries below govern the historical rewrite where still applicable.

- Always (product): obey voice/text direction; reuse deterministic blocks before generating; keep waveform listening-state visible while listening; log every tool call, service state, power switch, sync, and learned change in console; sync patterns/blocks/crons/settings/logs to homelab; announce cloud hand-offs before sending; recommend power moves only from measured drain history.
- Always (engineering): bounded Rust services behind the capability interface (Track A portable to Redox); evaluation-only handling of new LFM candidates against the pinned baseline; read-only probing of fleet lanes.
- Ask first: every prediction before acting (e.g. car → destination); every commit-class action — send, call, spend, share location, delete — including when requested inside a generated block; any cloud hand-off the user has not pre-approved; any auto power-off beyond pre-approved moves.
- Never: change models served on dual Spark, 5090, or 3090 (consume via gateway only); silently re-pin the on-phone brain or fall back to an unpinned model; listen while mic is user-off; auto-commit send/call/spend/share/delete; evolve or learn away the safety line; claim fleet-lane or on-device performance without measured evidence.
- User right, always: turn mic and waveform fully off/on at any time; inspect, correct, or forget any learned pattern; inspect cron list; forbid cloud per task.
- Ally lane: Asus ROG Ally models may be changed freely for fast-inference evaluation serving the phone.
- Historical migration boundaries retained: no phone flash/partition/slot changes outside the sanctioned `fastboot boot` path; keep postmarketOS bootable; no EDL/firehose tools.

- Always: bounded device-free Rust migration, isolated owned work, source inventory and fixture tests under the current approved slice.
- Ask only for genuinely missing scope decisions: firmware/assembly/toolchain boundary. Full upstream breadth and sole initial brain model are already settled; neither blocks independent device-free work.
- Never: phone commands, boot/recovery/flash/slot/partition changes, old feature work, protected VM replay, served-model changes, purchases or quota overage during this rewrite. Shared DSH maintenance may use the operator's explicit snapshot/restart/resume authority; never abandon interrupted sessions.
- Physical validation occurs only after rewrite gates and a separate explicit resumption decision; old hardware authorization is not automatic permission to resume.

## Success Criteria

- [ ] SC1: complete owned-source, dependency, feature and exception inventory with scope decision recorded.
- [ ] SC2: project host tools/tests/generators run as Rust with differential parity and no legacy runtime delegation.
- [ ] SC3: Rust boot stage replaces the required U-Boot behavior with format, memory and handoff parity; no linked C boot implementation.
- [ ] SC4: Rust-native CPU/MMU/exception/timer/GIC foundation passes architectural and negative tests without QEMU/libqemu/TCG.
- [ ] SC5: Rust emulator board/device/host interfaces satisfy the approved complete compatibility ledger and guest workloads.
- [ ] SC6: clean reproducible Rust build, dependency audit and call-site retirement pass before delivery work can resume.
- [ ] SC7: the sole initial LFM3-1.7B brain runs real, source-pinned inference in a custom native Rust service, with numerical/tokenizer/tool-format parity, bounded memory/streaming/cancellation, measured host evidence and later separately admitted OnePlus/Redox evidence. Contract fixtures alone do not satisfy this.
- [ ] SC8: Waveform+console shell is the only idle UI; waveform truthfully shows listening state (including asleep) and mic+waveform turn fully off/on on user command with no listening while off.
- [ ] SC9: Anticipation asks first and commit-class actions (send/call/spend/share-location/delete) always confirm, including inside generated blocks; generated blocks run sandboxed, console-logged, and promote to the deterministic library only when kept.
- [ ] SC10: Patterns, blocks, crons, settings, and logs sync to the homelab, survive phone reset, and every learned change is versioned, console-logged, and reversible (forget pattern).
- [ ] SC11: Power recommendations cite measured per-component drain history and estimated time-to-next-charge (measure-and-report until a baseline exists); auto-switches occur only within user pre-approval and are announced with reasons.
- [ ] SC12: On-device LFM evaluation benchmarks newer LFM candidates against the pinned LFM3-1.7B baseline on phone hardware (speed, RAM, thermal, drain, held-out task quality) and on the Ally lane; no re-pin without a new operator decision; Spark/5090/3090 models verified unchanged by read-only probe; Roomba transplant harness/results interoperate without sharing pin decisions.

## Open Questions

RESOLVED: full upstream U-Boot and QEMU features/platforms, plus all custom features, with Rust-only new project implementation/tests. Firmware/assembly/toolchain/shared-service boundaries and exact upstream coverage revisions require explicit recording in the phase1 contract; no retained native component is silently declared rewritten. Unsupported Rust targets remain visible blockers, never omitted platforms. This is not a frozen architecture or a claim that boot/emulator replacements exist.

## As-Built Addendum

Planning inventories completed; first trace-converter slice dispatched. No Rust-only stack or rewrite completion claimed.
