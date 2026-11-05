# Rust migration slice: trace converter

This host-only CLI converts mapped Linux `rwmmio` trace events to the JSONL format consumed by `mmio-replay`. It is a Rust implementation of `scripts/oracle/trace/convert.py`; the Python version remains the compatibility oracle until this slice is reviewed.

Run from this directory:

```sh
cargo run --offline -- <raw-trace> <output.jsonl> \
  --virtual-base 0xffff1000 --physical-base 0xa84000 --size 0x1000
```

Pass `--caller <regex>` to filter callers or `--dry-run` to validate and count records without writing the output. Tests include standalone Rust behavior checks and differential JSONL comparison against the existing Python oracle. Python is used only by the integration tests, never by the Rust CLI.

```sh
cargo test --offline
```

## Compatibility limits

This candidate does not yet provide full Python CLI parity. Trace VALUE fields now support signed ASCII decimal/hex/octal/binary values beyond `u128`, reducing them modulo the access width without allocating a big integer. Differential cases include negative values, long values, accepted separators and malformed-input output preservation. Addresses, widths, timestamps and CLI numeric parameters still use bounded `u128`; Python's Unicode digit grammar and configurable decimal-digit limits are not mirrored. Python caller patterns may also use features, including lookaround or backreferences, that Rust's `regex` crate rejects or interprets differently. CLI diagnostics and exit details are not byte-for-byte compatible with `argparse`.

These cases gate switching callers from Python to Rust. Keep the Python implementation as the active caller until required input ranges and caller patterns are inventoried, the compatibility gaps are resolved or explicitly bounded, and the remaining behavior has differential coverage. `parity_regression_negative_values` is now a normal passing test; no known-gap test is hidden by an ignore. Passing the covered differential cases does not establish complete Python compatibility or authorize switching callers.

## Tracked source inventory contract

Run the path/native-boundary inventory contract with:

```sh
cargo test --offline --test inventory_contract
```

`inventory.json` pins a historical tracked-tree baseline and enumerates every `.py` and `.sh` path, extensionless executable/shebang script, and `.patch` surface at that revision. The Rust test requires the pinned commit to remain an ancestor, reconciles those path sets against both the baseline and current tracked index, derives native C/header/assembly patch targets and inline-assembly references, validates embedded-generator and Rust FFI/native-boundary markers, and checks that recorded full hashes have 40 hexadecimal characters. Rust migration files added after the baseline are listed separately under `rust_sources_added_after_baseline`; this source delta is not silently folded into historical inventory. A Git fixture proves a later documentation-only descendant is accepted; mutation tests remove or duplicate script and patch rows and reject fabricated native targets. The inventory explicitly excludes the native-marker detector from self-scanning.

This is a path and source-marker contract, not a semantic feature inventory. The operator has confirmed all upstream U-Boot and QEMU platforms/features are in scope; feature-level coverage remains explicitly pending. No empty native-target list or filename-based classification is evidence of feature parity.
