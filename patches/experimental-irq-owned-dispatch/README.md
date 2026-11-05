# Unwired IRQ dispatch experiment — not in the active series

Source `da04b2bfc8fe3f66155b5bc5e1b4bc6ae69ff1aa` is an isolated experimental child of accepted source25
`ff56c966`. It is NOT patch0026, not a canonical ownership prerequisite, and not
production/SMP/hardware proof. Apply prototype.patch only on an isolated source25
checkout. All existing raw IRQ_CHIP access families remain unchanged.

The generic Ready-gated registry uses the accepted IRQ-save primitive and owned
Arc/copy actions. Mock controllers exercise its control flow; there is no real
DispatchCore adapter, builder/publication integration, raw-IAR path, or actual
controller MMIO parity proof. Do not integrate this module merely because it
compiles. Preserve immediate close/disable policy; no Retiring lifecycle added.

Root/Luna corrected a stolen cfg(dtb) attribute and separate reservation check/set
by adding one-lock try_reserve. Reserve admissions must use that transaction;
set_reserved(true) remains an unchecked low-level primitive, not an admission API.
The mock Send implementation was unnecessary and removed. Root tightened negative
controls to require actual named test failures, not any compiler failure.

Reproduce from the private project:

```sh
python3 patches/experimental-irq-owned-dispatch/test_irq_ownership.py KERNEL_ROOT NEW_EVIDENCE_DIRECTORY
```

Existing cached nightly2026-05-24/spin0.9.9 required; offline only. The command
includes actual prototype/guard source with mock core, controllers and IRQ mask.
Root run:18tests PASS (12prototype +6guard),6intentional behavioral mutations FAIL
named tests, Send-core success +not-Send/not-Sync/borrow-escape compile constraints
PASS. Offline A64 check/link and RV64 check PASS after corrections. No VM, Miri,
phone, real-controller path, CPU affinity or MMIO exclusivity proof. Evidence:
$HOME/artifacts/handoff/redox-perplexity/ownership-author-evidence/stage-b/root-reviewed/REVIEW.json.

Cursor wrapper transcript ended Service Unavailable exit1 even though saved build
and host artifacts exist; process is terminal. Launch/exit status is not patch
acceptance. Remaining work: actual controller adapters and all global/init/scheme
access families, real-source regression seam, banked completion affinity,
MMIO/Send and lock graph proof, explicit raw tests migration. Existing protected
runtime/images/AP/SGI and physical STOP unchanged. Future author starts from the
experimental commit only with explicit source ownership and reviewed scope.
