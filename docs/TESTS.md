# TESTS — Rust rewrite acceptance map

## P4 feature-on false-positive gate rejected — 2026-11-01 9:46 AM PT

Saved reboot/TTL logs hash-match receipts and prove0005 env forwarding/reset
non-regression. They do NOT prove the PON reset hook: no success/refusal message,
guarded strings absent from kernel, build log/dry-run report Make no-op despite
changed feature env. Kernel predates P4 build; unconditional PON init string
is not a feature-on assertion. P4-FEATURE-01 and exact artifact identities in
HANDOFF. Require fresh feature-keyed build + guarded-hook evidence; no VM rerun
or production edit by monitor. Prior13/26 component results do not close this gap.

## 0047 watchdog region guard — 2026-11-01 8:50 AM PT

Exact0043+WDT-only0044/45/46/47 artifact stack:13PASS/zero ignored. Removing
actual alignment guard makes its refusal control RED; removing overflow guard
makes wrap refusal RED; each exit101. Exact restoration cmp matches and13PASS.
WDT-BOUNDS-01 host source guard closed; FDT/MMIO mapping/silicon/resource proof
stays separate. Artifact hashes/commands in HANDOFF. No full migration gate tick.

## Malformed DTB early-rejection resources — 2026-11-01 3:20 AM PT (PST)

300 early rejections over truncated/bad-magic/oversized-declaration host fixtures:
zero allocation requests/extra requested live peak; baseline=end469560B after
every rejection. Injecting an8-byte temporary allocation/drop fails the asserted
zero-request gate (300vs0); exact restoration passes/cmp matches. This calibrates
the resource assertion only, not library mutation or all malformed inputs. Rust
artifacts/commands in HANDOFF; no physical/kernel/model/leak-freedom acceptance.

## Repeated saved-DTB requested allocations — 2026-11-01 2:25 AM PT (PST)

300post-warmup host walks (3saved DTBs x100) return requested live allocations
to469560B after EVERY walk. Peak extra94071B, largest request24576B; no net
requested live growth. Retaining subtree Vec in artifact calibration fails the
assertion (+1248B first repeated walk); exact restoration reproduces counters.
Actual Rust instrumentation/source identities in HANDOFF. No RSS/arena/phone-fit
or general leak-freedom claim; no new source gate or WDT-BOUNDS-01 acceptance.

## Pinned watchdog DTB inputs — 2026-11-01 1:30 AM PT (PST)

Rust audit of all3saved root-p3fix3 DTBs exits0: unique watchdog node,
reg=[0,0x17980000,0,0x1000], KPSS fallback, enabled, aligned/nonwrapping.
Actual blob hashes match each RESULT identity (HANDOFF). This validates these
host artifacts only; no live device, MMIO, memory-fit or new mutation acceptance.
WDT-BOUNDS-01 remains OPEN pending0047 and malformed-region refusal controls.

## Watchdog host control and saved gate audit — 2026-11-01 1:34 AM PT

0043 pure layer9PASS; actual disarm-readback guard mutation makes its existing
failure control RED, exact restoration GREEN/cmp equal. Saved root-p3fix3 serial
hashes match all3RESULT receipts; expected handoff/reset markers present. No
VM rerun/device proof: WDT_EN=0 in emulator, MemoryPeak unset. WDT-BOUNDS-01
source gap remains: translated-base alignment/overflow unchecked before volatile
access, FDT/MMIO glue excluded by host tests. Full details/artifacts in HANDOFF.

## P4 corrected host layer — 2026-11-01 12:39 AM PT

Committed0041 + PON-only0044 applied to exact0040 extraction:26PASS/zero ignored.
Ignoring actual target validation makes two root_invalid controls FAIL; ignoring
actual channel-extent validation makes two bounded_refuses_small controls FAIL.
Both runs exit101; exact restored source cmp matches and26PASS. Artifacts and
source identity in HANDOFF (p4-correction-review). Closes the host target/span
findings only. FDT/MMIO glue is excluded from these tests; kernel build and
device/runtime/resource proof remain separate. No migration checkbox advanced.

## P4 stage-specific faults — 2026-10-31 11:44 PM PT

Two independent host controls inject denial at RW status or readback status,
after the initial observer read succeeds:2PASS, no retry; write denial stops
before readback. Actual scratch write-status error-discard mutation makes the
write-denial control FAIL (Ok(4), exit101); exact restored source cmp matches
and2PASS. Artifacts p4-root-stage*; full identity/commands in HANDOFF. These
extend fault coverage without closing malformed-address/MMIO bounds findings
or proving physical effects, resource fit or leak freedom.

## P4 frozen-clock poll bound — 2026-10-31 10:49 PM PT

Unchanged0040 extracted host source + independent frozen-clock/stuck-bus test:
PASS in0.01s, requires timeout without RW writes. Removing actual poll-count
guard makes that test exceed2s (owned timeout exit124); exact source restoration
cmp matches and PASS in0.01s. Artifacts p4-root-poll* under canonical monitor
artifact directory, exact hash/commands in HANDOFF. Host termination bound only;
no1ms physical deadline, MMIO-fault or memory/leak proof. Address/range HOLD stays.

## P4 malformed-address controls — 2026-10-31 9:54 PM PT

Patch0040/a62540c exact extracted qcom_pon module:14 original host tests PASS.
Two independent controls FAIL: sid16 and register0x10088f must refuse before bus
writes, but return Ok(4) with the canonical sid0/register0x088f fake-bus trace.
Artifact p4-root-controls-tests root_ --nocapture exits101; extraction/fixtures
are Rust only. Full source and SHA identities in HANDOFF. FDT range/length loss
and unchecked volatile offset boundaries are separate unresolved source findings;
cfg(test) excludes that glue. No kernel build, device/MMIO or memory-fit proof.

## Tokenizer HOLD revalidated — 2026-10-31 8:59 PM PT

At main8e6cab6, offline tokenizer_loader18 PASS/zero ignored. Unchanged
candidate6cc561f loader matches root scratch by SHA256; four root tests rerun
with separate candidate target: print-only allocation measurement PASS, both
tag-spoof controls FAIL and asserted allocation budget FAIL (4194304>500639 on
fixed500638-byte malformed-flag input). Exit101, no source integrated. Main's
same-buffer hashing and raw duplicate checks remain accepted partial behavior;
main typed errors still collapse to MalformedJson. No generation, leak, repeated
resource-workload or device proof. Commands and live ownership in HANDOFF.

## Host troubleshooting fixture integrated — 2026-10-29 3:14 PM PT

Main source `5b7befc` fixes demo executor borrow lifetimes and asserts successful
Stage2 policy closure. Offline named act_slice10 + act_root_recovery3 +
session_contract36 + workflow_policy_step1: 50 PASS, zero ignored. Two main demo
runs exit0 and stdout cmp matches; SHA256
`24082db6b8e38b225cc316af212fb74b404bf80850a8ed55e80801bb079faecf`.
Exact shipped example in root scratch rejects actual act.rs Success closure
true->false (exit101); exact library restoration passes and stdout matches main.
Supersedes the missing-assertion rejection below for this example only. Fixtures
exercise pinned image decode, scripted success and ACK-only UnknownOutcome without
replay. No real transport/auth/device-effect or memory/leak acceptance; full0.2.4
remains OPEN. Late worker delivery and terminal receipt are recorded in HANDOFF.

## Workflow salvage demo missing assertion — 2026-10-29 2:15 PM PT

Unmerged6d657fa still fails compilation (known2xE0502). Root scratch fixes only
executor borrow lifetimes, then proves a demo-specific gap: changing actual
act.rs Success closure true->false leaves Stage2 Acknowledged but demo exits0.
Adding Stage2 assert policy.state()==ResultKnown makes that source mutation
RED/exit101; exact library restore GREEN, double run exit0/cmp identical.
Concrete draft/review artifacts dcwf01b-root-review and DC-WF-01BF-ROOT-REVIEW.md.
Worker/main files untouched. Not integrated acceptance; original named50
component tests + full owned diff review still required. Full0.2.4/resources/
real transport/device-effect proof remain OPEN.

## Asserted malformed-flag resource gate — 2026-10-29 1:20 PM PT

root_invalid_flag_allocation_measurement only asserts input rejection and prints
allocation size; it PASSES the held4MiB candidate, so is NOT an allocation gate.
New independent root_invalid_flag_read_buffer_request_budget asserts <=500639
bytes for the FIXED500638-byte fixture, using the measured baseline read-buffer
bound. Candidate6cc561f actually FAILS4194304>500639; baseline main source PASSES.
Separate targets, Rust fixtures/counter, no worker edits. Exact test and commands
in tok01c-root-review / tok01c-root-baseline and BR-TOK-01CE-RESOURCE-GATE.md
artifacts. This is not peak/total/leak/phone-fit proof or a universal input budget.
Next candidate must pass the asserted gate, not merely print a passing measurement.

## Rejected tokenizer candidate allocation probe — 2026-10-29 12:25 PM PT

Independent Rust root_invalid_flag_allocation_measurement: same500,638-byte
own-identity fixture with100,000-null invalid boolean value. Separate fresh builds
show largest allocation/reallocation request4,194,304bytes in unmerged6cc561f,
versus500,639bytes with main ee49053 source. Both refuse input. Generation occurs
outside per-thread System allocator counting window. These are request-size
measurements, not peak/total/live memory or acceptance thresholds; one case is
not repeated-workload/phone-fit evidence. Initial shared-target baseline was
discarded as reused candidate behavior. Exact artifacts/commands and source hashes:
BR-TOK-01C-RESOURCE-ADDENDUM.md. Existing tag-spoofing HOLD remains unchanged.

## Tokenizer typed-error candidate rejected — 2026-10-29 10:35 AM PT

Unmerged6cc561f tokenizer_loader23 tests independently PASS. Two additional
Rust controls in artifact tok01c-root-review FAIL: duplicate added-token content
`DuplicateVocabId { id: 12 }` forges a DuplicateVocabId variant; JSON root string
`"LimitExceeded"` forges a resource-limit error instead of MalformedJson.
Candidate remains HOLD; no taxonomy/encoding/generation acceptance. Correction
must preserve actual tags and payloads against arbitrary user tag text, pass23+2
controls, and fail actual unsafe-classification source mutation before restoration.
See canonical HANDOFF and BR-TOK-01C-ROOT-REJECTION.md artifact for exact commands.

## Known command failure closure — 2026-10-29 9:39 AM PT

Main3e4fb6f independently reviewed: matching ACKed Command Failure now closes
as ResultKnown without fabricating Success. Root reproduced58 named tests:
act_known_failure8, act_slice10, act_root_recovery3, session_contract36,
workflow_policy_step1. Reverting actual act.rs Failure arm to record_result(false)
in scratch makes original root probe RED (Acknowledged/ResultKnown); restoring
exact source GREEN. Wrong id/op/pre-ACK/duplicate controls preserve snapshots;
UnknownOutcome/timeout and reconstruction retain in-flight send evidence.
Host-policy defect closed only; real transport/auth/device effects, allocation/
resource accounting and full troubleshooting_workflow acceptance remain separate.
Worker delivery was post-cutoff per raw commit call and ledger; tests do not
prove a hard guard or timely execution. See canonical HANDOFF.

## UART CR shrinking-cap correction — 2026-10-29 6:54 AM PT

HW-36 integrated dada611. Root reproduced44lib+2probe=46PASS/zero ignored on
worker and main, fmt PASS and aarch64 cross-build PASS (no execution). Original
root probe unchanged. Removing the new CR-error pending_cr assignment in a
properly laid-out scratch tree makes cr_shrinking_cap_then_lf_no_phantom_line
fail with Ok(0) versus Ok(2); exact restoration GREEN. Initial missing relative
include fixtures caused compile failure and were not counted as mutation proof.
Gap-test wording now distinguishes source-side attempts from unobservable
internal charging. Allocation/repeated1000-line counters remain narrow host
evidence; no leak-free, real-MMIO, physical UART, boot or phone readiness claim.

## Host policy composition, partial0.2.4 — 2026-10-29 6:00 AM PT

DC-WF-01A integrated0cb4fd3 adds workflow_policy_step1. Root independently ran
it with act_slice10, act_root_recovery3 and session_contract36:50PASS/zero ignored.
Source mutation of session.rs stale-frame guard <= to < makes the composed test
RED; exact restoration returns GREEN. This composes stale-observation rejection,
UnknownOutcome/no blind retry, explicit resolution/retrust and result-before-ACK
ordering. Observation/ACK alone never claims completion; later result is required.
No library implementation changed. Full troubleshooting_workflow/0.2.4 stays OPEN;
real transport/auth/screenshots/persistent logs/UI effects and phone fit are unproved.

## Log-body codec/producer, partial0.2.1 — 2026-10-27 7:25 PM PT

device-control now executes52tests:20protocol_contract,4protocol_boundaries,
24log_record and4log_producer. All PASS/0ignored. Literal wire, body bounds,
UTF8/NUL/newline preservation, wrong-op refusal, borrowed views and identity
binding are covered. Root boot-binding source mutation was RED, restored GREEN.
Producer constructor now makes real records without manufacturing wire bytes;
application Trace/Debug levels are retained. No actual host log persistence,
phone logs, transport/auth/replay, screenshot/input/CLI or full0.2.1 acceptance
is inferred. Active continuation candidates need separate root review/tests.

## Priority diagnostics and CLI operation — 2026-10-27 5:46 PM PT

PLAN0.2.1 remains OPEN with partial framing implemented in crates/device-control:
protocol_contract20PASS plus protocol_boundaries4PASS,0ignored. Root independently
verified the full source and added literal wire, exact hard-cap, per-rejection
output preservation and repeated-workload cases. Actual policy-check source
mutation was RED, restored source GREEN. This does not implement body schemas,
pairing/session state, complete caller inventory or CLI semantics. Do not bank
the full item merely because its partial target is green. logging_contract,
interaction_contract and troubleshooting_workflow remain absent/NOT IMPLEMENTED.
Rust host/fake-transport evidence must remain distinct from future real Redox
phone proof. Details and measurement limits: workbench/DEVICE-CONTROL-CONTRACT.md.

Required negative controls: truncated/oversized messages and frames, invalid
stride/format/coordinates/rotation, stale/wrong-boot screenshots, malformed
commands, missing authorization, sequence gaps and log drops, disk-full or
reconnect loss, cancellation, command timeout, disconnect after possible
execution and duplicate action IDs. Prove held inputs release according to
policy; no blind replay of unknown command outcomes. Show that a received ACK
cannot pass a UI-effect assertion without a subsequent observation. Repeated
workloads require bounded queues/resource measurements and private log
redaction/retention checks, not a leak-free claim from Rust alone.

Later separately authorized physical acceptance must retrieve actual logs and
screenshots, execute a command, drive real UI actions through the shared input
broker, verify resulting screen/state and retain evidence across a failure.
Source/mock tests do not prove that connection, persistence through hard power
loss or remote power-button recovery. No photo or device command is requested.

## Typed quantized identity acceptance — 2026-10-27 5:24 PM PT

New tensor_quantized_identity target16PASS/0ignored; all required reader targets78PASS and combined brain109PASS. Private ValidatedBase requires existing actual-byte hash/structural validation and retains the immutable entry. Typed Q8 binding checks exact pins/base links/shape/recipe/length/digest before scale reading. Wrapper borrows payload, shape and manifest identity; base-proof creation still allocates bounded parsed metadata and a64-byte ID. Root independently observed14PASS/1FAIL in the cutoff packet (bad test expected length304 for299/300-byte fields), corrected exact field/length expectations without dropping assertions, removed unused newly drafted helpers/wildcard imports and added matching-shape rank/dimension/element/byte limits with a wrong payload digest to prove pre-hash order.

Root mutation disabling payload hash check made altered-payload test RED; disabling base-manifest-ID comparison made base-link test RED; restored source GREEN. Fixture is exactly34 bytes generated by retained Rust source and independently checked with unchanged coreutils comparison oracle:8694fb0e0737529d0fd84430327afc942b711d41c362b789587d6dddf0c45655. Rejected the draft1,024-byte/32block fixture and its conditional positive-test fallback. Exactly one10,000-cycle test with one prevalidated base measured3,264KiB process RSS/0.13seconds, not exact heap allocation/leak freedom/model fit. Huge hostile shapes that differ from the base test binding rejection, not arithmetic branch coverage; raw reader overflow tests remain separate. Existing unsafe/native dependency boundaries remain open.

Runner verify PASS, bank9f5bf1b. The automatic self-lane audit is explicitly degraded and saw only a truncated source snapshot; it is not full independent code-review proof. Root complete-diff/source review and separate Spark review outcomes are in HANDOFF and RUST-CONTINUE-20261028T0000-* receipts. No production manifest authentication, numerical quantization proof, pretrained generation, LoRA or physical result follows from these fixtures.

## Quantized raw-reader evidence — 2026-10-27 4:55 PM PT

Main292b2e3/c9e70ea adds tensor_quantized (7 tests) and tensor_quantized_limits (6), combined brain93PASS/0ignored. Exhaustive65,536 binary16 reference comparison uses separate Rust dyadic arithmetic and literal values; constructor rejects all2,048 NaN/Inf encodings. Covers signed quants/scales/zeros/subnormals, exact length, borrowed pointers, per-row alignment, limits, true byte overflow, version/access rejection and10,000 repeated parse/decode cycles. Root changed actual production signed-quant decode to unsigned: literal test RED128 instead of-128. Root removed half sign: exhaustive test RED at negative zero. Exact source restored, full suite GREEN; these are real source mutations, not merely changed input bytes. No unsafe/new dependency/payload allocation introduced. Compiler/std/allocator/unwind and earlier dependency boundaries remain unresolved.

RUST-CHECK-20261027T2342-* receipts under artifacts/handoff/redox-perplexity retain full diffs/raw worker results, earlier failures, root RED/GREEN and resource logs. Exactly one repeated test selected; 2,496KiB process RSS and below0.01second wall clock on the tiny34-byte fixture, not exact heap accounting, leak-freedom or full model fit. Production provenance integration target tensor_quantized_identity is still absent/RED. No0.1.2 completion, quantizer/GGUF container, weights, pretrained generation, LoRA or hardware claim.

## Current tensor identity evidence — 2026-10-27 4:01 PM PT

All-features brain suite: **80 PASS, 0 ignored** (30 module + 1 official config + 6 structural adversarial + 18 container + 22 identity/digest/resource + 3 borrowed views). The 19 worker identity tests are retained; root adds a header-limit ordering regression, the concrete SHA256 u8/U64 block size/alignment invariant and a 10,000-cycle identity/hash/parse workload. Root first observed incorrect header-limit/hash-error ordering as RED (`header_resource_policy_precedes_hashing` returned HashMismatch instead of LimitExceeded), then fixed the cheap header-size preflight and observed GREEN. Tests use pinned independent empty/abc/NIST multiblock digest vectors and a literal digest for the 160-byte synthetic tensor fixture. Requested names must match the trusted manifest entry even for identical bytes; this does not prevent a caller from replacing both entry and request, so manifest authenticity remains external. Malformed/oversized manifest strings, digest spellings, size/format/source/model mismatch and tampering fail. Correctly hashed malformed structure still fails; bounded malformed JSON with a wrong hash reports HashMismatch before JSON parsing.

Root found ad hoc inline Python fixture/hash generation in worker tool history; that recipe is not accepted as Rust migration evidence. The tracked Rust generator `crates/lfm3-brain/examples/identity_fixture.rs` now reproduces the 160-byte fixture without hashing it. Existing unchanged GNU coreutils sha256sum 9.4 independently produced `2fef6acb44773355afd01c9366ea439cb272c5b6c8235a11275b59d4a848e337`, matching the frozen literal. This external tool is a comparison oracle only; production and Rust tests use no Python/CLI hashing. Generator run and focused 22-test suite PASS. Original worker history is preserved; no inline non-Rust project test/generator logic is allowed in future packets.

`RUST-CHECK-20261027T2247-*` retains raw tool histories, complete source/diffs, dependency metadata/lock graph, source audit plus root corrections, RED/GREEN logs and x86_64/aarch64 release object observations. The SHA software path still contains unsafe GenericArray/block-buffer casts; the concrete host layout assertion is not a full soundness proof. force-soft does not remove cpufeatures/libc from the ARM build graph. getauxval was absent from inspected brain/SHA objects but present in ARM cpufeatures; whole-program native retirement remains OPEN. The actual repeated test selected one test and measured 3264 KiB max process RSS / 0.29 seconds; an earlier command selected zero tests and its rejected measurement is retained, not counted. No exact heap accounting, leak-freedom or model-fit claim.

`tensor_quantized` remains a missing target (NOT IMPLEMENTED/RED), and 0.1.2 remains OPEN. Manifest authenticity/signatures, trusted production manifest acquisition, secure/streamed file opening, real pretrained execution, LoRA and physical proof remain separate unsatisfied contracts. Do not bank from these fixture tests.

Latest operator clarification: no physical test admission during the Rust-first migration. Memory safety is not leak-freedom: relevant replacement gates require negative/malformed-input tests, review of unsafe boundaries and allocation/resource accounting with repeated-workload memory measurements. Missing measurements remain unverified.

No architecture/final parity gate is currently accepted. Future test targets below are requirements, not implemented tests. Missing targets must be reported RED/NOT IMPLEMENTED. A successful baseline crate suite does not pass these gates.

Scope is confirmed FULL UPSTREAM U-Boot/QEMU plus every custom feature. In every row below, coverage means all upstream architectures/platforms/features, not a OnePlus subset. U-Boot completion covers boards, drivers, commands, services and host tooling; emulator completion covers system/user targets, execution/accelerator contracts, platforms/devices, host backends, APIs and tools. Every missing/unsupported matrix row blocks final parity. New project test implementations are Rust; frozen non-Rust oracles are temporary comparison dependencies only.

| id | criterion | kind | command | expect | result |
|---|---|---|---|---|---|
| SC1 P1 | Complete inventory, pinned compatibility baseline and resolved scope/boundaries | cmd | `cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/tools/rust-migration/Cargo.toml --test phase1_gate` | All contract/omission controls pass | NOT IMPLEMENTED |
| SC2 P2 | Every owned host tool/test/generator is Rust and callers have parity without legacy runtime delegation | cmd | `cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/tools/rust-migration/Cargo.toml --test phase2_gate` | Inventory exhausted; all negative parity controls pass | NOT IMPLEMENTED |
| SC3 P3 | Rust boot stage replaces approved U-Boot interfaces | cmd | `cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/tools/rust-migration/Cargo.toml --test phase3_gate` | Parser/package bounds, static no-C linkage, Rust-stage guest execution and fake recovery transport contracts pass separately | NOT IMPLEMENTED |
| SC4 P4 | Rust CPU/MMU/exception/timer/GIC engine without QEMU/TCG | cmd | `cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/tools/rust-migration/Cargo.toml --test phase4_gate` | Architectural fixtures and injected fault controls pass; no C engine fallback | NOT IMPLEMENTED |
| SC5 P5 | Complete approved emulator/board/device/host/guest feature matrix | cmd | `cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/tools/rust-migration/Cargo.toml --test phase5_gate` | Every frozen matrix row has independent observable proof; stub/unsupported entries fail | NOT IMPLEMENTED |
| SC6 P6 | Clean reproducible build, audited dependencies, retired production legacy paths | cmd | `cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/tools/rust-migration/Cargo.toml --test phase6_gate` | Pinned clean environment reproduces named artifacts; production reachability and rollback mutation controls pass | NOT IMPLEMENTED |
| SC7 P7 | Sole initial LFM3-1.7B brain served by native Rust, complete runtime and target evidence | cmd | `cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/crates/lfm3-brain/Cargo.toml --test brain_delivery_gate` | Real pinned model output, native dependency audit, bounds/cancel/tool parity and separately admitted target receipts; no alternate model or remote/C engine fallback | NOT IMPLEMENTED |
| SC8 P9 | Waveform+console only idle UI; listening-state truthful incl. asleep; mic+waveform user off/on with no listening while off | cmd | `cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/apps/kimono-ui/Cargo.toml` | Off state blocks capture; on state resumes; waveform matches capture state in tests | NOT IMPLEMENTED |
| SC9 P9 | Ask-first anticipation; ask-to-commit for send/call/spend/share/delete incl. inside generated blocks | cmd | `cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/apps/agentd/Cargo.toml` | Commit attempts without confirmation are refused; predictions never auto-act | NOT IMPLEMENTED |
| SC10 P10 | Homelab sync of patterns/blocks/crons/settings/logs; survives reset; learned changes versioned, logged, reversible | cmd | `cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/tools/user-sim/Cargo.toml` | Sync round-trip + forget-pattern reversal pass; no silent behavior change | NOT IMPLEMENTED |
| SC11 P12 | Power recommendations cite measured drain history + time-to-next-charge; auto-switch only pre-approved, announced | cmd | `cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/apps/agentd/Cargo.toml --test drain_analytics` | Recommendation without baseline refused; estimates cite sample window | NOT IMPLEMENTED |
| SC12 P13 | LFM eval: candidates vs pinned LFM3-1.7B on phone+Ally (speed/RAM/thermal/drain/quality); no auto re-pin; fleet Spark/5090/3090 unchanged | cmd | `cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/crates/lfm3-brain/Cargo.toml --test ondevice_bench` | Measure-and-report comparison; read-only fleet probe identical before/after | NOT IMPLEMENTED |

Phase0 brain foundations, tiny numerical fixtures, fake backends and service contracts cannot pass SC7. Wrong model/revision, corrupt tensor, cache-state leakage, unsupported ISA, over-budget context, cancelled generation, malformed tool calls and unavailable backend must fail visibly. Actual pretrained host inference must precede any model-serving readiness claim. Hardware remains STOPPED.

Current partial brain evidence: `cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/crates/lfm3-brain/Cargo.toml --features service` passes six Rust service-boundary tests (stream ordering, model/request bounds, unavailable backend, cancellation/deadline/sink failure, input-plus-output token reservation and mid-stream cancellation). The backend is a cfg(test) fixture, never pretrained inference. The pinned model config validator is now accepted as a narrow subpacket: nine model tests plus six service tests and one public-API check against the independently retained official config (16PASS with --features model). It rejects missing baseline fields, changed values/layer order/RoPE and wrong identity. The numerical kernels module remains absent, so the0.1.1 all-features gate stays RED; no serving-ready or phone-fit claim. Crate-wide cargo fmt also cannot resolve that missing module; direct rustfmt checks on the new files pass.

## SC8 — actual-use LoRA specialization

NOT IMPLEMENTED. PLAN phase8 is mandatory: supervised LoRA on the same pinned LFM3-1.7B, followed by base-versus-adapter evaluation on untouched actual-use workflows. Required Rust targets are specialization_data, specialization_eval, lora_training, lora_host_training, lora_artifacts and specialization_gate. They do not exist yet; missing targets are RED, not skipped acceptance. Data/schema/evaluation preparation may overlap independent ready work, but actual training requires native Rust model/autodiff/backend proof, curated data, frozen scoring thresholds and fresh GPU admission. No actual-use training corpus or adapter has been produced.

Negative controls must detect session/workflow leakage, secrets or missing provenance, fabricated success labels, missing assistant loss masks, frozen-base mutations, wrong module/shape/rank, mismatched checkpoint/tokenizer/base hashes, invalid gradients and incomplete resume. Zero adapter must reproduce the unchanged base; nonzero updates must be validated by independent gradients and actual model loss/output, not just an artifact file. Training loss alone cannot pass the gate. Held-out tool success, formatting, clarification, broker refusal and retention/resource regressions are measured against the same base under the predeclared protocol. Re-test any merge/quantization and require rollback proof. Host fine-tuning does not prove phone improvement or authorize physical deployment.

## First slice evidence

`cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/tools/rust-migration/Cargo.toml` runs the candidate Rust trace converter's current tests. The Python subprocess exists only in differential tests. Root reproduced the negative-value mismatch RED, then implemented native modular ASCII VALUE parsing; the regression is now an ordinary nonignored passing test. Five differential tests cover the existing cases plus 80 signed/long/malformed value-width combinations and output/input preservation. A further positive-ASCII base-0 parser subpacket now has a Rust differential numeric_contract test:13 literal cases plus the original underscored-size case, with output preservation on invalid literals. Full current trace suite17PASS/0ignored. This does not close arbitrary-precision/address/signed CLI/whitespace, Unicode/digit-limit, regex or diagnostics gaps.

`cargo test --offline --manifest-path $HOME/Developer/experiments/redox-oneplus6/tools/rust-migration/Cargo.toml --test python_parity -- --include-ignored` passes the implemented coverage (5 PASS, 0 ignored), not every required compatibility case. Item2.1.1 remains OPEN until the remaining contract gaps have explicit regression coverage and are resolved. Do not switch callers or delete the Python oracle merely because this partial suite is green.

## Mutation requirements

Phase1: omit a tracked script, embedded C fragment, native dependency or unresolved scope entry. Phase2: reintroduce old-language runtime delegation, negative integer parsing difference or a missing caller. Phase3: inject C U-Boot linkage, truncate payload, cross reserved memory or omit ABI/required service. Phase4: corrupt an instruction/MMU permission/IRQ result or load libqemu. Phase5: substitute a stub, suppress an interrupt/input event or drop a required platform row. Phase6: change a pinned input, silently invoke Python/C fallback, lose a required artifact, or break rollback. Each must be observed RED before its gate is accepted.

## Clean-build and retirement contract to freeze in phase1

Name container/OS/toolchain/source/lockfile/input hashes; allowed external dependencies and networking policy; Android payload, Rust boot stage, kernel/initfs, emulator and tool binary artifacts; deterministic comparison rules and provenance exceptions. Production runtime graphs and test-oracle graphs are separate. The final Rust runtime cannot delegate to old product executables. Keep private Git baseline1263b74, fixtures and old-language oracle source available for reviewed source rollback, without invoking old phone actions automatically. Retirement is a targeted source/caller change, never destructive history removal.

## Unverifiable criteria

- Waveform visible/truthful while phone is asleep on real hardware — manual device check (screenshot/photo while asleep); host tests prove state logic only. (SC8 P9)
- Spoken recommendation reads sensibly to the user (wording, brevity) — manual listening check; automated tests verify cited numbers and gating, not phrasing quality. (SC11 P12)
- (Historical) software contracts once implemented are verifiable above; physical phone verification beyond these two remains gated by Track B resumption, not inferred from host commands.

## Numerical foundation evidence — 2026-10-27 2:09 PM PT

The all-features brain suite now executes30 module tests plus1 independent official-config API fixture (31PASS/0ignored). Fifteen numerical tests cover analytic outputs, dimension/nonfinite/bounds refusal, no-state-change on rejected conv input, overflow and reset. Root source mutations separately prove the D8 partial-RoPE test rejects a doubled exponent and the SiLU subnormal test rejects early f32 rounding; restored source passes. Approximation helper now rejects either nonfinite side and subnormal comparison uses a one-ULP bound. Rust math dependency audit and release-object import receipt are RUST-CHECK-20261027T2057-* under artifacts. Full pretrained inference, memory/latency/leak measurements, complete native-runtime retirement and phase0.GATE remain NOT IMPLEMENTED. These tests are not physical proof.

## Tensor structure evidence — 2026-10-27 3:08 PM PT

Combined brain suite58PASS/0ignored:30 module,1 official-config,18 tensor_artifacts,3 tensor_views and6 tensor_adversarial. The21 original parser cases remained after splitting valid-view tests from malformed-input tests. Root reproduced RED then GREEN for duplicate names containing serde diagnostic text and empty shapes whose nonzero prefix overflows. Additional tests exercise all proper truncations, huge lengths, recursion bounds, decoded escaped duplicates, every header position/byte value and10,000 repeated parses. Header/total byte caps precede JSON parsing; rank/count/dimension checks follow bounded metadata parsing; no tensor bytes are allocated/copied. This is not exact peak heap accounting.

`RUST-CHECK-20261027T2152-*` receipts preserve worker raw tool results, full source, RED/GREEN logs, source hashes, wasm32 compile-only check and `/usr/bin/time -v` measurement:3072KiB max process RSS/0.09seconds for the tiny repeated-workload test alone. No unsafe introduced; compiler/std/allocator/unwind/native host boundaries remain OPEN. Hash/provenance and quantized-format tests `tensor_identity`/`tensor_quantized` do not exist and must stay NOT IMPLEMENTED/RED. Do not bank0.1.2 from the passing structural tests. No pretrained generation, LoRA, phone fit or physical deployment follows.
