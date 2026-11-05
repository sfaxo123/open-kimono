# K3: QEMU boot without a drive

The aarch64 UEFI bootloader's `ramboot` feature embeds a kernel and initfs in
one EFI program. UEFI PXE/TFTP supplies that program to QEMU; the Redox kernel
receives `REDOX_RAMBOOT=1`, so `init` starts the serial rescue shell without
mounting RedoxFS or switching to `/usr`. The OS does not start a network or
storage driver. The QEMU network device is used by firmware to fetch the EFI
program and is idle after handoff.

## Build

The Redox build checkout is
`$HOME/Developer/forks/redox/redox`. The patched DT kernel artifact is
`$HOME/Developer/forks/redox/kernel/build/aarch64/kernel`. Neither
checkout is modified by these commands. Build outputs stay in the K3
worktrees. The Redox Podman image, cross toolchain, and `cookbook_redoxer`
were prepared by W0.

```sh
BASE_K3=$HOME/Developer/forks/redox/base-k3
BOOT_K3=$HOME/Developer/forks/redox/bootloader-k3
REDOX_BUILD=$HOME/Developer/forks/redox/redox

cd "$BASE_K3"
podman run --rm --workdir /mnt/base-k3 --pids-limit=1024 \
  --volume "$REDOX_BUILD":/mnt/redox \
  --volume "$REDOX_BUILD/build/podman":/root \
  --volume "$BASE_K3":/mnt/base-k3 \
  --env PATH=/root/.cargo/bin:/mnt/redox/prefix/aarch64-unknown-redox/sysroot/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
  --env TARGET=aarch64-unknown-redox \
  --env COOKBOOK_TOOLCHAIN=/mnt/redox/prefix/aarch64-unknown-redox/sysroot \
  --env COOKBOOK_SYSROOT=/mnt/redox/prefix/aarch64-unknown-redox/sysroot \
  redox-base make ramboot-initfs TARGET=aarch64-unknown-redox \
  BUILD_DIR=/mnt/base-k3/build/aarch64 LINKER=aarch64-unknown-redox-ld \
  CARGO=/mnt/redox/target/release/cookbook_redoxer

cd "$BOOT_K3"
mkdir -p build/tftp
RAMBOOT_KERNEL=$HOME/Developer/forks/redox/kernel/build/aarch64/kernel \
RAMBOOT_INITFS="$BASE_K3/build/aarch64/ramboot-initfs.img" \
  make -C build -f ../Makefile TARGET=aarch64-unknown-uefi \
  "$BOOT_K3/build/bootloader-ramboot.efi"
cp build/bootloader-ramboot.efi build/tftp/bootaa64.efi
```

If either input path changes to an older timestamp, remove the ignored
`build/bootloader-ramboot.efi` before rebuilding so Make cannot reuse the
previous embedded payload.

## QEMU acceptance

Run this from `bootloader-k3` in a PTY, then at `ram:~$` type
`echo K3_FINAL_RAM_SHELL_OK`. No QEMU drive, block backend, or CD-ROM is
attached.

```sh
timeout --signal=TERM 70 script -q -f -c \
  'qemu-system-aarch64 -machine virt,acpi=off,gic-version=3 -cpu max -smp 1 -m 2048 -bios /usr/share/qemu-efi-aarch64/QEMU_EFI.fd -boot order=n -nic user,model=virtio-net-pci,tftp=$HOME/Developer/forks/redox/bootloader-k3/build/tftp,bootfile=bootaa64.efi -display none -serial stdio -monitor none' \
  docs/workbench/k3-qemu-final.log
```

`k3-qemu-final.log` records PXE download of the 8,554,496-byte EFI program,
`REDOX_RAMBOOT=1`, `init: RAM boot shell ... no rootfs mounted`, the `ram:~$`
prompt, `K3_FINAL_RAM_SHELL_OK`, and `ls` showing `lib` and `bin`. The normal
bootloader configuration also passed `cargo check`. Other evidence:

- `base-k3/docs/workbench/k3-base-check.log`: Redox target check of `init` and `ramsh`.
- `base-k3/docs/workbench/k3-minimal-initfs-build.log`: dedicated initfs build.
- `bootloader-k3/docs/workbench/k3-bootloader-minimal-build.log`: embedded EFI build.
- `bootloader-k3/docs/workbench/k3-normal-bootloader-check.log`: normal-path check.

The initfs is 7,016,712 bytes. The shell provides `help`, `echo`, `pwd`,
`ls`, `cat`, and `exit`; it is a small rescue shell rather than Ion. QEMU
proves the DT-only, no-drive path. Booting the same EFI payload through
OnePlus 6 fastboot/U-Boot and panel or UART access remains untested.
