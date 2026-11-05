# W0B — aarch64 device tree and RAM-backed QEMU boot

**Result (2026-10-24 PT):** QEMU `virt,acpi=off,gic-version=3` now boots Redox through device tree, copies the live RedoxFS into RAM, reaches a serial login, and accepts `user` into Ion. `echo W0B_SHELL_OK` ran in that shell. The raspi3b target also reached serial login in live mode. This is **RAM-backed after loading**, not a strict no-disk boot: UEFI and the bootloader still read an image from a QEMU drive. A run with no drive enters the UEFI shell and never loads Redox. The original non-live images still fail to mount their disks after init starts. No package was installed with sudo, no branch was pushed, and no VM was left running.

## Source and commit

- Kernel checkout: `$HOME/Developer/forks/redox/kernel`, branch `op6/dt-acpi-off`, commit `8186c379` (`Fix inherited devicetree interrupt domains on aarch64`). Clean working tree at handoff. The commit message ends `Co-Authored-By: Codex <noreply@openai.com>`.
- Redox build checkout: `$HOME/Developer/forks/redox/redox`; build images are untracked/ignored there. The baseline `.config` is described in `W0-BUILD.md`.
- Patch: `src/dtb/irqchip.rs` walks inherited `interrupt-parent` domains and parses `interrupts` using the referenced controller's `#interrupt-cells`; `interrupts-extended` uses each entry's own phandle and width. The aarch64 timer and PL011 use the specifier's matching controller. Root interrupt controller selection can fall back to a root chip when no root `interrupt-parent` is supplied. Malformed/missing domains no longer trigger the fdt library's panic in this path.

## Cause and reproduction

The pinned `repnop/fdt` parser resolves a legacy `interrupts` node's `interrupt-parent` from its immediate DT parent when no explicit property is on the device. On QEMU virt, the root declares `interrupt-parent = <0x8002>` but bus/device nodes do not; their immediate parent has no `#interrupt-cells`. `node.property::<Interrupts>()` therefore panicked with `MissingRequiredProperty("interrupt-cells")` in IRQ-chip init. It also affected timer and PL011 IRQ decoding. The fixed parser follows the inherited interrupt domain and reads the referenced controller's cell count. QEMU's DTB was saved as `w0b-virt.dtb` (from `dumpdtb`).

From `$HOME/Developer/forks/redox/redox`:

```sh
# The requested cortex-a75 was attempted, but QEMU 8.2.2 has no such CPU model:
qemu-system-aarch64 -machine virt,acpi=off,gic-version=3 -cpu cortex-a75 -smp 1 -m 2048 -bios /usr/share/qemu-efi-aarch64/QEMU_EFI.fd -drive file=build/aarch64/minimal/harddrive.img,format=raw,if=virtio -display none -serial stdio -monitor none -net none
# qemu-system-aarch64: unable to find CPU model 'cortex-a75'

timeout --signal=TERM 45 qemu-system-aarch64 -machine virt,acpi=off,gic-version=3 -cpu max -smp 1 -m 2048 -bios /usr/share/qemu-efi-aarch64/QEMU_EFI.fd -drive file=build/aarch64/minimal/harddrive.img,format=raw,if=virtio -display none -serial stdio -monitor none -net none > $HOME/artifacts/handoff/oneplus-redox-20261024/w0b-repro-max.log 2>&1
```

`w0b-repro-max.log` records `IRQCHIP INIT` followed by the panic. The failed `cortex-a75` command's stderr is `w0b-repro.log`. `-cpu max` was used for all successful virt tests.

## Build and boot commands

From `$HOME/Developer/forks/redox/kernel`:

```sh
cargo fetch --locked
cargo fmt --check
make check ARCH=aarch64
make ARCH=aarch64 OBJCOPY=aarch64-linux-gnu-objcopy
```

The default `aarch64-unknown-redox-objcopy` is not installed; the host's `aarch64-linux-gnu-objcopy` worked. `make check` and the release build passed (existing Rust warnings remain). Logs: `w0b-cargo-fetch.log`, `w0b-kernel-check-final.log`, and `w0b-kernel-build-final.log`. The first format check found formatting changes (`w0b-fmt-check.log`); `cargo fmt` was applied and the final format check passed. `git diff --check` passed before commit.

From `$HOME/Developer/forks/redox/redox` (the baseline image and Podman prerequisites came from `W0-BUILD.md`):

```sh
make ARCH=aarch64 CONFIG_NAME=minimal PODMAN_BUILD=1 USE_SELINUX=0 CI=1 PODMAN_OPTIONS='--rm --workdir /mnt/redox --interactive --cap-add SYS_ADMIN --device /dev/fuse --network=host --env TERM=dumb --pids-limit=1024' live
cp --reflink=auto build/aarch64/minimal/redox-live.iso build/aarch64/minimal/w0b-live.iso
mkdir -p build/aarch64/minimal/w0b-live-mount
# Keep this foreground redoxfs process running in a separate terminal/session:
build/fstools/bin/redoxfs --no-daemon build/aarch64/minimal/w0b-live.iso build/aarch64/minimal/w0b-live-mount
# In another terminal:
cp $HOME/Developer/forks/redox/kernel/build/aarch64/kernel build/aarch64/minimal/w0b-live-mount/usr/lib/boot/kernel
cp $HOME/Developer/forks/redox/kernel/build/aarch64/kernel.all build/aarch64/minimal/w0b-live-mount/usr/lib/boot/kernel.all
sync
fusermount3 -u build/aarch64/minimal/w0b-live-mount
# In a PTY so the serial login can receive keyboard input:
timeout --signal=TERM 55 script -q -f -c 'qemu-system-aarch64 -machine virt,acpi=off,gic-version=3 -cpu max -smp 1 -m 2048 -bios /usr/share/qemu-efi-aarch64/QEMU_EFI.fd -drive file=build/aarch64/minimal/w0b-live.iso,format=raw,if=virtio -display none -serial stdio -monitor none -net none' $HOME/artifacts/handoff/oneplus-redox-20261024/w0b-shell-final.log
# At redox login: type user and Enter. At Ion: echo W0B_SHELL_OK
```

`w0b-shell-final.log` shows `Switching to live disk`, timer virq 30, serial virq 33, `init: switchroot to /scheme/initfs`, `init: switchroot to /usr /etc`, `redox login:`, `Welcome to Redox OS!`, the `user:~$` prompt, and `W0B_SHELL_OK`. The boot still logs a virtio-blkd userspace fault and missing network/audio/display devices; none prevented the RAM-backed shell. Image SHA-256: `7805277123cee6d084ecec064c097913b9907112c3d28cd2f7c270ae226c3a7f` (`build/aarch64/minimal/w0b-live.iso`).

The non-live image with an earlier patched kernel reached init but could not mount RedoxFS from virtio; see `w0b-patched-dt.log`. A strict no-drive command landed in `UEFI Interactive Shell` with `No mapping found`; see `w0b-no-disk.log`:

```sh
timeout --signal=TERM 8 qemu-system-aarch64 -machine virt,acpi=off,gic-version=3 -cpu max -smp 1 -m 2048 -bios /usr/share/qemu-efi-aarch64/QEMU_EFI.fd -display none -serial stdio -monitor none -net none > $HOME/artifacts/handoff/oneplus-redox-20261024/w0b-no-disk.log 2>&1
```

## Raspi3B+ baseline

The documented `BOARD=raspi3bp` config is `config/aarch64/raspi3bp/minimal.toml`. Commands from the Redox build checkout:

```sh
make ARCH=aarch64 BOARD=raspi3bp CONFIG_NAME=minimal BUILD=build/aarch64/raspi3bp-minimal PODMAN_BUILD=1 USE_SELINUX=0 CI=1 PODMAN_OPTIONS='--rm --workdir /mnt/redox --interactive --cap-add SYS_ADMIN --device /dev/fuse --network=host --env TERM=dumb --pids-limit=1024' all
make ARCH=aarch64 BOARD=raspi3bp CONFIG_NAME=minimal BUILD=build/aarch64/raspi3bp-minimal build/aarch64/raspi3bp-minimal/raspi3bp_uboot.rom
(sleep 9; printf '\r') | timeout --signal=TERM 55 qemu-system-aarch64 -machine raspi3b -kernel build/aarch64/raspi3bp-minimal/raspi3bp_uboot.rom -smp 4 -m 1024 -drive file=build/aarch64/raspi3bp-minimal/harddrive.img,if=sd,format=raw -display none -serial stdio -monitor none -net none > $HOME/artifacts/handoff/oneplus-redox-20261024/w0b-raspi-enter.log 2>&1
```

U-Boot loaded `bootaa64.efi`, the Redox bootloader loaded kernel and initfs, then the unpatched packaged kernel hit the **same** interrupt-cells panic (`w0b-raspi-enter.log`). A copy of that SD image with the patched kernel reached init but could not mount RedoxFS from the SD card (`w0b-raspi-patched.log`). A further run toggled `l` for live RAM copy; it reached 123/126 MiB before the 70-second timeout (`w0b-raspi-live.log`). The 110-second follow-up completed the RAM copy and reached `redox login:` over serial (`w0b-raspi-live-final.log`). It registered a virtual timer IRQ and serial IRQ without a DT panic. The build and firmware fetch logs are `w0b-raspi-build.log` and `w0b-raspi-firmware.log`. The successful live-mode command was:

```sh
(sleep 9; printf 'l'; sleep 1; printf '\r') | timeout --signal=TERM 110 qemu-system-aarch64 -machine raspi3b -kernel build/aarch64/raspi3bp-minimal/raspi3bp_uboot.rom -smp 4 -m 1024 -drive file=build/aarch64/raspi3bp-minimal/w0b-harddrive.img,if=sd,format=raw -display none -serial stdio -monitor none -net none > $HOME/artifacts/handoff/oneplus-redox-20261024/w0b-raspi-live-final.log 2>&1
```

## Remaining strict no-disk gate

Redox has a real live mode (`mk/qemu.mk` defaults `live=yes` on aarch64; `make live` builds `redox-live.iso`; the bootloader's `live` option allocates RAM and copies the RedoxFS image). It is not initfs-only. The current UEFI bootloader first discovers RedoxFS on a block device or a `redox-live.iso` file on an EFI filesystem, then loads `usr/lib/boot/kernel` and `usr/lib/boot/initfs`. `base/init.initfs.d/50_rootfs.service` mounts the UUID root and `base/init/src/main.rs` switches to `/usr`. With no boot medium, the firmware has no Redox EFI application to launch.

The minimal change for a strict diskless demonstration is a loader path that receives the EFI bootloader and kernel/initfs from firmware/network/RAM (or packages them into one EFI binary), plus an explicit initfs-only init target that skips `50_rootfs.service` and the `/usr` switchroot, and includes a serial userspace program or Ion and its dependencies in initfs. The current live image proves the DT, timer, serial, init and shell path after the image is copied to RAM; the diskless loader and init target are **not implemented** here. For OnePlus 6, a fastboot-loaded U-Boot/EFI payload is the likely boot source, but that path needs separate verification on the phone.

Next acceptance: launch QEMU with **no `-drive`, `-device` block backend, or `-cdrom`**, load the EFI/kernel/initfs payload through the new path, and capture a serial userspace marker. Stop if it modifies phone partitions, requires a host sudo installation, or changes a shared fleet resource; none is needed for QEMU work. Rollback this kernel patch with `git revert 8186c379` on this branch if a DT regression is demonstrated. No push has been made.
