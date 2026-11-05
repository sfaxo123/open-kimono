# qcom-rproc

MIT, `no_std + alloc` protocol core for PAS, SMEM, GLINK, QRTR V1 and QMI TLVs.
`cargo test --manifest-path crates/qcom-rproc/Cargo.toml --offline` runs host models.

This crate does not execute SMC, map physical memory, ring a mailbox, load
firmware, or start a remote processor. The platform adapter must supply those
operations, plus DMA-safe buffers, cache barriers, power/clocks, IRQ and SMEM
partition support. SDM845 PAS IDs and transport traces need oracle validation
before hardware use. See `specs/qcom-rproc.md` and `docs/PROVENANCE.md`.
