# Driver specifications

Write one Markdown file per driver. Each specification should name the hardware block and device-tree `compatible` value, its dependencies, register map, initialization sequence, interrupts, DMA and power requirements, Redox interface, and verification steps.

Cite every hardware claim to the relevant Linux, U-Boot, or edk2 source using repository path and line number (`file:line`). Record the source revision so line references remain reproducible. Note any gaps or conflicting references before implementation.

## Template (every spec)
1. **Block** — name, DT `compatible`, node path, `reg`, `interrupts`, clocks, resets, supplies, pinctrl (from
   `linux/arch/arm64/boot/dts/qcom/sdm845.dtsi` + `sdm845-oneplus-common.dtsi` + `-enchilada.dts`, file:line).
2. **What firmware leaves configured** (ABL/U-Boot) and what the driver must (re)do.
3. **Register map** used by the minimal driver (offset, field, meaning) — each line cited.
4. **Sequences** — init, operate (TX/RX/transfer/event), error recovery, shutdown — numbered steps, cited.
5. **Interrupts / DMA / power** — polling-first plan, then IRQ path (which irqchip), DMA needs, SMMU notes.
6. **Redox interface** — kernel piece vs user-space daemon, scheme name, `/scheme/memory/physical@uc` mappings,
   `/scheme/irq` usage, start from `exampled`.
7. **Verification** — host tests (register-model or recorded trace), Linux-oracle capture to compare (which
   debugfs/trace/devmem commands on the running pmOS phone), on-phone acceptance via `fastboot boot`.
8. **License path** — references used and their licenses; `MIT clean-room` unless a GPL-only source was needed
   (then `GPL-2.0`, separate crate). Gaps/conflicts listed explicitly.

Local sources (3090): `~/Developer/forks/{linux,u-boot,openbsd-src,edk2-sdm845,redox/*,pmaports}`.
