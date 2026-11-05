# K8c — IRQ-width re-integration and drive-free boot

Verified 2026-10-24 4:56 PM PT. Only the K8c kernel/base/bootloader
worktrees, the Redox build checkout, this project worktree, and the existing
phone-boot payload artifact were written. No phone access or push.

## Source and patch commits

- Kernel `op6/integration-3`: `2780adc1` (u16 IRQ scheme), `df2ee0b6`
  (GICv3 24-bit INTID), `d73469ef` (GENI UART RX interrupt), atop K8b
  `f922469d`. `IRQ_DESC_COUNT = 2048` and the IRQ queue length follows it.
  K1's handler-aware reservation check was retained.
- Base `op6/integration-3`: `18400fde` (unchanged K8b head).
- Bootloader `op6/integration-3`: `41317dc` (unchanged K8b head).
- Project `build/integration-3`: `d18baca` updates `scripts/integrate.sh`,
  exports the resolved K6 series in `patches/integration-3/kernel/`, and
  refreshes the full `patches/integration/{kernel,base,bootloader}/` export.
  The original `patches/kernel-irq-width/` is unchanged.

## Verification

- `CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc`
  and `CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_RUNNER='qemu-aarch64-static -L /usr/aarch64-linux-gnu'`
  with `cargo test --target aarch64-unknown-linux-gnu -- --skip context::switch::tests::test_work_stealing`:
  **59 passed, 0 failed, 1 filtered**. This includes
  `every_irqchip_virq_has_a_queue` and
  `spi_354_round_trips_through_virq_386`.
- `make check ARCH=aarch64`: exit 0. `bash -n scripts/integrate.sh`: exit 0.
- In the base worktree, `cargo test -p simplefb -p geni-i2c-core -p rmi4-touch-core --offline`:
  17 simplefb, 4 GENI I2C, and 5 RMI4 tests passed.
- `bash scripts/integrate.sh --qemu`: exit 0. It built the aarch64 Redox
  kernel, the RAM initfs inside Podman using the cookbook toolchain, and the
  EFI payload. The initfs contains `spmid`, `geni-i2cd`, `rmi4d`, `inputd`,
  and `vesad`; neither touch daemon has an auto-start service.
- DT-only QEMU: `virt,acpi=off,gic-version=3`, `-cpu max -smp 1`, 2 GiB,
  EDK2, `-device ramfb`, 800x600 simplefb overlay; EFI loaded by PXE/TFTP,
  **no block device**. [`K8C-QEMU.txt`](K8C-QEMU.txt) records
  `REDOX_RAMBOOT=1`, `spmid: no SPMI arbiter in device tree; skipping`,
  `ram:~$`, and `vesad: simplefb 800x600 ... from device tree`. The
  45-second timeout stops the idle shell. EDK2/ACPI probes log expected
  nonfatal errors in this DT-only mode; vesad starts without a fault.

The host test log, build logs, and QEMU DTB are in ignored `out/integration/`.
The tracked QEMU log has carriage returns removed for readable diffs; it is 10,027 bytes, SHA-256
`f4ebaebe94dd748abaa529e34158237082dc0c0ba84941cdd00559cdaa23cc0b`. The raw log remains in ignored `out/integration/qemu.log`.

## Refreshed payload

- `$HOME/artifacts/handoff/oneplus-redox-20261024/phone-boot/redox-ramboot.efi`
- Size: 12,003,840 bytes
- SHA-256: `3b025a82185e16daf11f6948e35e8c0e5d957b4f7007771908251e0111490f28`
- `phone-boot/SHA256SUMS`: `sha256sum -c` passed for all listed artifacts.

The EFI bytes can vary on a repeat build; this QEMU log corresponds to the
payload hash above. QEMU virt has virqs only through 287 and no GENI or
SDM845 hardware. The wide IRQ path is proven by host tests, while the phone's
virq 386 console, physical display, PMIC, touch, and ABL boot remain hardware
checks for the main operator's future phone run. Do not infer them from this
QEMU boot.

Reproduce with `bash scripts/integrate.sh --qemu` from this project worktree;
verify the new payload hash and `sha256sum -c SHA256SUMS` in `phone-boot`.
For rollback, restore the previously verified payload and SHA256SUMS from
artifact history, or revert the three K8c kernel commits on this branch and
rebuild. Stop if the source branches or worktree ownership differ from the
paths above, or if the QEMU shell and graceful `spmid` skip disappear.
