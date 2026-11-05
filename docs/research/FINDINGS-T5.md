# T5 — cross-check: Perplexity and Grok (2026-10-24 ~1:05 PM PT)

Same decisive question sent to both (Redox aarch64 status, OnePlus 6 feasibility, boot paths, EL1, drivers,
prior art, alternatives). Conversations: Perplexity `perplexity.ai/search/772de211-…` (62 sources, 30 s);
Grok `grok.com/c/88185d5d-…` (90 sources, Fast mode, 2 m 47 s).

## Where both agree with T1–T4
- Redox aarch64 = "limited support" (official book), QEMU most complete, Pi 3B+ documented, multicore landed
  Aug 2026; bootloader has real AArch64 UEFI + DTB handoff (prefers DT over ACPI).
- Boot path for OnePlus 6: ABL → Android boot image wrapping **U-Boot or edk2** → Redox EFI loader → kernel.
  `fastboot boot` for temporary tests. U-Boot fastboot lacks a UFS backend (Perplexity, upstream docs).
- **Prior art on a Qualcomm phone:** Paul Sajna, "Redox in your pocket" (2025-09-21) — U-Boot + bootefi,
  patched DTB cell parsing, removed pcid-spawner, reached Orbital login, stuck without touch (Grok + T1).
  Grok also cites a RustConf 2026 lightning talk showing phones reaching login, blocked on input.
- EL1 only under Qualcomm's hypervisor; not a blocker for a native Redox kernel, but rules out
  "microkernel + Linux driver VM" (qhypstub doesn't cover SDM845; seL4 summit 2024: couldn't get EL2).
- Driver difficulty: UART low-med, touch medium (once GENI I2C/GPIO work), USB/UFS/display med-high,
  PMIC/power high & cross-cutting, Wi-Fi very high, **ADSP audio very/extremely high**.
- Both recommend: for a usable, voice-first phone keep the SDM845 Linux kernel and replace userspace; run
  Redox as a bounded RAM-resident framebuffer experiment.

## New or different details
- Perplexity: Pocketblue documents a OnePlus 6/6T U-Boot in `boot` + ESP in `system_b` (credible U-Boot path).
  Mobian on OnePlus 6 tested 2026-04-26 and 2026-06-21 per Debian wiki (Wi-Fi, camera, speaker).
- Perplexity's bounded Redox sequence: EFI diagnostic app first (CurrentEL, memory map, DTB, framebuffer),
  then Redox from RAM, single CPU, shell on framebuffer, then touch or USB gadget before native display.
- Grok says "Redox has Adreno support" in its table — almost certainly a dropped "no" (every other source:
  framebuffer only on aarch64). Treated as an error.
- Grok: edk2 gives UFS access via binary drivers extracted from XBL — firmware-time only, not runtime.

## Disagreements / unverified
- Whether postmarketOS's U-Boot is "tested on OnePlus 6" — search snippets only (T1, T2); verify on pmaports.
- pmOS wiki feature table for enchilada — bot-walled for agents; check in a real browser.
- CurrentEL on our phone — measure (`dmesg | grep -i "EL[12]"`) rather than infer.
