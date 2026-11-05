# PLIC scalar-address source parity

```sh
python3 scripts/oracle/plic-address/run.py KERNEL_ROOT NEW_EVIDENCE_DIRECTORY
```

Uses cached nightly-2026-05-24 and offline Cargo. Refuses an existing evidence
directory. No download, emulator, or device access.

`before.rs` is verbatim accepted27 PLIC source, SHA256
`b0c4fa1dcc5c556f9a7c42955fabb992b6d5d0e8f6814387a4187ea7499a66e1`.
The three actual syscall Io source files are hash-pinned. The extractor copies
real accessors, PLIC operations, initialization assignment/tail, and Io bodies;
only the volatile boundary and target cfg are adapted for host observation.
The full DTB/init graph is not exercised. No unpublished kernel base commit
is needed by the runner.

Eight tests compare 228 accessor and 216 PLIC-method cases, including ordered
32-bit read/write traces, return values and caught host panics. Four controls
must compile and fail named assertions. Four Send/Sync compile-fail fixtures
and two marker-removal positive controls check preservation of auto traits.

Seeds are deterministic. The formulas check the task's explicit offsets and
baseline behavior; they are not separate hardware-spec conformance proof.
Invalid context64 enable/disable preserves the priority write before the
context assertion. Zero claim returns None at the accessor and panics in the
controller wrapper. Kernel panics abort; host catch_unwind only observes parity.

Host buffers, PHYS_OFFSET=0 and I/O hooks do not prove a physical linear map,
device memory semantics, CPU affinity, MMIO exclusivity, global IRQ ownership,
SMP, emulator or phone behavior. RV64/A64 check/link remain separate gates.
