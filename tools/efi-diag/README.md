# OnePlus 6 EFI diagnostic

A small MIT-licensed AArch64 UEFI application. It reports CurrentEL, an EFI memory-map summary, the FDT configuration-table entry (if present), and GOP mode to both the EFI console and the GOP display. It reads the root `model` and `compatible` properties from the FDT. It waits for a key or 60 seconds, then returns to the firmware. GOP `BltOnly` modes are drawn with `Blt`, and their framebuffer base is reported as `None` because no CPU-accessible framebuffer is exposed.

## Build on the 3090

```sh
rustup target add aarch64-unknown-uefi
cargo build --manifest-path tools/efi-diag/Cargo.toml --target aarch64-unknown-uefi --release
# EFI binary: tools/efi-diag/target/aarch64-unknown-uefi/release/op6-efi-diag.efi
```

`uefi = 0.41.0` (crates.io, matching the local `~/Developer/forks/op6-refs/uefi-rs` reference clone). `font8x8` supplies the bitmap glyphs. `wcslen` is provided locally because LLVM emits that symbol for this target while the UEFI target has no C runtime.

## QEMU smoke boot

```sh
mkdir -p tools/efi-diag/qemu-fat/EFI/BOOT
cp tools/efi-diag/target/aarch64-unknown-uefi/release/op6-efi-diag.efi tools/efi-diag/qemu-fat/EFI/BOOT/BOOTAA64.EFI
timeout 12s qemu-system-aarch64 -machine virt -cpu cortex-a72 -m 1024 \
  -bios /usr/share/qemu-efi-aarch64/QEMU_EFI.fd \
  -drive if=none,id=boot,format=raw,file=fat:rw:tools/efi-diag/qemu-fat \
  -device virtio-blk-device,drive=boot -device virtio-gpu-pci \
  -display none -serial stdio -monitor none -no-reboot
```

The captured serial output is in `QEMU-PROOF.txt`. The 12-second test timeout deliberately stops QEMU before the app's 60-second wait expires. This firmware does not supply an FDT configuration-table entry, so FDT parsing needs a phone-side check tonight.

## Load on the phone

Keep postmarketOS intact. Copy the EFI binary to a FAT partition or U-Boot-readable removable/temporary storage; do not flash partitions. For U-Boot, load it into free RAM and call `bootefi` with the device-tree address U-Boot is using, for example:

```text
fatload usb 0:1 ${loadaddr} EFI/BOOT/BOOTAA64.EFI
bootefi ${loadaddr} ${fdt_addr_r}
```

Adjust device, partition, and addresses to the live U-Boot environment. With edk2, place it at `EFI/BOOT/BOOTAA64.EFI` on a FAT volume and select that volume from the boot manager. Confirm the photo/screenshot and serial report before attempting the Redox bootloader. If the firmware has no GOP or DTB entry, the report says so.
