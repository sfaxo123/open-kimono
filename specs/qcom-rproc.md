# Qualcomm remote-processor core (K13)

## Block and scope
SDM845 ADSP PAS is `qcom,sdm845-adsp-pas` at `/remoteproc-adsp`; its GLINK edge is `lpass`, remote pid 2, with IRQ SPI 156 and mailbox 8. ADSP PAS has watchdog/fatal/ready/handover/stop-ack interrupts, XO clock, `adsp_mem`, QMP and SMP2P stop state. The node is disabled in the generic SDM845 DT and board enablement must be checked before boot. SMEM is `qcom,smem` at 0x86000000, size 0x200000, `no-map`, hwlock 3. Sources: Linux `arch/arm64/boot/dts/qcom/sdm845.dtsi:802-807,903-934` @ `5b0764a53235d3c77e1ed674e836e1b3fe1a4b1f`.

## Firmware and dependencies
SCM/PAS requires secure monitor access and authenticated firmware in a reserved DMA-safe region. Firmware parsing/loading, memory allocation, QMP, clocks, regulator votes, SMP2P, IRQ and mailbox drivers are outside this crate. No assumption that ABL leaves ADSP running. OpenBSD `sys/dev/fdt/qcpas.c:392-442,453-565` gives the PAS ordering (metadata, memory setup, authenticate/reset); `qcscm.c:830-930` gives call service/command and argument types. Host code only prepares requests behind `Smc`, never executes an SMC instruction.

## Interface and minimal sequences
1. Validate the caller-selected PAS ID from a platform allowlist; metadata and memory region are caller-owned. `init_image(id, metadata_pa)` → `mem_setup(id, addr, size)` → `auth_and_reset(id)`. Reject zero/misaligned addresses and zero size. `shutdown(id)` is explicit. Do not start remote cores just because a client opened a scheme.
2. Lookup pre-existing SMEM items by ID and host. Bounds-check item count, allocation flag, offset and length before exposing a slice. The first 8 items are fixed; OpenBSD `qcsmem.c:31-46,358-372,450-480`. This crate implements a host model and a checked global-table image reader; partitioned SMEM and hwlock-backed allocations remain a hardware gate.
3. GLINK uses the SMEM transport ring. Negotiate version, open a named channel, advertise RX intents, transmit only within a granted remote intent, and return `RX_DONE` after consuming data. Command IDs from OpenBSD `qcpas.c:716-731`; actual ring framing/flow control must be checked with oracle traces before device use. The host model rejects duplicate channels, absent intents and overflow.
4. QRTR packet V1 has eight LE u32 fields (version, type, src node/port, confirm, payload length, dst node/port), Linux `net/qrtr/af_qrtr.c:30-50`. A nameserver maps `(service,instance)` to node/port. Local name and port bookkeeping is not yet wire-level nameserver control messaging.
5. QMI service messages use type/transaction/message/length followed by `type:u8, length:u16 LE, value` TLVs. Decoder must reject truncated headers, lengths and duplicate required TLVs. Golden vectors test bytes; the oracle must capture the actual target service IDs and payloads.

## Register map, interrupts, DMA and power
No MMIO register writes in this core. SMC requests and shared-memory byte windows are explicit interfaces. The ADSP GLINK interrupt is SPI 156; mailbox 8 signals peer ring changes (`sdm845.dtsi:926-932`). DMA/cache coherency and SMMU state are unverified. Caller must provide coherent reserved memory and power/clock readiness before hardware use. Stop/shutdown only by explicit owner request.

## Redox placement and verification
Pure `no_std + alloc` crate under `crates/qcom-rproc`; a future privileged scheme daemon owns physical mappings, mailbox and SMC bridge. Host tests exercise PAS ordering/errors, global SMEM lookup, ring/channel/intent behavior, QRTR packet/name/port routing and QMI golden vectors plus malformed cases. Hardware acceptance requires a read-only Linux oracle trace of PAS call order/status, SMEM TOC, GLINK ring/version/intent traffic, QRTR nameserver traffic and QMI service messages, then a separate fastboot-booted Redox test. This task neither touches the phone nor claims hardware validation.

## License and gaps
Original MIT implementation based on documented behavior only; no upstream code copied. OpenBSD `qcscm.c`, `qcpas.c`, `qcsmem.c` are ISC; Linux DT and wire-format files are GPL oracle only. Source revisions and use are in `docs/PROVENANCE.md`. Gaps: SDM845-specific PAS IDs, secure monitor ABI/SCM conduit, real SMEM partitions and hwlocks, GLINK transport/cache barriers, QRTR wire nameserver exchange, and firmware images remain unverified.
