# T1: Redox OS on aarch64 (research as of 2026-10-24)

Method: web fetches of redox-os.org posts and the Redox book, plus shallow clones of
gitlab.redox-os.org kernel (HEAD a0a8ff9, 2026-10-24), base (HEAD d2ffc4c, 2026-10-22),
bootloader (HEAD cc922ec, 2026-08-25). Code claims come from reading those trees.
Web-fetch summaries were produced by a small model; load-bearing ones are marked.

## 1. Real aarch64 hardware, targets, boot
- Latest tagged release is 0.9.0 (2024-09-07 tag; announced 2024-09-09).
  https://gitlab.redox-os.org/redox-os/redox/-/tags , https://www.redox-os.org/news/release-0.9.0/
  - Release notes: "significant progress" on ARM64, "partial support for Raspberry Pi 3B+"
    (Ivan Tan booted it), Orbital session works under QEMU. No newer tag exists (checked 2026-10-24).
  - Project now points users at daily `server`/`desktop` images instead (Aug 2026 post).
- The Redox book ARM64 page (https://doc.redox-os.org/book/aarch64.html, undated) says "limited support".
  Only Raspberry Pi 3 B+ is documented (U-Boot + FAT32 EFI partition + DTBs + config.txt, serial on GPIO14/15).
  The QEMU section is marked "TODO: update outdated information".
- Boot method: UEFI. The bootloader crate has `src/os/uefi` (and `bios`). A DTB is passed to the kernel
  (bootloader `src/os/uefi/dtb.rs`). ACPI is also supported on aarch64 (kernel `src/acpi/madt/arch/aarch64.rs`;
  PSCI secondary CPU bring-up "from ACPI", commit 2026-07-27). Devicetree is the main path on SBCs.
- Real boards touched in 2026 (monthly posts):
  - July 2026: Amlogic Meson UART on an Amlogic S905W2 board; bootloader fix preserving UEFI cacheability
    (fixed hang at kernel entry on S905W2 and possibly others); FP-register corruption, PTE shareability,
    DeviceTree panic, and mmap memory-type fixes. https://www.redox-os.org/news/this-month-260731/ (2026-07-31)
  - Aug 2026: multi-core AArch64 support by lbecher, "performance still unmeasured, more testing needed";
    TLS offset fix; ARM64 packages listed. https://www.redox-os.org/news/this-month-260831/ (2026-08-31)
  - May 2026: fix for boot hang from PCI driver spawning on ARM64. https://www.redox-os.org/news/this-month-260531/
  - Mar 2026: ARM64 CPU feature detection, HW RNG, dynamic loader relocation fix. https://www.redox-os.org/news/this-month-260331/
- No aarch64 post exists besides the 2018 RSoC one on the news index (redox-os.org/news/, fetched 2026-10-24).
  ARM progress appears only as bullets in the monthly posts.

## 2. Kernel on ARM (read from kernel master, 2026-10-24)
- Files in `src/arch/aarch64/device/`: `irqchip/{gic.rs (GICv2: arm,cortex-a15-gic / arm,gic-400), gicv3.rs (arm,gic-v3, 204 lines),
  irq_bcm2835.rs, irq_bcm2836.rs}`, `generic_timer.rs` (ARM generic timer, per-CPU programmable since 2026-07-26),
  `psci.rs` (hvc or smc conduit read from DT `method`; CPU_ON, SYSTEM_OFF, SYSTEM_RESET), `serial.rs`, `rtc.rs`, `cpu/{boot,topology}.rs`.
  Irqchip chosen by DT compatible string (`irqchip/mod.rs`).
- UARTs from DT (`src/dtb/serial.rs`): arm,pl011, ns16550a, snps,dw-apb-uart, Amlogic Meson. No Qualcomm GENI/MSM serial.
  This matters for SDM845 (its debug UART is GENI QUP serial), so there would be no kernel console unless GOP/USB-serial is used.
- SMP: commit series 2026-07-25 to 2026-08-23 ("Bring up PSCI secondary CPUs through a physical trampoline", SGI IPIs, TLB shootdown,
  DT CPU topology, NUMA-aware trampoline). Very new; the Aug post says it needs more testing.
- MMU: kernel runs at EL1 (exception code handles "EL1 with SPx"; ELR_EL1/SPSR_EL1). Expects to be entered from UEFI at EL1 or EL2 handoff
  by the bootloader; I did not find explicit EL2 handling and mark that UNKNOWN. PTEs default to Inner Shareable (2026-07-24).
- Kernel size: ~42.9k lines of Rust in `src`, ~6.8k of that aarch64 arch + dtb.
- The kernel exposes the DTB to userspace as `/scheme/kernel.dtb` (used by `hwd` and `bcm2835-sdhcid`).
- Not found / UNKNOWN: Qualcomm GIC redistributor quirks, Qualcomm SMMU/IOMMU handling (SDM845 has an SMMU; Redox has no IOMMU code that I found).
- SDM845 itself (not Redox): GICv3 + ARM generic timer + PSCI, so those three kernel pieces are in principle compatible. UNVERIFIED on hardware.

## 3. Driver model
- Drivers are user-space daemons using schemes. Primary doc: `base/drivers/README.md` (repo `redox-os/base`, driver tree moved out of the
  old `redox-os/drivers` repo, whose last commit is 2025-11-29).
  - `/scheme/memory/physical[@wb|@uc|@wc]` for MMIO/DMA-memory mapping; `/scheme/irq` for interrupts, polled via `/scheme/event`.
  - Libraries: `redox-scheme`, `redox_event`; example driver: https://gitlab.redox-os.org/redox-os/exampled
  - `drivers/common/src/dma.rs` has aarch64 cfg (DMA helper). `scheme/irq.rs` in kernel has aarch64/riscv64-specific code.
- Discovery: PCI via `pcid`/`pcid-spawner` (config-file matching `/etc/pcid.d`), ACPI via `acpid`.
  There is an in-progress `hwd` daemon with a `devicetree` backend (47 lines) that only reads `/scheme/kernel.dtb`,
  parses it and DUMPS nodes (`probe()` logs and returns). No generic DT-compatible -> driver spawner exists yet; verified by reading the source.
- Only DT-driven platform driver in the tree: `storage/bcm2835-sdhcid` (916 lines, Raspberry Pi SD host; finds its node in the DTB by compatible
  string itself). It is bundled in the ARM64 image since July 2026.
- Existing ARM-usable drivers: virtio (blk 283, net 235, gpu 1235, core 1819 lines) via PCI on QEMU virt; xHCI (xhcid 9074 lines, PCI only);
  usbhidd, usbscsid (1622), nvmed (1466), ahcid, pl011/ns16550/Meson UART (in kernel); `vesad`/`fbcond` framebuffer (UEFI GOP style).
  UNKNOWN whether xhcid can be attached to a non-PCI (platform/DWC3) controller; its code takes a PCI BAR.
- No drivers anywhere in the tree for: Qualcomm UFS, SDHCI/eMMC (generic), I2C/GENI QUP, DWC3, touchscreen, display panel (MDSS/DSI), GPU, modem, Wi-Fi, audio (Qualcomm ADSP/WCD934x), battery/PMIC. Verified by directory listing of `base/drivers` (2026-10-22).

## 4. Userspace on aarch64
- Orbital and COSMIC apps exist for aarch64: package index https://static.redox-os.org/pkg/aarch64-unknown-redox/ (~113 packages;
  orbital, orbutils, cosmic-edit/files/term, netsurf, rust 79M, llvm21/lld21; NO gcc compiler package, only libgcc/libstdcxx) (fetched 2026-10-24).
- relibc: aarch64 fixes landed in 2026 (TLS offset, setjmp linker errors, ld.so relocation; Mar/Aug 2026 posts above).
- Networking: smoltcp-based netstack in `base/netstack`; hardware NIC drivers are PCI x86-oriented (e1000, rtl8168, ixgbe) plus virtio-net.
  No Wi-Fi driver stack at all. UNKNOWN whether any real ARM board gets a working NIC.
- Audio: `audiod` exists (base); hardware drivers are ac97/ihda/sb16, all x86 PCI. No ARM audio hardware driver.
- Input: ps2d (x86), usbhidd (USB HID via xHCI). USB gamepad support added May 2026. No touchscreen/evdev for touch.
- Verified-working aarch64 environment is QEMU (virt) per release notes; real-board desktop usage is not documented as working.

## 5. Roadmap, funding, people, phone attempts
- Funding (June 2026 post https://www.redox-os.org/news/this-month-260630/): NLnet/NGI Zero Commons EUR 50,000 "Virtualized Redox"
  (virtio, networking, RedoxFS, WASM; 4 part-time devs + 4lDO2); separate EUR 11,500 grant for capability-based security.
  Neither funds ARM hardware or phones. Operating costs >$3,000/month vs <$1,000 revenue.
- May 2026 post: two RSoC 2026 projects (scheduler: new CPU scheduler / EEVDF), >$5,000 each.
- Active ARM contributors named in posts: Luiz Fernando (Amlogic S905W2, UEFI cacheability), lbecher (AArch64 SMP), Wildan Mubarok
  (many fixes, BCM2835), bjorn3 (drivers), Philipp Bartsch (CPU features/RNG), Anhad Singh (loader), plus Jeremy Soller, 4lDO2.
- RISC-V work also active (posts mention Sv39 MMU), competing for attention.
- PHONE PRECEDENT: Paul Sajna booted the AArch64 Redox desktop live image on a BlackBerry KEY2 LE (Qualcomm; the post does not name the SoC)
  via postmarketOS-style Qualcomm U-Boot (Android boot image on the boot partition, `bootefi bootmgr`).
  Result: login prompt on the phone screen; no touch, so unusable. Details: the Redox bootloader's DTB parsing had to be hack-patched
  (`src/os/uefi/dtb.rs` read as 32-bit, not upstreamed), `pcid-spawner` removed from init to avoid boot halt, intermittent memory-map parse error,
  serial via U-Boot USB gadget at 115200. No kernel change was needed.
  Sources: https://blog.paulsajna.com/redox-in-your-pocket/ (2025-09-21), https://www.osnews.com/story/143404/yes-redox-can-run-on-some-smartphones/ (2025-09-22),
  https://hackaday.com/2025/09/24/who-wants-a-rusty-old-smartphone/ (2025-09-24). The OnePlus 6 is NOT the device used (UNKNOWN whether anyone tried it).
- U-Boot on SDM845: postmarketOS "SDM845 generic device support" MR (gitlab.postmarketos.org/postmarketOS/pmaports/-/merge_requests/4599),
  reportedly tested on OnePlus 6; U-Boot docs https://docs.u-boot.org/en/v2022.07/board/qualcomm/sdm845.html. Not independently re-verified by me.

## 6. Effort signals (line counts of .rs files, base repo 2026-10-22)
| Component | Lines |
|---|---|
| bcm2835-sdhcid (platform SD host) | 916 |
| nvmed | 1466 |
| ahcid | 1225 |
| xhcid (USB host) | 9074 |
| usbscsid | 1622 |
| virtio-core | 1819 |
| virtio-gpud | 1235 |
| driver-graphics (shared) | 2898 |
| ihdgd (Intel graphics) | 4822 |
| pcid | 3019 |
| acpid | 2860 |
| all drivers/ combined | ~50.9k |
| kernel src | ~42.9k |
- Ballpark for an SDM845 bring-up (my estimate, not sourced): UFS host + SCSI glue 3-6k lines, DWC3/xHCI platform glue + PHY 2-4k, GENI I2C + touch 1-2k,
  framebuffer via UEFI GOP is free but panel-off/backlight control is not; GPU (Adreno 630) is infeasible in scope; modem/Wi-Fi/audio out of scope.

## Summary (10 lines)
1. Redox aarch64 boots on QEMU virt, Raspberry Pi 3 B+ (docs), Amlogic S905W2 (July 2026), and a Qualcomm phone (BlackBerry KEY2 LE, Sept 2025, login screen only).
2. No release since 0.9.0 (2024-09); use daily images or build from source; ARM docs are outdated ("limited support").
3. Kernel has GICv2/GICv3, generic timer, PSCI (hvc/smc), DT-based irqchip/UART discovery, and brand-new SMP (Jul-Aug 2026, lightly tested).
4. SDM845 uses GICv3 + generic timer + PSCI, so the kernel core is plausibly a fit; Qualcomm-specific serial (GENI), SMMU and EL2 handoff are untested.
5. Boot path is realistic: postmarketOS-style U-Boot on OnePlus 6 -> EFI -> Redox bootloader -> kernel, as Sajna did on a sibling Qualcomm phone; expect to patch bootloader DTB parsing.
6. Drivers are user-space schemes, but DT-driven platform discovery is only a stub (`hwd` devicetree backend dumps nodes); each driver must find its own DT node (bcm2835-sdhcid pattern).
7. Missing for a usable phone: touchscreen (GENI I2C + panel touch IC), storage (UFS) beyond a USB/live image, USB host/gadget (DWC3 platform glue; xhcid assumes PCI), PMIC/battery/charging, display power/brightness.
8. Missing for a "full" phone: Wi-Fi/Bluetooth/modem, audio, GPU acceleration, cameras, sensors, power management (all with no ARM-side Redox groundwork).
9. Userspace is ready enough once input and storage exist: Orbital, COSMIC apps, NetSurf and Rust exist as aarch64 packages; no GCC package.
10. Verdict: Redox can realistically boot to a GUI login on OnePlus 6 soon (days to weeks, bootloader patch + U-Boot); a usable touch UI needs several new drivers (weeks to months of focused work); a daily-driver phone is not realistic. Upstream funding does not cover this; expect to carry patches yourself.
