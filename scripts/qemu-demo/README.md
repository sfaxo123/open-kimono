# scripts/qemu-demo — device-free QEMU gate for the Wasm service sandbox

Builds and boots a Redox aarch64 RAM image that runs `apps/sandbox-demo`, then
asserts on the serial console that the sandbox refused what it was supposed to
refuse. Nothing here writes outside this worktree: every artifact goes to
`out/qemu-demo/` (gitignored), and the fork trees (`forks/redox`, `base-e1`,
`kernel-e1`, `bootloader-e1`) are read-only inputs — the cookbook container
mounts `/mnt/redox` with `:ro` and uses a worktree-local `CARGO_HOME` over a
read-only `RUSTUP_HOME`.

```
scripts/qemu-demo/build.sh       # hello_rs.wasm, digest gate, cargo vendor, aarch64-unknown-redox ELF
scripts/qemu-demo/check-digest.sh# fresh wasm == embedded asset == source pin (build.sh calls it)
scripts/qemu-demo/stage.sh       # initfs copy + 45_sandbox_demo.service + pack + ramboot EFI + FAT ESP
scripts/qemu-demo/boot.sh        # DTB overlay, bounded QEMU boot (no NIC), strict validator
scripts/qemu-demo/validate.sh    # the acceptance rule, in one place
scripts/qemu-demo/evidence.sh    # receipts + ledger + docs/workbench/QEMU-SANDBOX-DEMO.txt
scripts/qemu-demo/tests.sh       # fault-injection tests for the gate itself
```

Run them in that order. Overrides: `REDOX_FORKS`, `REDOX_COOKBOOK`, `CARGO_HOME`,
`QEMU_DEMO_OUT` (artifact dir, default `out/qemu-demo`), `QEMU_BIN`,
`BOOT_TIMEOUT` (default 300 s). `stage.sh` needs passwordless sudo to loop-mount
the ESP image (mtools is not installed on this host).

## What counts as a pass

`validate.sh` is the only acceptance rule, and both `boot.sh` and `evidence.sh`
consult it. A capture is accepted only if, after ANSI stripping, it contains
`SANDBOX_HELLO_OK`, `SANDBOX_REFUSED_TRACEEMIT`, `stopped service is never
re-entered` and a terminal `sandbox-demo: done` that comes after the other
three — and contains no `sandbox-demo: FAIL` anywhere. Two markers alone prove
nothing: the app prints them before its last assertion, so a run that then
failed its re-entry check used to look like a success. `apps/sandbox-demo`
prints its markers only after the assertion that justifies them, so `done` is
the last line the demo can emit.

`boot.sh` polls the capture through `validate.sh --poll` (exit 0 accept, 1 keep
waiting, 2 FAIL seen), and QEMU is bounded: `timeout --signal=TERM
--kill-after=5` plus `-pidfile` and an EXIT trap that TERMs, waits, then KILLs
both the wrapper and the guest, so an interrupted boot leaves no orphan.

`build.sh` refuses to cross-build if the freshly built guest does not hash to
the asset embedded in the binary and to the pin in `src/main.rs`
(`check-digest.sh`). Otherwise a stale committed asset could be booted while the
log claims the digest of the wasm that was just built.

## Evidence

`evidence.sh` validates first and writes nothing at all if the capture is
rejected. For an accepted capture it writes an immutable per-run receipt under
`docs/workbench/evidence/<run-id>-{receipt.tsv,serial.txt}`, appends one row to
the append-only ledger `docs/workbench/QEMU-SANDBOX-DEMO-runs.tsv`, and
regenerates `docs/workbench/QEMU-SANDBOX-DEMO.txt` from that run. The run id is
derived from the capture's mtime and digest, so re-running the boot cannot
overwrite the receipts backing an earlier document. `--serial <capture>`
documents a capture that is not the current `out/qemu-demo/serial.raw`.

The build chain is not bit-reproducible — `boot.sh` regenerates the DTB on every
boot and QEMU's `dumpdtb` embeds random kaslr/rng seeds — so the hashes are
per-run receipts, not identity proofs across runs.

## Testing the gate

`scripts/qemu-demo/tests.sh` needs no QEMU and no build: it injects fault
captures (two markers plus `FAIL`, missing `done`, missing re-entry assertion,
`done` printed early, empty), checks the validator rejects every one and accepts
the healthy ones, shows that the old two-grep predicate still accepts the faults,
exercises the digest gate, and runs `boot.sh` against a stub QEMU that ignores
TERM to prove the timeout and the trap both reap the child. It writes only under
`out/qemu-demo/tests/`.
