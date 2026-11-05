# T2 findings: booting non-Linux (Redox) on OnePlus 6 (enchilada, SDM845)
Researched 2026-10-24. Web search tools only; several sources (postmarketOS wiki, pmaports MR) were blocked by Anubis/redirect, so those items are UNKNOWN. Source dates given where seen.

## 1. Stock boot chain
- Chain: PBL -> XBL (UEFI firmware, has fastboot app, inits display) -> ABL -> LinuxLoader.efi -> loads kernel from boot partition. Retail devices sign XBL and ABL, so they cannot be replaced. (Renegade overview: https://renegade-doc.readthedocs.io/en/latest/edk2/Overview.html, undated; FOSDEM24 Connolly slides https://archive.fosdem.org/2024/events/attachments/fosdem-2024-1716-u-boot-for-modern-qualcomm-phones/slides/22104/fosdem24_aDevbas.pdf, Feb 2024: "We can't replace the Android bootloader... But we can chainload", build with CONFIG_LINUX_KERNEL_IMAGE_HEADER=y).
- ABL expects an Android boot image whose "kernel" looks like a Linux Image (arm64 Image header) with a DTB appended; Renegade does exactly this to fool ABL. U-Boot docs: "Android bootloader expect gzipped kernel with appended dtb, so let's mimic linux" (https://docs.u-boot.org/en/v2026.04/_sources/board/qualcomm/board.rst.txt). Example: `mkbootimg --kernel u-boot-nodtb.bin.gz-dtb --output boot.img --pagesize 4096 --base 0x80000000` (header v2 variants with --dtb also shown). ABL also populates memory layout / KASLR offset for the payload (older U-Boot qcom doc, via search snippet).
- Flash step in U-Boot docs: `fastboot flash boot boot.img` then `fastboot erase dtbo` (dtbo conflicts with appended DTB).
- `fastboot boot <img>` without flashing: Renegade's porting guide uses `fastboot boot boot-CodeName.img` as the test step (https://renegade-doc.readthedocs.io/en/latest/edk2/port.html). On a LOCKED stock device fastboot boot of unsigned images fails with "Failed to load/authenticate boot image: Load Error" (https://forum.renegade-project.tech/t/did-i-brick-my-oneplus-6/2143). Bootloader here is unlocked, so it should work; not verified by me on this unit.
- A/B: OnePlus 6 is A/B. Lineage-style command flashes both: `fastboot flash --slot=all boot boot.img` (search snippet, UBports/Lineage threads). Header version / page size accepted by this ABL: UNKNOWN beyond the v2 + 4096 pagesize examples above.

## 2. Mainline U-Boot on SDM845
- Maintainer: Caleb Connolly (Linaro). Upstream arch/arm/mach-snapdragon covers MSM8916, MSM8996, SDM845, SM6115, QCM2290, SM8250, SC7280 (FOSDEM24). One generic target `qcom_defconfig` + `qcom-phone.config`, reads memory map from DT.
- Upstream docs (v2026.04) name only Pixel 3/3 XL (blueline/crosshatch) and Samsung S9 (starqltechn) as phones. OnePlus 6 NOT listed there. UNKNOWN whether upstream defconfig works on enchilada as-is.
- BUT: postmarketOS "SDM845 generic device support" MR (https://gitlab.postmarketos.org/postmarketOS/pmaports/-/merge_requests/4599, page blocked, only search-snippet) says U-Boot is flashed to the boot partition, provides EFI and auto-selects DTB, and the author tested a OnePlus 6. Wiki: https://wiki.postmarketos.org/wiki/Qualcomm_SDM845_(generic-sdm845) (blocked, UNVERIFIED contents). Fork: https://gitlab.com/sdm845-mainline/u-boot (archived, points to git.codelinaro.org/linaro/qcomlt/u-boot; release sdm845-phones-v0.1.0). So a working U-Boot for enchilada very likely exists (pmOS ships it) - details UNKNOWN.
- Features as of FOSDEM24 (stale): USB for SDM845 "on the lists", UFS "in progress". Current state (2026) UNKNOWN. A 2026-05 patch series adds an SPL path for SDM845 booted via EDL with DFU (https://lists.denx.de/pipermail/u-boot/2026-May/617922.html, v7, not confirmed merged); not needed here.
- UEFI: U-Boot provides UEFI services (SystemReady-passing per slides), can run arbitrary .efi payloads (bootefi / extlinux). Framebuffer handoff: simple-framebuffer from what XBL/ABL left; per-device details UNKNOWN. Fastboot/USB gadget in U-Boot: depends on USB support state, UNKNOWN for enchilada.

## 3. edk2 UEFI (Renegade / edk2-msm)
- Project: https://github.com/edk2-porting/edk2-sdm845 (a.k.a. edk2-msm); README lists OnePlus 6 (enchilada) as supported. Goal "boot any fuOS at EL1" (Renegade overview). Packaged as a fake-kernel boot image with appended DTB, as in section 1. Warns never to port to Sony/Google devices (wipes UFS).
- Framebuffer: XBL already inits display; port uses SimpleFbDxe (GOP framebuffer).
- Firmware table: Windows needs ACPI (unsolved compat issues, BSOD risk); others use DT. Many XBL-derived drivers (buttons, USB, PMIC) do NOT work; UFS works. Doc calls the port "badly broken, though usable as a bootloader". Only Windows is documented to boot. Arbitrary EFI app: UEFI shell/EFI apps should load (GOP + UFS ok) - not explicitly confirmed; UNKNOWN. Releases: https://github.com/edk2-porting/edk2-sdm845/releases (not inspected).
- Caveat: no working USB/keyboard in UEFI, so no interactive input; boot is from storage only. Redox ships a UEFI bootloader, so edk2 -> Redox bootloader is plausible but unreported.

## 4. Exception level
- Renegade says EL1 target. For SDM845 a Qualcomm hypervisor holds EL2: UNVERIFIED for this chip specifically; strong precedent from MSM8916 (proprietary hyp blocks EL2, qhypstub replaces it: https://github.com/msm8916-mainline/qhypstub) and an HN comment about Gunyah. Treat as EL1 only.
- Consequences (inferred, not sourced): no EL2/KVM/own hypervisor; GICv3 access at EL1 normal (distributor/redistributor MMIO); PSCI via SMC (conduit smc, firmware at EL3/hyp) for CPU on/off and power-off; do not touch EL2 system registers. Redox kernel must not assume EL2 entry (needs to run from EL1 start).

## 5. Debug output
- Hardware UART: 1.8 V, 115200 8N1, Rx/Tx on board test points; diagram on postmarketOS wiki "Serial debugging/Cable schematics" (https://wiki.postmarketos.org/wiki/Serial_debugging:Cable_schematics) referenced from the 2021 OnePlus 6 DT patch (https://patches.linaro.org/patch/483532, fetch failed; via search snippet). Needs 1.8 V level shifter/adapter, opening the phone, soldering. Bootloader output also on it. Not on headphone jack: no evidence found (OnePlus 3T muxes UART on USB, different phone).
- Easiest console: framebuffer (XBL/ABL leaves display on; both U-Boot and edk2 give simple fb). Early UART address/driver for geni UART on SDM845: UNKNOWN here (read mainline sdm845.dtsi / enchilada DTS `chosen stdout-path`).
- USB gadget serial: needs USB bring-up in your payload or U-Boot; UNKNOWN status. ramoops/pstore: mainline DTS reserved-memory ramoops on OnePlus 6 UNVERIFIED; pstore survives warm reboot only if region untouched; reading it back requires booting pmOS afterwards. Practical: log to RAM region + framebuffer, read via pstore-like region from U-Boot/Linux. All speculative.

## 6. Recovery / brick risk
- EDL 9008 + MSM Download Tool restores stock OxygenOS (Windows only; Qualcomm 9008 driver; use USB 2.0 port). Enter EDL: power off, hold Vol Up + Vol Down, plug USB. Source (updated 2022-07-16): https://www.thecustomdroid.com/oneplus-6-6t-unbrick-guide/ ; also https://gist.github.com/ahnafnafee/4058bc63e048b4c139aee6d4b229bc80 . The tool also relocks the bootloader and wipes the phone. Tool files come from third parties, verify hashes.
- Documented self-inflicted brick: deleting all partitions with parted on sda while installing Windows; only userdata (partition 17) should be deleted. Recovered via MSM tool. https://forum.renegade-project.tech/t/did-i-brick-my-oneplus-6/2143
- Do NOT touch: xbl/abl/modem/firmware partitions, devinfo/persist/bluetooth/dsp (calibration; persist loss is hard to restore), partition table. Only write boot_a/boot_b (and optionally erase dtbo). Back up boot_a/b, dtbo_a/b, persist, modem before first flash (`dd` from pmOS). Do not relock the bootloader with a non-stock boot image.
- Return to pmOS: re-flash the saved pmOS boot.img via fastboot (`fastboot flash --slot=all boot`), rootfs untouched.

## 7. Prior art
- Windows 10 ARM on OnePlus 6/6T via Renegade edk2 (Renegade docs above; XDA mention https://www.xda-developers.com/oneplus-6-6t-mainline-linux-kernel-support).
- FreeBSD forum thread, 2024-09-07: user with OP6 reaches UEFI via Renegade, asks for SDM845 BSD support; no replies (https://forums.freebsd.org/threads/sdm845-bsd-support.94860/latest).
- U-Boot as ABL payload on Pixel 3 / S9 / RB3 (U-Boot docs). pmOS booting via U-Boot EFI on OP6 (pmaports MR 4599).
- Redox on real hardware: Phoronix 2026-05-06 on April status (x86 focus, ACPI) https://phoronix.com/news/redox-os-april-2026 . Redox aarch64 on real hardware / DT boot: UNKNOWN (2022 0.8.0 reports QEMU only). No Redox-on-SDM845 report found.

## Recommended boot path (lowest brick risk)
1. Back up (via pmOS dd): boot_a/b, dtbo_a/b, persist, modem, devinfo; save current pmOS boot.img. Install MSM tool + stock ROM + 9008 driver on a Windows box BEFORE starting.
2. Never write anything but boot_<slot> (use the inactive slot if you can switch: `fastboot set_active`), never repartition.
3. Test pipeline with `fastboot boot` only (no flash): U-Boot (sdm845-mainline/pmOS build for enchilada) packaged as fake-kernel + appended DTB boot image. Confirm it shows on the screen.
4. Reuse pmOS's own enchilada U-Boot boot.img as the known-good chainloader if its build is obtainable; verify it boots Linux first.
5. From U-Boot, load Redox's aarch64 UEFI bootloader as an EFI app (`bootefi`) from a USB/ext partition or ramdisk; start with a trivial Rust EFI hello-world that draws on the GOP/simplefb framebuffer.
6. Do the Redox kernel work against EL1 start, GICv3, PSCI-over-SMC, DT from U-Boot (don't expect ACPI), serial = framebuffer first.
7. Add UART via test points (1.8 V adapter) only when the framebuffer log is insufficient.
8. Fallback path: Renegade edk2 `fastboot boot` image if U-Boot lacks something; expect no USB/input there.
9. Only flash to boot_<slot> after repeated successful `fastboot boot` runs; keep the other slot as pmOS.
10. If stuck: hold Vol Up+Down (fastboot/EDL) -> reflash saved boot.img, or MSM tool as last resort (wipes the phone).
