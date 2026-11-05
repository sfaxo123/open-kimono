# Synthesis — ground-up rebuild of the OnePlus 6 phone OS (2026-10-24)

Sources: FINDINGS-T1 (Redox aarch64), T2 (boot paths), T3 (driver inventory), T4 (alternatives), T5
(Perplexity + Grok cross-check), Gemini draft. Six independent sources; conclusions below are where they agree.

## Bottom line
1. **Redox can boot on the OnePlus 6.** Its bootloader already does AArch64 UEFI + device-tree handoff, and
   someone already booted Redox to the login screen on another Qualcomm phone (BlackBerry KEY2 LE, Sept 2025) with
   two small hacks and no kernel changes. Path: ABL → U-Boot (as an Android boot image) → Redox EFI → kernel,
   tested with `fastboot boot` only, so nothing is flashed.
2. **Every phone driver is new on Redox:** storage (UFS), touch, USB, power/battery, display, Wi-Fi, audio.
   Mainline Linux, U-Boot, edk2-sdm845 and postmarketOS are the open-source references for each.
3. **Audio is the wall, and audio is our product.** Microphone and speaker go through Qualcomm's ADSP firmware
   and its messaging stack (remoteproc → GLINK → QRTR → APR → Q6 services → SLIMbus → WCD9340 codec). It sits
   last in the dependency chain, so the platform drivers under it come first.
4. **No shortcut via virtualization.** Qualcomm's hypervisor owns EL2 on SDM845 (no known unlock), so "Redox or
   seL4 + a Linux VM for drivers" is blocked on this phone.
5. **Every source independently recommends** keeping the mainline SDM845 Linux kernel for the product and
   rebuilding everything above it — and running Redox as a bounded, upstream-friendly side track.

## Recommended architecture: two tracks, one repo
**Track 1 — "our own OS" on the Linux kernel (the product, shareable):**
- Keep: mainline SDM845 kernel + device tree + phone firmware blobs (what already gives us mic, speaker, Wi-Fi,
  touch, display, USB, battery today).
- Rebuild from the ground up, on the 3090: a minimal, reproducible image (no Phosh/GNOME/Firefox dependency):
  our own init/service layout, a Rust shell/compositor that IS the voice UI (waveform, transcript, cleaned text,
  live reasoning, speech — always visual + audible), small Rust services with narrow capability-style
  interfaces (Redox's "everything is a scheme" idea applied on Linux), voice stack (STT, LLM, TTS) as services,
  homelab fallback that explains itself. Image built and provisioned as code; hardware profile per device.
- This is the "customizable, shareable, super-intelligent phone OS"; another device = another profile + kernel.

**Track 2 — Redox on the OnePlus 6 (research, upstream contributions):**
- Bounded milestones, each a demo: EFI diagnostic app → Redox from RAM to framebuffer shell → touch (GENI I2C +
  Synaptics RMI4) as our first upstream driver → USB gadget networking → then decide.
- Design Track 1's service interfaces so they could later sit on Redox schemes — the shared seam between tracks.

## What we keep from today's build
Voice pipeline lessons and code (prompt cache `--swa-full`, echo guard, streaming trace, wake-free listening),
`docs/phone-os/ISSUES.md`, provisioning/profile approach, `oneplus-phone` skill, Wi-Fi watchdog, USB recovery.

## Risks
- Bricking: mitigate with `fastboot boot` only, full partition backups first, other A/B slot keeps pmOS, never
  repartition; EDL tool staged. (T2 lists the exact partitions.)
- Debug visibility: UART needs soldering to 1.8 V test points; plan on framebuffer console + USB gadget instead.
- Unverified: pmOS U-Boot on enchilada (verify on pmaports), pmOS wiki feature table, CurrentEL (measure).

## Decision (operator, 1:15 PM PT)
**Port everything to Redox.** Use the open-source code (Linux, U-Boot, edk2, pmOS) as reference, build in
parallel but systematically one driver at a time, with every available resource. No timelines. See PLAN.md.
