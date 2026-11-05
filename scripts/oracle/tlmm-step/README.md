# TLMM source trace parity

Run from the private project with a kernel checkout containing source patch0026:

    python3 scripts/oracle/tlmm-step/run.py KERNEL_ROOT NEW_EVIDENCE_DIRECTORY

Offline cached nightly2026-05-24 only; output directory must be new. No VM,
redoxer, phone, network download or dependency installation is run.

before.rs is the verbatim TLMM source from accepted25/ff56c966, captured before
the extraction and checked by SHA-256
fad54896de565fb8ffb394c686a252d37cf80cd38a9413e3b4d4ac03cdf91142.
Keeping the fixture avoids requiring this local kernel commit in fresh private
patch-series reproductions. extract.py copies actual structs/methods into the
generated harness; it does not rewrite the scan algorithm. The current checkout's
real qcom pin table is included. Volatile hooks model W1C status and trace actual
handler reads, clears and events. Callback mutations touch disjoint fake MMIO
state; they never create a second mutable controller reference.

Baseline8tests and new10tests pass, including exact full trace equivalence.
Four isolated mutants must compile and fail named ordering/coverage assertions:
omit-clear, late-clear, skip-next and drop-later. Includes pin0/149, tile spans,
probe-only, disabled/nonpending, cursor bounds, owned step, and later pending
changes during events.

This proves source-preparation protocol against fake MMIO. It does not prove
hardware W1C/volatile behavior, token semantics, global core borrow safety, CPU
affinity, SMP or runtime. Existing outer trigger_virq mutable borrow is unchanged.
