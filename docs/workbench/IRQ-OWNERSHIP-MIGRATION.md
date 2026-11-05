# Proposed IRQ ownership migration — reviewed interfaces

2026-10-26 PT. Planning/review only against kernel09d0774e. No implementation,
build, VM, device, service, inference, AP or SGI admission is granted here.
This replaces neither the canonical handoff nor GICV3-AP-CONTRACT.md.

## Decision and minimum coherent boundary

Use one guarded controller core plus owned dispatch continuations. Separate
standalone callbacks from controllers. Standalone callbacks use safe shared
receivers; controllers retain mutable receivers exclusively behind the guard.
A handler-slot mutex and a partial core `&self` conversion are unnecessary.
The minimum production integration is the complete access-family migration below;
no intermediate mixed raw-global/guarded graph is a valid shipped checkpoint.
Preparatory tests/type extraction can be separate commits with old behavior intact.
This is a feasible interface proposal, not a soundness or SMP completion claim.

## Interfaces and reference lifetimes
Proposed interfaces (names illustrative; semantics mandatory):

```rust
trait IrqHandler: Send + Sync {
    fn handle(&self, virq: u32, token: &mut CleanLockToken);
    fn eoi_after(&self) -> bool { false }
}
trait InterruptController: Send { // no InterruptHandler supertrait
    // Existing mutable MMIO/configuration methods remain guarded.
    fn begin_dispatch(&mut self, input: u32) -> ControllerStep;
    fn resume_dispatch(&mut self, cursor: Cursor) -> ControllerStep;
}
// Owned values only: IDs, copied routing, Arc, cursor; never controller/desc refs.
enum Action { Handler(Arc<dyn IrqHandler>, Delivery), Scheme(u16),
              Dispatch(Delivery), RegisteredOnly(Delivery), Resume(Cursor) }
// Private wrapper owns Mutex<CoreState>; no public Deref or mutable accessor.
struct IrqRegistry { /* IRQ-save spinlock and initialization state */ }
```
`Delivery` includes virq, copied controller/hwirq completion target, dispatch mode,
and a routing-generation/in-flight lease. `Cursor` includes controller ID, input,
next TLMM pin or BCM/PLIC continuation, and copied completion obligation.
They are synchronous stack-owned, non-Send values. No continuation carries an
IRQ-chip pointer, `&Core`, controller reference, guard, or borrowed descriptor.
No allocation is needed for dispatch: Arc clone, fixed small Action values and
bounded-by-topology call stack suffice; validate acyclic controller topology.
Private `with_core` uses an HRTB closure with owned return values; guard lifetime
cannot enter the return type. Acquire IRQ masking and lock before constructing
ANY core reference, including shared lookup/read/iteration and error logging.
Drop every core/controller/descriptor reference and guard before invoking an
Action. Resume reacquires and freshly borrows. Controller methods cannot call
registry APIs, callbacks, scheduler, event queues, allocation or blocking code.
Fallible registration/allocation happens before the lock; Arc destruction occurs
after unlock. Diagnostic formatting copies values and logs after unlocking.
Standalone audited production implementations are AArch64/RISC-V Com1Irq (unit
struct) and RISC-V ClintConnector (two usize fields). They need no mutable self;
change to `&self` is complete for THESE callbacks only. Their device state stays
behind COM1/CLINT locks. Compiler-enforced Send+Sync replaces unsafe handler
Send/Sync assertions. Test Record already uses an atomic. Controller stubs cease
implementing the callback trait; they remain guarded mutable controllers.

## Delivery, completion, and same-virq policy

1. Enter with the existing clean token. Under core guard, validate virq, pin route,
   choose registered handler before child before scheme, clone handler Arc or
   execute one controller MMIO step. Release guard and temporary IRQ mask.
2. Invoke chosen action with the original clean token; no handler-slot lock exists.
   Shared handler invocations may run concurrently or recursively, including the
   same virq. Each invocation owns its own completion obligation. Nothing is
   classified as temporarily unregistered; no Busy fallback, take-out, queue,
   silent coalescing, or dropped delivery is introduced.
3. After callback return, capture opt-in EOI result and freshly guard core to
   execute the copied completion target exactly once, then unpin. Scheme paths
   keep their existing userspace-ack EOI responsibility; do not auto-EOI them.
4. Existing callbacks must not reenter dispatch while holding their backend lock.
   COM1's lock statement ends before irq_trigger; ClintConnector releases CLINT
   before timeout/tick. Event/receive internal callees require the audit below.
   Safe `&self` permits core reentry, but does not magically make backend lock
   recursion safe. Future stateful handlers must use reviewed interior state,
   release its lock before reentry, and satisfy Send+Sync without blanket unsafe.
Controller sequences, executed on the initiating CPU, preserve actual source:

- BCM2835: claim under guard; valid virq<1024 selects RegisteredOnly, which calls
  only a registered callback (no child/scheme fallback). Preserve old direct-call
  semantics: it does NOT consult callback eoi_after. After callback/no-handler or
  invalid-source diagnostic, Resume performs BCM self irq_eoi(claim) exactly once.
- TLMM: probe_only exits. For each enabled pin in ascending order, freshly guard,
  read status and clear it by writing1 BEFORE yielding Scheme(virq_base+pin).
  Invoke scheme event outside guard, then resume at next pin. Do not snapshot all
  pins/clear them all early, dispatch registered per-pin handlers, or reverse clear
  and event. Preserve irq_handler_eois_root and outer root-EOI decision unchanged.
- PLIC: claim under guard, translate, then normal nested Dispatch(virq). Success
  adds NO unconditional PLIC EOI after nested return. Failed translation logs and
  completes the claimed hardware IRQ once. Preserve current zero-claim behavior
  unless separately reviewed; do not invent successful zero-claim processing.
- HLIC interrupt: acknowledge sip as before; copied hart->controller mapping,
  translate under guard; registered-handler/child selection preserves its panic
  on an unconnected source. Hlic controller callback forwards virq_base+input as
  a normal Dispatch. CPU_INTERRUPT_HANDLERS becomes builder-owned then published
  immutable lookup, not a second mutable global graph.
- AArch64 COM1: receive, scheme event, return, EOI. RISC-V COM1 currently performs
  explicit global EOI inside its callback; migrate it to identical after-return
  completion (not blanket default-true). ClintConnector keeps default false.
- GICv2 raw-IAR fast acknowledge/end and GICv2 AP/local gates stay first and keep
  raw source-CPU bits. Fallback root methods guard core before borrowing controller.
  Preserve AArch64 handler root-EOI branching, spurious handling and timer EOI;
  do not add generic automatic root completion to every Action.
No Action queue crosses ERET. Completion-bearing paths must not migrate CPU or
schedule between banked acknowledge and completion; audit actual scheduler paths
before admission. Clint timeout/tick has no newly held core/backend guard. If a
completion path can switch CPUs, a reviewed CPU-pinning primitive is required;
non-Send alone does not prove scheduler affinity. Userspace scheme ack remains
its existing separate operation, not a stack-held interrupt completion token.

## Configuration, publication, locking, MMIO
Descriptor in-flight state pins route across dispatch. Registration is atomic:
existing handler=>EEXIST; previously empty but dispatch in flight=>EBUSY; otherwise
install Arc. Configure/translate/check-reserved/route/enable/reserve is ONE guarded
transaction; pure translation and read-only snapshots may be separate. O_STAT
reads without mutation. Iterators/lists return owned copies, never guard-borrowed
iterators. Release during in-flight marks Retiring, prevents new reservations,
and defers disable/route-clear until last lease ends; finalization happens under
guard. Late scheme acknowledgments retain existing mapped-IRQ behavior; do not
claim this redesign establishes a new userspace hardware-cookie protocol.
Build graph in local IrqBuilder with owned controllers/descriptors while CPUs and
interrupt delivery are still gated. Replace init signatures with explicit copied
InitInfo (own phandle, parents, resolved parent translation, hart/context). Parent
translation is performed before borrowing the child mutably. BCM parse receives
its parent virq; PLIC gets own DT identity and HLIC/context resolution; HLIC gets
own phandle/hart and builder mapping output. No init method looks up global core.
Replace ACPI setup and all timer/serial initialization direct mutations too.
Publish completed builder as Ready under guard once; before Ready runtime APIs
return ENODEV, not lazy initialization. Do not enable a source before its handler
and root selection are published; preserve current GICv3 AP and SGI gates.
Lock order: caller holds no backend/event/ordered token lock when entering core;
local IRQ-save -> core spinlock -> bounded MMIO only -> unlock -> restore exactly
saved IRQ state -> callbacks/backend/event locks with original clean token.
Preserve scheme kwrite's HANDLES read guard across ack-store and guarded EOI;
otherwise close can remove/release the reservation between them. The only proposed
nested edge is HANDLES L1 -> IRQ-save/core spin (rank strictly after L1), never
core -> HANDLES. close drops its write guard before set_reserved as today. Full
lock-edge/rank review remains required. Do not invent unlocked handle snapshots
without a separately reviewed lifetime protocol.
No fabricated CleanLockToken. Add debug core-held assertions in action runner.
AArch64 save/restore covers I mask only, preserving other DAIF fields; nested
IRQ-save restores masked state. RISC-V saves/restores SIE equivalently. AArch64
exception entry masks IRQs, SPSR preserves PSTATE/DAIF, ERET restores it. This
proposal makes no new current single-CPU race claim and adds no unmask policy.
Audit each controller's Send requirement. Integer MMIO bases need no unsafe Send;
PLIC's raw pointer needs a narrow owned MMIO-address wrapper or justified Send
impl for Plic only: stable mapped range, no escaping MMIO Rust refs, guarded all
access, no destruction/unmap, volatile operations, correct context/CPU use.
GICv2 bypass and any other direct MMIO path must not concurrently create Rust
register &mut aliases with guarded methods: retain scalar volatile address access
or prove disjoint banked ranges. A core mutex alone cannot justify overlapping
MMIO references. No blanket Sync impl, per-CPU graph, or shared raw core escape.

## First author scope and executable acceptance contract
Kernel files relative to the pinned source root: src/dtb/{irqchip.rs,mod.rs};
src/scheme/irq.rs; src/acpi/madt/arch/aarch64.rs;
src/arch/aarch64/device/{mod.rs,serial.rs,generic_timer.rs};
src/arch/aarch64/interrupt/irq.rs; src/arch/aarch64/device/irqchip/{mod.rs,gic.rs,
gicv3.rs,irq_bcm2835.rs,irq_bcm2836.rs,tlmm.rs,pdc.rs,null.rs};
src/arch/riscv64/device/serial.rs; src/arch/riscv64/device/irqchip/{mod.rs,plic.rs,
hlic.rs,clint_sbi.rs}. Add portable dtb/irq_dispatch.rs and private irq-lock wrapper
under src/sync only if needed; do not change DAIF entry policy, SGI targeting,
AP enable gates or existing GICv3 register model. Audit all architectures by
complete source search; any additional caller expands the reviewed file manifest
before edits. These are one coherent author task, not parallel half migrations.
Existing executable seam verified by reading its implementation:
`python3 $HOME/Developer/experiments/redox-oneplus6/scripts/test_irq_callback_eoi.py <kernel-root>`
extracts actual trait/trigger_virq/irq_eoi and invokes rustc --test. Its four tests
cover old callback completion only; extraction will need reviewed adaptation.
Kernel Makefile test boots via redoxer and is prohibited. Inline cfg(test) does
not establish a runnable whole-kernel host target. No new tests ran in this task.

Missing required harness: source-rooted portable production dispatch + registry
transaction module, compiled directly by rustc --test with injected lock/IRQ/MMIO
sinks. Runner must include/extract the actual guarded facade, action runner,
BCM/PLIC/TLMM steps, and registration/reserve operations, not rewrite the algorithm.
Proposed command after creating the harness is `python3 scripts/test_irq_ownership.py <kernel-root>`
(project script, not an existing command). Test callback reentry with guard-held
assertion, concurrent same-virq Arc invocations, atomic duplicate reservation,
retiring route, source-specific action traces, default-false and COM1 completion,
root IAR preservation, Ready gating, and IRQ-mask restoration including nested
masked entry. Negative controls: invoke callback inside guard (must fail without
hanging), omit each completion, replace BCM RegisteredOnly with normal fallback,
reverse TLMM clear/event; each mutation must fail a named production-path test.
Compile-fail checks forbid borrowed Action and !Send controller/!Sync handler.
Offline A64 check/link uses existing nightly-2026-05-24, --offline --locked,
--bin kernel --target targets/aarch64-unknown-kernel.json, -Z build-std=core,alloc
-Z build-std-features=compiler-builtins-mem -Z json-target-spec; check then rustc
release link with linkers/aarch64.ld as Makefile:40-51. Fresh artifact target-dir,
2 jobs, bounded memory/time; retain commands/logs. RISC-V compile gate required
for changed PLIC/HLIC code; cache/toolchain availability unverified, no download.

## Remaining review gates

Before author admission: review exact private lock type/compiler ordering (asm
must not use nomem; acquire/release plus compiler fences around mask transitions),
completion CPU affinity/scheduler call reachability, COM1 receive/event/CLINT
backend lock reentry and scheme HANDLE/drop scopes, controller MMIO Send/alias
proofs including GICv2 bypass, and the runnable new harness design. These are
explicit unresolved proof obligations; this document does not call them PASS.
The chosen ownership interfaces solve core lifetime and handler Busy design;
whole-subsystem safety and source equivalence still require that review. If any
proof fails, revise this contract before code; keep AP/SGI and runtime gates shut.

## Codex/Luna review — 2026-10-26 1:04 PM PT

Accepted as a coherent interface direction, NOT implementation-ready or a safety
proof. Selected guarded-core/owned-action model addresses the lifetime defect in
the rejected partial &self conversion. No handler-slot mutex/Busy fallback is
introduced. Current standalone callbacks have immutable unit/usize fields; their
backend locks still need reentry/lifetime audit. Controller initialization must
use explicit builder context, including global parent/own-controller lookups.

The scoped HANDLES -> core EOI nesting above corrects the original planning draft;
keeping existing kwrite serialization avoids introducing a close/ack TOCTOU. The
core must never call HANDLES/event/callback/logging/backend paths while guarded.
Remaining author-admission gates are the exact guard/compiler ordering, banked
completion CPU affinity, backend/handle lock graph, controller MMIO Send/alias
proofs and executable production seam. These remain unchecked. No AP, SGI, source
author or VM admission follows from this document.

Existing verification packet has a CLI typo: test_irq_callback_eoi.py takes a
KERNEL ROOT DIRECTORY, not a source-file argument. Prior check/link were run via
CARGO_NET_OFFLINE=true and make ARCH=aarch64 check / make ARCH=aarch64
OBJCOPY=aarch64-linux-gnu-objcopy, with existing PATH/toolchain. Raw Cargo commands
printed in logs alone do not enforce offline mode. A new interface necessarily
requires a reviewed harness adaptation preserving the four existing semantics;
unchanged old extractor matching is not proof. No test ran in this design review.

## Isolated source experiment admission — 2026-10-26 01:54 PM PT

Root admits one bounded implementation experiment in detached artifact worktree
`kernel-ownership-author-worktree` at09d0774e. This changes the workflow gate, not
acceptance: proof obligations are conditions for integration, and may be tested
in an isolated patch rather than requiring an executable harness before code
exists. No source integration, AP/SGI enabling, VM or device operation is admitted.
Incomplete policy must stop with preserved partial work and an exact blocker.
All ownership branches must remain coherent; no mixed raw-global/guarded graph is
a shippable checkpoint. Root reviews actual diff, negative controls and offline
compile/link before exporting any canonical patch. Native5090 coordinates the
existing authorized Cursor wrapper with known allowance/mode, reviewed brief hash
and original1200sec deadline; no retry/fallback or overage.

Luna completed the prior backend source callee gaps: COM1 remains locked through
GENI poll_rx sink/debug_input/debug_notify; queue push lock drops before condition
notify, ContextLock drops before wakeup, and local/nonlocal wakeup queues plus
Wakeup SGI contain no directly observed synchronous scheduler switch in those
reviewed paths. This is source-scoped evidence, not a runtime migration/affinity
proof or an assumption about arbitrary future callbacks. HANDLES-through-EOI
serialization and all core/IRQ/MMIO integration checks remain required.


## Preparatory guard accepted — 2026-10-26 2:16 PM PT

The first full author produced no edits; bounded Stage A delivered the unwired
IRQ-save helper as canonical0025/sourceff56c966. `with` is unsafe: caller must
prove no scheduling, blocking, migration or IRQ-enable through cleanup/unwind.
The private !Send token is not CPU pinning. HRTB prevents returning its mutable
borrow; spin guard releases before saved IRQ restoration on normal/unwind paths.
The actual production module passed6host tests, two failing semantic mutations,
normal/lifetime/Send/unsafe-required compile fixtures, A64/RV codegen and offline
A64 check/link. Reproduce using scripts/test_irq_save_guard.py; HANDOFF links logs.
These prove only preparatory source/codegen behavior, not production callsites or
hardware ordering. No IRQ_CHIP wiring or AP/SGI/runtime admission follows.

Spark and Luna source review narrowed one unnecessary proposed prerequisite:
irq_eoi uses stable basic.ic_idx/ic_irq rather than mutable basic.route. A copied
completion target plus owned Arc callback lifetime need not invent a new Retiring
route-lease protocol merely for Rust reference lifetime. Mutable controllers still
require guarded access. Concurrent close/reservation/reconfiguration and actual
in-flight delivery remain functional-policy gates; preserve existing HANDLES
ack/close ordering and never adopt Spark's broad functional-safety conclusion.
Next author must close the complete call graph and prove unsafe call obligations
before a coherent core migration; no mixed raw-global/guarded checkpoint ships.


## Stage B experiment policy override — 2026-10-26 2:49 PM PT

Root/Luna reviewed CURSOR-OWNERSHIP-STAGE-B.md (artifact root), exact SHA
`ee8080ae1a0422f65323e115aa6002e2d6cfb51990c1634ee88b9ebbeff3967b`.
An isolated source experiment at ff56c966 may now use the accepted IRQ-save
primitive rather than repeat the missing-guard author attempt. Prior proof
obligations remain integration gates; partial source is not canonical acceptance.
For this FIRST slice preserve existing immediate close/disable/route-clear and
HANDLES-through-ack behavior. Earlier proposed Retiring/generation/inflight/EBUSY
policy is NOT required or admitted merely for reference ownership. Copied stable
ic_idx/ic_irq and owned Arc callback lifetime do not require it. Functional
concurrent close/reconfiguration/delivery remains SMP-unproved. No API/lifecycle
expansion, AP/SGI, VM or device admission follows. Controller mutable accesses
still require guards; only owned actions/callbacks leave the guard.

Native callsite inventory is incomplete and contains errors: A64 COM1 uses the
patch24 deferred EOI hook, RV COM1 still calls EOI; `init_ap` and fast root paths
are GICv2 (`gic::active()`), not GICv3. Controller EOI is a trait call with
implementation-specific CPU/panic/reentry obligations, not inherently pure MMIO.
Root raw-IAR completion differs from descriptor-derived callback completion.
Raw test accesses must migrate or remain explicitly unverified; they cannot be
ignored in integrated API acceptance. The native's extra fifth read exceeded its
four-command budget. Inventory is evidence to correct, not authority to implement
a mixed raw-global/guarded graph.

The author brief allows one1200sec Cursor experiment, no commit/push/deploy,
with exact scope and source-rooted tests/offline target checks. Registered
InterruptHandler/scheme/event/backend callbacks must run outside core; bounded
controller trait MMIO/claim/EOI steps run inside only after source audit. Current
phone STOP and existing consumed runtime images remain unchanged.

## Controller-effects review and TLMM source preparation — 2026-10-26 PT

The native effects inventory is useful only with callee and build-policy
corrections: kernel dev/release profiles use panic=abort and panic.rs prints then
halts. Runtime unwrap/assert sites are fatal-path concerns, not recoverable unwind
paths. GICv2 fallback GicCpuIf::irq_ack panics on ID1023 (gic.rs:448-457); its
trait wrapper is not clean merely because it has no inline panic. Fixed TLMM
0..149 iteration maps completely to qcom pin offsets, so its unwrap is
invariant-backed rather than an observed reachable panic. GIC one-bit W1
enable/disable writes must remain unchanged. No present race is established.

Root/Luna reviewed one narrow real-source prerequisite: TLMM next-pending-pin
scan clears exactly one enabled pending pin before returning owned data; the
existing handler then emits irq_trigger and resumes after that pin. This may
be accepted as behavior-preserving source preparation only after actual old/new
handler trace comparison, production-helper host tests, behavioral negative
controls and offline A64 check/link. It does not fix the outer core mutable borrow
or integrate the experimental registry. No snapshot-all/clear-all early; later
hardware pending changes during a callback must be seen. Host tests must not
manufacture a second mutable Tlmm reference; use disjoint fake MMIO state.

The isolated author owns only tlmm.rs in kernel-tlmm-step-worktree at accepted
source25ff56c966 plus tlmm-step-evidence. Brief CURSOR-TLMM-STEP-BRIEF.md SHA
d92403d6cf7f6675127253eaf6d95d0bbb78f0510ba8c7287aca3af450c1cfc6,
one600sec worker through the same native5090 coordinator and canonical wrapper,
known allowance/mode only. Launch is not delivery; root must collect actual
terminal/process/diff and proof before any canonical26 source patch. No AP/SGI,
VM, device, serving or old experimental source changes.

## GICv3 diagnostic preparation — 2026-10-26 4:40 PM PT

The preceding TLMM experiment is accepted as source0026 at36faf9cf; its full
evidence and remaining limitations are in the current HANDOFF. It is not an
active author or an authorization to replay that command.

The native5090 GICv3 source packet contains13 verbatim source blocks at this
same base. Root matched each block against source, corrected six line-range
annotations and preserved the raw packet. Unread callees remain unread; this
is not a safety verdict. Root receipt: GICV3-PACKET-ROOT-REVIEW.json under the
existing redox-perplexity artifact directory.

Luna reviewed the next bounded one-file prerequisite: a no-log helper takes
the real RdRegistry, INTID and already captured affinity, and returns either
the RD address or owned missing-mapping diagnostic fields. Existing private_rd
must compute current MPIDR/affinity once and retain its exact single error
message and Option result. No registry or IRQ/MMIO semantics change. This only
prepares future out-of-guard diagnostics; current wrapper logging and outer
IrqChipCore ownership remain unresolved.

Acceptance requires actual extracted old/new wrapper parity, real registry and
affinity source, mapped/missing cases, owned snapshot after registry mutation,
log text/count/order, three behavioral negative controls, and offline A64
check/link. Host MPIDR is a shim, never CPU-affinity proof. Source imports
syscall::Result; use explicit core::result::Result for the new owned error.
No AP/SGI, VM, phone or global-guard proof follows. Admission and outcomes
belong in the single current HANDOFF, not inferred from this design note.

Root accepted source0027 at d752e98f after full actual diff review, portable
42host tests/120parity cases/four behavioral controls and independently rerun
offline A64 check/link. Fresh27patch index equals source tree94e8c58b.
Spark's test-evidence review found no blocker; its general lock-callability
inference is rejected and its u32::MAX parity claim corrected (helper test only).
Current HANDOFF has exact receipts and remaining migration gates.

## Scheme/core packet review — 2026-10-26 5:28 PM PT

The native lock-scope packet is useful only with root/Luna corrections. COUNTS
is a plain spin::Mutex imported at scheme/irq.rs:15, not ordered::Mutex and
not assigned an ordered level. HANDLES is ordered RwLock<L1>. Its read guard
in kwrite also spans usercopy read_usize; a future core guard must not expand
over that operation. Existing HANDLES-through-ack serialization stays required.
Root preserved the raw packet and corrected omitted cfg guards, an inserted
collect semicolon and token-impl ellipsis; all16 code blocks now match source.
SCHEME-LOCK-PACKET-ROOT-REVIEW.json records exact ranges. Do not call abbreviated
or edited source verbatim.

The graph remains unguarded: trigger_virq holds its mutable core borrow across
callbacks/scheme dispatch; open_phandle_irq check/configure/reserve is not an
atomic transaction. register/reservation paths, raw iteration/kopenat/test
accesses and usercopy/transitive lock edges still require full migration review.
No partial mixed guarded/unguarded routing is accepted.

Next bounded prerequisite: remove PLIC whole mapped-register Rust references.
Retain the old packed layout only as offset metadata and use scalar-address
u32 volatile accesses with identical priority/pending/enable/context offsets,
bounds, pre-init/null and zero-claim failures. Enable/disable must preserve the
priority write then single-read/single-write OR/AND-not sequence. Preserve
original !Send/!Sync via a private marker and actual-source compile-fail tests;
no unsafe trait assertions or inference of MMIO exclusivity/CPU safety.

Luna reviewed PLIC-ADDRESS-BRIEF.md SHA
9d3f6aef8b989b210608f85819e8814c7394a5be7c6b4e93dc0faa4f8df9c03a.
One isolated plic.rs author through SAME5090 coordinator is permitted after
fresh admission,600seconds/no retry. Actual source-rooted old/new traces and
behavioral controls plus RV64 AND A64 check/link are acceptance gates. Existing
toolchain rust-objcopy requires its toolchain-bin LD_LIBRARY_PATH; verified
version22.1.6, no install. Exact launch/delivery belongs in current HANDOFF.
This prepares one MMIO aliasing constraint, not global IRQ ownership or SMP.

### Deterministic packet gate

Future Rust source packets must pass the checked-in verifier before claims are
used for authoring:
    python3 scripts/oracle/verify_source_packet.py KERNEL_ROOT PACKET SOURCE...
SOURCE arguments are explicit relative source files. The JSON records unique
byte matches, actual start/end lines and source/packet hashes. It verifies
neither prose, claimed header ranges nor safety. Empty/ambiguous/edited code
blocks reject. Root ran it on this packet: corrected16blocks PASS; raw packet
REJECTS blocks2/3/16, including the inserted collect semicolon. Preserve that
negative-control report with SCHEME-LOCK-PACKET-ROOT-REVIEW.json.

PLIC prerequisite accepted as0028/sourceb8c0b2c1 after author timeout124 and
root completion. Real-source8host tests/444parity cases/four compiling behavioral
controls/four negative+two positive trait fixtures and RV64+A64check/link PASS;
fresh28patch tree equals source. Root corrected unsupported IRQ_CHIP locking
comments. No global ownership, MMIO exclusivity, CPU/SMP/runtime acceptance.
Portable runner: scripts/oracle/plic-address/run.py. Actual current receipts
and next action remain in HANDOFF, not inferred from the earlier author plan.

### BCM owned pending/claim prerequisite

0029/source5cd10012 extracts no-log pending data and a copied valid-virq claim,
used by the real BCM registered-handler-only path. Existing ack wrapper emits
its own diagnostic once; handler emits once on its separate path. Pending
diagnostic only accompanies unmappable hwirq, so map-before-log cannot add an
overflow before that diagnostic. Normal flow includes missing handler and
invalid mapping; callback panic/abort does not reach trailing EOI. No fallback
or eoi_after policy added. Preserve ordinary arithmetic: zero/high-only BASIC
and zero summary words panic in debug and wrap in release. Native review's
F1 was an ambiguity, not a discovered proposal defect; bits10..20 are fast-map,
while high-only bits21..31 hit masked-zero arithmetic.

Portable scripts/oracle/bcm-step/run.py compares actual baseline/candidate
methods in debug/release with1500cases/profile; four compiled controls/profile
must fail named parity assertions. A64check/link and fresh29patch index tree
equality pass. Boundary substitutions and host panic-catching are not kernel
abort/global ownership/MMIO/CPU/SMP/emulator/phone proof. Current handles and
receipts remain in HANDOFF.

### GICv2 continuation constraints

Accepted29 source has two distinct acknowledge paths. irqchip/mod.rs first
uses gic::acknowledge_local when GICC_BASE is nonzero; this reads raw IAR,
virq_for rejects masked IDs1020..1023, and interrupt/irq.rs skips EOIR for
those IDs in its no-virq branch. The trait GicCpuIf::irq_ack panic on1023
belongs to the fallback path, not the ordinary local helper. end_root first
attempts the raw local EOIR helper before trait fallback. Preserve raw-IAR
completion identity; do not replace these distinct policies with one generic
spurious rule. Dispatch also has early IPI/timer returns and child-root-EOI
policy, so controller methods alone are not a complete caller proof. Source
ordering/publication is not proven runtime reachability or SMP ownership.
Current packet hashes, reviews and next source task remain in HANDOFF.

The root-selector also preserves an earlier selection when ROOT_IC_IDX_IS_SET
is nonzero; otherwise it resolves the DT root interrupt-parent, falls back to
the first parentless chip and panics if neither resolves. Do not omit the early
return. gic::active() requires BOTH GICD/GICC bases and gates AP initialization
and local enabling; acknowledge_local/end_local test only GICC_BASE. A review
that substitutes active() as the runtime ack/EOI condition is incorrect. These
source conditions do not establish when a partial initialization state can be
observed at runtime or prove a complete publication/ownership contract.

### BCM2836 inclusive descriptor domain

Accepted0030 corrects the existing LAST9/count9 mismatch: mapper allowed IRQ9
but allocation initialized0..8. Count LAST+1 makes descriptor ownership and
next-domain start consistent. It preserves the1024write cap and declared-count
advance near that cap, parser-error path and preselected-root stores. Actual
initializer/mapper host regressions, A64check/link and fresh30patch equality
pass; hardware/global publication/alias/SMP/runtime behavior is not proved.
Native source header hash and always-wrap/no-panic prose were corrected from
actual byte-verifier/source pin evidence; a packet match alone is not prose
acceptance. Current receipts and source handles remain in HANDOFF.
