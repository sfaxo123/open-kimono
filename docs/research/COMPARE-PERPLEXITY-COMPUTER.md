# Notes compared: Perplexity Computer plan vs ours (2026-10-24 ~3:30 PM PT)

Source: Perplexity Computer artifact "Redox OS on OnePlus 6 — Driver Port Plan" (draft v2, merges Gemini draft +
Perplexity/Grok fact-checks), session task 638dce5d. Ours: PLAN.md / SYNTHESIS.md / FINDINGS-T1..T5.

## Agree
Boot path (ABL → U-Boot as Android boot image → Redox EFI), `fastboot boot` only, framebuffer first, EL1 only,
single core first, reuse firmware-initialised state, touch = Synaptics RMI4 s3706b on i2c12, panel
sofef00-ams628nw01, USB2 peripheral only, audio/Wi-Fi last, no calendar estimates.

## They add (adopt all — verify each fact against our local clones before coding)
1. **GICv3 is single-core only in Redox today**: kernel `src/arch/aarch64/device/irqchip/mod.rs` says secondary-CPU
   bring-up supports GICv2; GICv3 "remains on the safe single" path. SMP on SDM845 = new kernel work (redistributor
   per CPU + PSCI CPU_ON).
2. **PDC** (`qcom,sdm845-pdc` @0x0b220000) sits in front of the GIC for wake IRQs, and **TLMM needs a kernel
   irqchip** + `interrupts-extended` support — Redox rejects devices using `interrupts-extended` today.
3. **No SMMU driver** in Redox (`apps_smmu`, smmu-500): DMA risk — check how firmware leaves it (bypass vs fault).
4. **Exact boot framebuffer**: simple-framebuffer a8r8g8b8 1080×2280, stride 4320, @0x9d400000 (36 MiB, no-map,
   `cont_splash_mem`) — Phase 3 target via a simplefb path for vesad/fbcond.
5. **Debug UART**: `uart9` `qcom,geni-debug-uart` @0x00a84000 115200n8 → new kernel serial kind in `dtb/serial.rs`
   (kernel UARTs today: PL011, ns16550a, dw-apb, Meson).
6. **Buttons**: power = PM8998 PON; volume = PM8998 GPIOs 5/6 (PMIC, not TLMM) → needs an SPMI arbiter daemon
   early; alert slider is on TLMM.
7. **Brightness** via MIPI DCS to the panel, not PMI8998 WLED (WLED disabled on this phone).
8. **Redox APIs, concretely**: `/scheme/memory/physical@uc|wc|wb` for MMIO, `/scheme/irq/…` (incl. `irq:phandle-N`
   with DTB) + `/scheme/event`; template daemon `exampled`; schemes `display.vesa`, `input`, `disk.*`. The GitHub
   mirror of redox-os/drivers was archived 2026-04-28 — work on GitLab (drivers now live in `base`).
9. **Gemini corrections**: no PM845 (PM8998/PMI8998/PM8005); `driverkit`/`pciscan` don't apply; display is
   MDSS+DPU (Adreno is the GPU); UFS 2.1 not NVMe; scheme names differ; modem vs Wi-Fi are separate subsystems.
10. **Licensing (important, missing from ours)**: Linux drivers and the enchilada DTS are GPL-2.0; Redox is MIT.
    A Rust translation of a Linux driver is a derivative work. **Per-driver license decision before code**:
    clean-room from DT facts + permissive references (OpenBSD, edk2, JEDEC UFSHCI spec) for MIT, or a separate GPL
    repo when derived. (Asahi/Haiku policies cited.)
11. **Test device**: strongly prefers a **second OnePlus 6** for destructive work; daily phone = `fastboot boot` only.
12. **Reference snapshot list** for Phase 0: dmesg, /sys/firmware/devicetree, clk_summary, regulator_summary,
    /proc/iomem, /proc/interrupts, pinctrl — matches our W1 / ACCEL-R3 oracle idea.
13. Per-driver workflow: DT bindings → capture behaviour on reference phone → license decision in README →
    daemon from `exampled`, polling first → IRQ → DMA → `fastboot boot` test with logged runs.
14. Its next action #1: "reconcile with the 3090 Claude Code transcript" — i.e. this file.

## We add (not in theirs)
- Linux-as-oracle depth (register/regmap/MMIO tracing, QRTR/APR capture, driving blocks from Linux user space via
  UIO before Redox) — ACCEL-R3.
- Compatibility-layer option for big stacks (LinuxKPI / DDE-Linux style) — ACCEL-R2.
- Parallel agent factory + acceptance gates — ACCEL-R5; `oneplus-phone` skill, Wi-Fi watchdog, USB recovery path.
- Paul Sajna's KEY2 LE Redox boot (exact precedent) — T1/T5.

## Decisions needed (operator)
- Second OnePlus 6 for destructive tests? (theirs: strongly preferred)
- License policy: MIT clean-room where possible + separate GPL repo for derived drivers?
