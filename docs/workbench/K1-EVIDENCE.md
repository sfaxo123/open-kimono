# K1 host and QEMU evidence

Branch: `op6/irqchips`. All output paths below are in this kernel worktree.

## Host

`rustc --edition=2024 --test src/arch/aarch64/device/irqchip/qcom_irq.rs -o /tmp/k1-qcom-irq-tests && /tmp/k1-qcom-irq-tests`

`k1-host-tests.log`: four passing tests cover the SDM845 PDC ranges and GIC INTID offset, missing pins 109–114, trigger decoding, bank split, TLMM tile/type decoding, and GPIO wake mapping including unmapped GPIO 126.

`make check ARCH=aarch64` and `make ARCH=aarch64 OBJCOPY=aarch64-linux-gnu-objcopy` passed (`k1-check.log`, `k1-build.log`). Existing compiler warnings remain. `cargo fmt --check` passed.

`make check ARCH=x86_64` could not compile because this host lacks `nasm` (`k1-x86-check.log`); this is an environment gate, not an x86 pass.

## DT-only QEMU

Fixture: `k1-irqchips.dtso`. `k1-base.dtb` came from QEMU's `dumpdtb`; `dtc -@` and `fdtoverlay` produced `k1-virt-irqchips.dtb`. The fixture uses `redox,irqchip-probe-only`, so PDC and TLMM register reads/writes are skipped. Its PDC pin 100 maps to QEMU-supported SPI 32; the host test covers the actual SDM845 SPI 615 mapping for that pin. The fixture has `wakeup-parent` wired to PDC and requests read-only route probes for PDC pin 100 and TLMM GPIO 125.

The image is a copy of the existing `w0b-live.iso` with this worktree's release kernel and `kernel.all`; its SHA-256 is in `k1-live.sha256`. Boot command:

```sh
(sleep 9; printf 'l'; sleep 1; printf '\r') | timeout --signal=TERM 45 qemu-system-aarch64 -machine virt,acpi=off,gic-version=3 -cpu max -smp 1 -m 2048 -bios /usr/share/qemu-efi-aarch64/QEMU_EFI.fd -dtb docs/workbench/k1-virt-irqchips.dtb -drive file=docs/workbench/k1-live.iso,format=raw,if=virtio -display none -serial stdio -monitor none -net none > docs/workbench/k1-qemu-final.log 2>&1
```

`k1-qemu-final.log` records:

```text
PDC probe: phys=0xb220000 version=0x0 ranges=1
PDC route: pin=100 GIC INTID=64 virq=64
TLMM probe: phys=0x3400000 GPIOs=150 virqs=288..438
IRQ probe route: controller=pinctrl@3400000 pin=125 flags=2 virq=64
IRQ probe route: controller=interrupt-controller@b220000 pin=100 flags=2 virq=64
redox login:
```

QEMU was stopped by the 45-second timeout (exit 124). The log also shows the previously documented userspace `virtio-blkd` MSI-X panic; the RAM-backed image reached login despite it. There is no claim of a delivered GPIO/PDC hardware interrupt.

## Remaining hardware gates

- Read the phone's PDC version and enable-bank state before physical arming; v3.2 register layout is deliberately rejected.
- Confirm MMIO mapping attributes and TLMM tile/target behavior against the Linux oracle.
- Exercise TLMM summary delivery and PDC wake edges on phone hardware; QEMU has no SDM845 PDC/TLMM model.
