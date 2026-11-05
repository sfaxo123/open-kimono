# K13 host verification

Command: `~/.cargo/bin/cargo test --manifest-path crates/qcom-rproc/Cargo.toml --offline`
Result: PASS, 8 integration tests, 0 unit and 0 doc tests; full output: `K13-CARGO-TEST.txt`.
The 8 suites include 29 explicit negative assertions (`Err(Error::...)`) covering invalid PAS IDs/addresses/status, absent and out-of-bounds SMEM items, duplicate inserts, GLINK capacity/version/intent/frame errors, QRTR bad packet/duplicate service, and QMI missing/duplicate/truncated TLVs and invalid message type.

Command: `~/.cargo/bin/cargo fmt --manifest-path crates/qcom-rproc/Cargo.toml --check`
Result: PASS.

Command: `~/.cargo/bin/cargo clippy --manifest-path crates/qcom-rproc/Cargo.toml --all-targets --offline -- -D warnings`
Result: PASS; full output: `K13-CLIPPY.txt`.

No QEMU or phone run: QEMU has no SDM845 remote processor and this task prohibits phone access. Firmware load, actual SMC conduit, cache-coherent SMEM mappings, GLINK mailbox/IRQ, QRTR nameserver control messages, and QMI service-specific schemas still need a platform adapter and oracle trace acceptance. See `specs/qcom-rproc.md`.
