# BCM2835 owned pending/claim step source parity

Run from the private project with an existing kernel checkout and a NEW evidence directory:

    PATH=$HOME/.cargo/bin:$PATH python3 scripts/oracle/bcm-step/run.py KERNEL_ROOT NEW_EVIDENCE_DIRECTORY

The immutable `baseline.rs` is the accepted28 upstream-derived BCM source fixture (SHA256 pinned in `run.py`), not an alternative implementation. The runner extracts actual `ffs`, ack, mapping, handler and candidate owned helpers. Hardware reads become ordered register-buffer reads; the global registry becomes a separate local registry parameter; logging is captured; empty EOI calls are instrumented. Callback code never constructs another mutable controller reference.

Four tests run in both debug and release. There are 1,500 old/new scenarios per profile: BASIC pending bits, representative summary-bank bits including31 and zero, mixed priorities, mapping boundaries/overflow, handler absence/presence/panic, direct ack and handler. Debug panics and release wraps are compared. Exact quiet/log/callback/EOI traces and owned snapshot behavior are also asserted. Four compiled mutants (wrong fast map, wrong pending read, extra quiet-path log, missing invalid-path EOI) must fail the named parity test in each profile.

The boundary substitutions and host panic-catching do not prove device-memory attributes, MMIO exclusivity, kernel abort handling, CPU affinity, global borrow correctness, registration/init, SMP, emulator or phone behavior. Bank bits are sampled, not exhaustive. Builds/checks are separate acceptance evidence. No model/server/device operation is performed.
