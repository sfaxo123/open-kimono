# GICv3 diagnostic helper source parity

Run from the project checkout, with a freshly applied kernel series:

```sh
python3 scripts/oracle/gicv3-rd/run.py KERNEL_ROOT NEW_EVIDENCE_DIRECTORY
```

The runner refuses an existing evidence directory. It uses cached
nightly-2026-05-24 and offline Cargo; it does not download or boot anything.

`before.rs` is the verbatim accepted26 production file, SHA256
`4acca44f812ad8c295b9e06005698b6fe8062e15aa09dd113f4872e5fee1a4d5`.
It is an upstream source fixture, not code to refactor. The actual current
model source is included by path and pinned by hash. Real old/new methods
are extracted by signature and brace boundaries, with outside-extraction
identity checked. This avoids depending on an unpublished local base commit.

The 42 tests comprise 30 existing model tests, nine current-item tests and
three parity tests. Wrapper parity covers 120 cases. Four controls must
compile and then fail named behavioral assertions; the final clean run must
pass. Receipts and logs stay in the requested evidence directory.

MPIDR and logger are host shims; only formatted message text is compared.
No CPU, real MMIO, kernel logger prefix, global alias safety, AP/SMP, emulator
or phone proof follows. Kernel AArch64 check/link are separate gates.
