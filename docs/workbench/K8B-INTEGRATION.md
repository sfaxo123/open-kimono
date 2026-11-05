# K8b — drive-free integration with simplefb and idle touch binaries

Verified 2026-10-24 4:48 PM PT. `scripts/integrate.sh --qemu` rebuilt the
`aarch64-unknown-redox` RAM initfs with the Podman cookbook toolchain, rebuilt
the EFI payload, checked `phone-boot/SHA256SUMS`, and booted QEMU with no drive.
The QEMU log is [`K8B-QEMU.txt`](K8B-QEMU.txt). No phone was touched or remote
pushed.

## Source and patch commits

- Kernel `kernel-k8b`, `op6/integration-2`: `f922469d` (unchanged K8 head).
  `patches/kernel-irq-width/` was absent and was skipped.
- Base `base-k8b`, `op6/integration-2`: `acf31abd`, `ae6fbfae`, `22e5daa5`
  (simplefb); `da68cd46` (K9 touch workers, with K7's existing `mmio-replay`
  retained and `Cargo.lock` regenerated); `d4143d31` (vesad); `d576eb47`
  (idle touch binaries); `18400fde` (inputd and graphics startup).
- Bootloader `bootloader-k8b`, `op6/integration-2`: `41317dc` auto-selects the
  preferred GOP mode for RAM boot, so boot does not wait for an Enter key.
- Rebased source series: `patches/integration-2/{base,bootloader}/`. The ordered
  base series incorporates `patches/base-simplefb/` and `patches/base-touch/`.
  The original K9 patch cannot apply verbatim after K7 because it adds a second
  `mmio-replay` and edits an older lockfile.

## Verification

- Host: `cargo test -p simplefb -p geni-i2c-core -p rmi4-touch-core --offline`
  passed: 17 simplefb, 4 GENI I2C, and 5 RMI4 integration tests. Full output:
  ignored `out/integration/host-tests.log`.
- Target: `geni-i2cd`, `rmi4d`, `inputd`, and `vesad` compiled for
  `aarch64-unknown-redox` and were packaged in the RAM initfs. No startup
  service exists for either touch worker. Their physical I2C/touch behavior
  remains unverified.
- QEMU: `virt,acpi=off,gic-version=3`, `-cpu max -smp 1`, 2 GiB, EDK2,
  `-device ramfb`, and a DT overlay describing the GOP framebuffer at
  `0xBC430000` as 800×600, `a8r8g8b8`. PXE/TFTP loads the EFI payload;
  there is no block device. The log records `REDOX_RAMBOOT=1`,
  `spmid: no SPMI arbiter in device tree; skipping`, `ram:~$`, and
  `vesad: simplefb 800x600 ... from device tree`. There is no vesad panic or
  readiness failure in the final log. The 45-second timeout stops the idle
  rescue shell.

## Refreshed payload

- `$HOME/artifacts/handoff/oneplus-redox-20261024/phone-boot/redox-ramboot.efi`
- Size: 11,999,744 bytes
- SHA-256: `0a531378ecb5358c89e526a274985140e1427438073a178a0b74741e59525214`
- `phone-boot/SHA256SUMS`: full `sha256sum -c` passed.

The EFI build can change bytes on a repeat build of the same commits; use the
hash of the exact payload paired with this log. Keep the last verified payload
if a rebuild fails. The QEMU DT fixture is a QEMU ramfb address, not the
OnePlus 6 splash framebuffer. Phone display, ABL, touch, physical buttons,
and IRQ-width integration remain open hardware/source gates.
