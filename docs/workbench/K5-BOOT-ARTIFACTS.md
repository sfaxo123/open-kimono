# K5 — OnePlus 6 boot artifacts, prepared without the phone

**Status (2026-10-24 PT):** Mainline U-Boot, the enchilada DTB, an Android boot-image v0 candidate, the EFI diagnostic, the K3 RAM Redox payload, and a GPT/FAT ESP image are built under `out/phone/`. QEMU `virt` booted the diagnostic through U-Boot and booted the Redox payload to its RAM rescue prompt through U-Boot. **ABL acceptance, phone display, USB fastboot handoff, and Redox operation on SDM845 are unverified.** No phone was accessed or flashed. Nothing was pushed.

## Source choices and limits

- U-Boot: `~/Developer/forks/u-boot` at `8649a54d301a4d0ac526324c7a7ad2574939ff27`; `qcom_defconfig` plus `qcom-phone.config`. The phone fragment enables an on-screen boot menu, USB ACM console option, and `fastboot oem run`. I disabled only the **host build tool** `CONFIG_TOOLS_MKEFICAPSULE`, because the host lacks GnuTLS headers; U-Boot's EFI loader remains enabled. The source tree is clean. The GPL-covered U-Boot binary is paired with this pinned source revision; see `docs/PROVENANCE.md`.
- DTB: `~/Developer/forks/linux` at `5b0764a53235d3c77e1ed674e836e1b3fe1a4b1f`, `arch/arm64/boot/dts/qcom/sdm845-oneplus-enchilada.dts` plus kernel includes, compiled with `cpp` and `dtc -@`. The sparse Linux checkout had a broken symlink for `include/dt-bindings/input/linux-event-codes.h`; its target was read with `git show` from the **same commit** into an ignored build include. `dtc` completed with existing duplicate-unit-address warnings. The resulting DTB root reports `OnePlus 6` and `oneplus,enchilada`.
- Packaging: pmaports `device/community/device-oneplus-enchilada/deviceinfo` at `e6ff10dd9061b415dcf4eed7f873bf6e5e333db6` says fastboot, append DTB, page size 4096, base `0`, kernel offset `0x8000`, ramdisk `0x01000000`, second `0x00f00000`, tags `0x100`; it does not specify a header version. U-Boot's Qualcomm board guide prescribes a gzipped `u-boot-nodtb.bin` with an appended DTB. I chose **header v0** for this candidate. That choice must be compared against a known-good OnePlus boot image before trying it; an offline package cannot prove what this ABL accepts.
- Ubuntu's `mkbootimg` `1:34.0.4-1build3` was extracted into `out/phone/`, without installing it. Its package omits the module for an unconditional optional GKI-signing import. A build-only copy removes that import; v0 packaging does not call the signing code. `unpack_bootimg` independently reports `ANDROID!`, header 0, page size 4096, kernel load address `0x8000`, no ramdisk, and the extracted payload compares byte for byte with `u-boot-nodtb.bin.gz-dtb`.
- The K3 `op6/ramboot` base and bootloader branches were archived into ignored `out/phone/` and built there. Their commits are `fd6e5e6339a44390ac85750bb1512b3b3b6cd636` and `19dd2e5008bf8dfd01769c767f17ff115c45c124`. The embedded kernel came from the DT-patched Redox kernel at `8186c37915f1fa429a1fe6258598393ac27aebbf`. The EFI diagnostic was built from this repository's `tools/efi-diag`.

## Artifacts

All paths are relative to the repository root. `out/phone/` is gitignored; copy the listed files needed tonight to the operator machine and verify their hashes there.
`out/phone/SHA256SUMS` covers all nine table entries and passed `sha256sum -c SHA256SUMS` from `out/phone/`.

| File | Purpose | SHA-256 |
| --- | --- | --- |
| `out/phone/u-boot-enchilada-v0.img` | Candidate for **ABL `fastboot boot`**, never flash | `2f86f373c21d3eb083624844db3ed002d674edea52eadf86ce3a08c34ab89f72` |
| `out/phone/u-boot-nodtb.bin` | Raw phone U-Boot build before gzip and DTB append | `df7ff67c3ea98abb1b59a5ba9e9d4c99c3e7bbf03d48864f1c8b4851e73c2d58` |
| `out/phone/u-boot-nodtb.bin.gz-dtb` | Android-image kernel payload, gzip plus enchilada DTB | `c50fec59e1c6bedaa5077b324615bf3eb001315144501da850d007b0a7619662` |
| `out/phone/sdm845-oneplus-enchilada.dtb` | Compiled phone DTB | `48518e1de2b2024aa078f4ba2ce2dfc1e3f9e8923c4a212b320413dbb1259383` |
| `out/phone/efi-diag.efi` | First EFI payload, also `EFI/BOOT/BOOTAA64.EFI` on ESP | `1c7f666200771ed480c9278e32525baaa11dee2ff1742eea56b2992d0ebf4679` |
| `out/phone/redox-ramboot.efi` | Second, K3 kernel plus initfs payload, also `REDOX/RAMBOOT.EFI` on ESP | `31888122c058db000bcd4bb079a414b944436b5c957ab9190b87bd9742e68afd` |
| `out/phone/op6-boot-esp.img` | 64 MiB GPT image, one FAT32 ESP starting at sector 2048 | `6ce8d93c5b5c0888a9125178ecfb21b5d9c7e4990b00f430a28f0de55d5dd1ba` |
| `out/phone/u-boot-qemu.bin` | QEMU `qemu_arm64_defconfig` U-Boot, for smoke tests only | `390982af5b85ab02bf6ff4abe1da9d3e40093a63f2d06d5942780e614b9988ff` |
| `out/phone/op6-redox-qemu.img` | Test-only ESP copy with K3 as default boot target | `2dd08665c632829cf911d4a1084b9e5b54c1790a356a80db8686e9587074356a` |

The ESP's two EFI files were read back with `mcopy` and compared with the standalone binaries. `sfdisk --dump` reports a GPT EFI System partition at sector 2048 with 126976 sectors. The diagnostic is the default boot target; the Redox EFI file requires an explicit load.

## QEMU smoke evidence

QEMU has no SDM845 model. Both runs used the `qemu_arm64_defconfig` binary, `-machine virt,acpi=off,gic-version=3 -cpu max -smp 1`, one virtual FAT boot drive, and no Redox rootfs drive. The Redox run used a **test copy** of the ESP (`out/phone/op6-redox-qemu.img`) with the K3 EFI file temporarily placed at the fallback boot path. The deliverable ESP still defaults to the diagnostic.

```sh
timeout --signal=TERM 25 qemu-system-aarch64 -machine virt,acpi=off,gic-version=3 -cpu max -smp 1 -m 2048 -bios out/phone/u-boot-qemu.bin -drive if=none,id=esp,file=out/phone/op6-boot-esp.img,format=raw -device virtio-blk-device,drive=esp -display none -serial stdio -monitor none -net none
timeout --signal=TERM 55 qemu-system-aarch64 -machine virt,acpi=off,gic-version=3 -cpu max -smp 1 -m 2048 -bios out/phone/u-boot-qemu.bin -drive if=none,id=esp,file=out/phone/op6-redox-qemu.img,format=raw -device virtio-blk-device,drive=esp -display none -serial stdio -monitor none -net none
```

- [`K5-QEMU-DIAG.txt`](./K5-QEMU-DIAG.txt): U-Boot boot manager starts the EFI app; it prints `OnePlus 6 EFI diagnostic`, `CurrentEL: EL1`, an EFI memory map, and a DTB entry (`linux,dummy-virt`). GOP is unavailable in this headless QEMU configuration. The 25-second timeout stopped the app during its deliberate 60-second wait.
- [`K5-QEMU-REDOX.txt`](./K5-QEMU-REDOX.txt): U-Boot starts `Redox OS Bootloader 1.0.0`; the kernel receives `REDOX_RAMBOOT=1`; init prints `RAM boot shell on /scheme/debug; no rootfs mounted` and `ram:~$`. The 55-second timeout stopped QEMU. Scripted input did not complete a shell command in this U-Boot QEMU path. K3's earlier **direct UEFI** run recorded a shell command in `K3-RAMBOOT.md`; K5 establishes boot-to-prompt through U-Boot.

## Tonight's non-flashing procedure

The phone owner must first confirm ownership, USB/charging state, and that the planned oracle snapshot and partition backups have completed under their own runbook. These commands are **for that later session**, not executed by K5. Keep postmarketOS recoverable. Do not flash, erase DTBO, change slots, or repartition.

1. On the host, check `cd out/phone && sha256sum -c SHA256SUMS`. Read the existing boot partition **backup** with `unpack_bootimg` and compare its actual header version, page size, load offsets, and appended-DTB convention. If they differ from this candidate, stop and rebuild the Android wrapper; do not infer ABL compatibility from QEMU.
2. With the phone already in its stock fastboot mode and the correct device selected, run `fastboot boot out/phone/u-boot-enchilada-v0.img`. Do not substitute `fastboot flash`. If ABL rejects it, or U-Boot does not show a menu/console or a retrievable log, stop and return to the existing pmOS boot path. U-Boot's upstream guide discusses flashing and erasing DTBO for other boards; those steps are **outside this procedure**.
3. In the U-Boot phone menu, enable the USB serial console gadget and enter U-Boot fastboot mode. On the host, confirm this is U-Boot's connection before sending a payload: `fastboot getvar version-bootloader`. Use `fastboot oem run:'printenv fastboot_addr_r fdt_addr_r'` to read the dynamically allocated RAM and DT addresses. If either is unset, inaccessible, or overlaps reserved memory according to `bdinfo`/the live memory map, stop. The Qualcomm phone environment sets `fastboot=fastboot -l $fastboot_addr_r usb 0`; `fastboot stage` places bytes in that RAM buffer, without writing storage.
4. Stage and run the diagnostic first:

   ```sh
   fastboot stage out/phone/efi-diag.efi
   fastboot oem run:'bootefi ${fastboot_addr_r} ${fdt_addr_r}'
   ```

   Photograph the screen or capture the USB serial report. Check CurrentEL, memory map, FDT model/compatible, and GOP/framebuffer report. **Stop after the diagnostic** unless the operator has reviewed those results and explicitly cleared the next Redox attempt. `bootefi` should return when the diagnostic exits after a key or 60 seconds; confirm the U-Boot fastboot connection remains healthy.
5. Only after that gate, stage the K3 RAM payload and run it at the same validated dynamic addresses:

   ```sh
   fastboot stage out/phone/redox-ramboot.efi
   fastboot oem run:'bootefi ${fastboot_addr_r} ${fdt_addr_r}'
   ```

   This is a RAM-only Redox rescue experiment; it does not mount a phone root filesystem. A U-Boot console alternative, **only if USB mass storage is actually discovered**, is `usb start`, `fatls usb 0:1 /`, `fatload usb 0:1 ${loadaddr} EFI/BOOT/BOOTAA64.EFI`, `bootefi ${loadaddr} ${fdt_addr_r}` for the diagnostic, then `fatload usb 0:1 ${loadaddr} REDOX/RAMBOOT.EFI`, `bootefi ${loadaddr} ${fdt_addr_r}` after the gate. Check `bdinfo` and `printenv loadaddr fdt_addr_r` first. The ESP image is a prepared FAT volume, not proof the phone supports this USB host path.

## Open questions for tonight

1. What boot-header version and load addresses does the phone's **known-good** boot image actually use, and will its ABL accept this v0 gzip-plus-DTB candidate via `fastboot boot`?
2. Does this U-Boot build start on enchilada with the live ABL-modified DTB, present a visible menu, and expose a working USB serial or U-Boot fastboot gadget? The QEMU build exercises only virtual hardware.
3. Does U-Boot preserve a valid FDT and GOP/framebuffer for the EFI diagnostic, and where does its runtime RAM buffer land relative to the phone's reserved-memory regions?
4. After the diagnostic gate, can the K3 EFI payload start the Redox kernel on SDM845? K1 PDC/TLMM and K2 GENI UART support are separate wave tasks; a QEMU rescue prompt does not prove a phone console.

If any gate fails, capture the exact screen/serial/fastboot output and return to postmarketOS through the existing non-flashing boot path. No K5 job remains running.
