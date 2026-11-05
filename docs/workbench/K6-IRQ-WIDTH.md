# K6 — IRQ numbers > 255 (evidence)

Worktree `~/Developer/forks/redox/kernel-k6`, branch `op6/irq-width` (from `op6/geni-uart` `58316f0f`). Not pushed.

## Commits

- `9a051a61` irq: carry IRQ numbers wider than u8 through the irq scheme
- `e428c087` gicv3: keep the full INTID when acknowledging an interrupt
- `257aff94` devices: deliver GENI debug UART RX through the SE interrupt

## What was wrong (all on the path SPI 354 → virq 386)

| Place | Bug |
|---|---|
| `scheme/irq.rs` `irq_trigger(u8)`, handles, `COUNTS: [usize; 224]` | virq truncated / no queue above 223; `open_phandle_irq` rejected virq ≥ 224 |
| `dtb/irqchip.rs` `trigger_virq` | `irq_trigger(virq as u8)` silently aliased virq 386 → 130 |
| `arch/{aarch64,riscv64}/device/serial.rs` `Com1Irq` | `irq.try_into().unwrap()` to u8 → panic in IRQ context |
| `dtb/irqchip.rs` `is_reserved`/`set_reserved`, `irq_iter_for` | u8 index / `ic_irq as u8` |
| `scheme/irq.rs` `Handle::Phandle(u8, …)` | phandles > 255 (normal on SDM845 DTs) truncated in fpath/fstat |
| `gicv3.rs` `irq_ack` | `IAR & 0x1ff`: INTIDs > 511 aliased (SDM845 SPIs reach 608 direct, 636 via PDC = INTID 668); spurious 1023 became 511 and was EOI'd |

Type choice is `u16` for scheme IRQ numbers, the same as K1 (`op6/irqchips`), so the scheme hunks mostly coincide.
x86 IDT vectors stay `u8`. DT queue length is `dtb::irqchip::IRQ_DESC_COUNT` (1024 here); the test
`every_irqchip_virq_has_a_queue` asserts it equals `IRQ_CHIP.irq_desc.len()`, so it fails loudly if a merge
with K1 (2048 descriptors) leaves them out of step.

GENI RX: `enable_irq` programs `UART_RX_STALE_CNT` = 160, unmasks `S_IRQ_EN` RX watermark|last (bits 26|27),
`M_IRQ_EN` SEC_IRQ (bit 31) and `SE_IRQ_EN` M|S (bits 2|3). The console handler (`service_irq`) clears the S
status and only the M SEC bit, then drains. TX stays polled and no TX event is unmasked.

## Reproduce

```sh
export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc
export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_RUNNER="qemu-aarch64-static -L /usr/aarch64-linux-gnu"
cargo test --target aarch64-unknown-linux-gnu -- --skip context::switch::tests::test_work_stealing
make check ARCH=aarch64; make check ARCH=riscv64
make ARCH=aarch64 OBJCOPY=aarch64-linux-gnu-objcopy
```

## Evidence (this folder)

- `k6-host-tests.log`: **55 passed** (K2 baseline 37; 19 new — 7 scheme, 2 irqchip, 3 GICv3, 7 GENI IRQ — and
  K2's `enable_irq_keeps_se_interrupt_delivery_masked` removed because the behaviour it pinned is gone). Key: `trigger_counts_a_virq_above_255`, `unhandled_virq_above_255_reaches_its_queue`,
  `registered_handler_sees_the_full_virq`, `spi_354_round_trips_through_virq_386`, `iar_keeps_intids_above_511`,
  `keystroke_raises_the_line_and_service_drops_it`, `polled_tx_never_raises_the_line`.
- `k6-kernel-check.log`, `k6-kernel-check-riscv64.log`, `k6-kernel-build.log`: EXIT=0.
  x86_64 `make check` also passes but only with a throwaway `nasm` stub (empty trampoline; `nasm` not installed) — no log kept.
- `k6-qemu-boot.log`: private copy of the W0B live ISO with this kernel (`k6-live.iso`, sha256 in `k6-sha256.txt`),
  QEMU `virt,acpi=off,gic-version=3 -cpu max -smp 1`. GIC irq_range (0, 288), PL011 serial virq 33, `redox login:`,
  typed `user` + `echo K6_SHELL_OK` over the console IRQ (new `u16` handler path), `K6_SHELL_OK` printed,
  0 `unexpected irq`. The `pcid irq_helpers.rs:313` panic is the W0B baseline (`../w0b-shell-final.log:153`).

## Gaps

- QEMU virt has 256 SPIs (virq ≤ 287) and no GENI device, so virq ≥ 386 and the GENI IRQ path are proven by host
  tests and the register model only. First real proof is the phone: expect `serial_port virq = 386`.
- `RX_STALE_TICKS` = 160 assumes 10 on-wire bits (spec §3); GENI RX error bits (parity/break/overrun) are cleared
  but not reported. Level-interrupt behaviour of SEC_IRQ is modelled from OpenBSD, not a captured trace.
- `cpu_stats::add_irq(u8)` (per-IRQ stats, commented out on aarch64/riscv64) is still u8 — left alone.
- Merge with K1: conflicts expected in `scheme/irq.rs` (queue length line), `dtb/irqchip.rs` (`IRQ_DESC_COUNT` vs
  literal 2048, `is_reserved`), `generic_timer.rs` (both widen the same line). Take K1's 2048 by setting
  `IRQ_DESC_COUNT = 2048`.

## PROVENANCE entries (for `docs/PROVENANCE.md`)

| Our file | Upstream | Kind | SPDX of source |
|---|---|---|---|
| `kernel src/devices/uart_qcom_geni.rs` (IRQ enable/ack bits) | OpenBSD `sys/dev/ic/qcuart.c` @ `20cb344` (S_IRQ 26/27, M SEC_IRQ 31, intr ack order); U-Boot `include/soc/qcom/geni-se.h` @ `8649a54d` (SE_IRQ_EN bits 2/3, facts only) | spec-only | ISC; GPL-2.0+ (facts only, no code) |
| `kernel src/arch/aarch64/device/irqchip/gicv3.rs` (INTID width) | Arm GICv3 architecture: ICC_IAR1_EL1.INTID [23:0], 1020–1023 special | spec-only | n/a |
