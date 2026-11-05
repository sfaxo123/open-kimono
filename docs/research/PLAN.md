# PLAN — Redox OS on the OnePlus 6 (SDM845), every driver ported

Operator decision 2026-10-24 1:15 PM PT: port everything to Redox; open-source code is the reference; build in
parallel but systematically, one driver at a time to "done"; use every resource. No timelines — order and
acceptance only. Research: FINDINGS-T1..T5, SYNTHESIS.md (this folder).

## Ground rules
- **Reference, not guess:** every driver starts from a written spec extracted from mainline Linux (driver + the
  `sdm845.dtsi` / `sdm845-oneplus-enchilada.dts` nodes), cross-checked with U-Boot / edk2-sdm845 where they have
  the same block. Register maps and sequences are cited by file:line. (GPL code is a reference; our Rust
  drivers are written fresh and licensed to match Redox — MIT.)
- **Prove on the 3090 first, then the phone:** QEMU aarch64 for kernel/userspace changes; a register-level
  model or recorded MMIO trace for drivers where possible; the phone only with `fastboot boot` (nothing flashed)
  until a stage is proven.
- **Brick safety:** full partition backup before the first phone boot; never repartition; never touch
  xbl/abl/devinfo/persist/modem; pmOS stays bootable on the other A/B slot; USB recovery path kept.
- **One driver "done" = ** builds in the Redox cookbook, has host tests, runs on the phone, is documented in
  `docs/phone-os/ISSUES.md` style, and is offered upstream to Redox.
- **Parallel but ordered:** many drivers get their specs and skeletons in parallel; each is finished in
  dependency order (a driver can't be "done" before the bus it sits on).

## Workstreams and dependency order
W0 **Workbench (3090)** — Podman + QEMU aarch64; clones under `~/Developer/forks/`: redox build system, kernel,
   bootloader, base (drivers), relibc, cookbook; linux (mainline, sparse: arm64 qcom + dts), u-boot, edk2-sdm845,
   pmaports (enchilada). Baseline: Redox aarch64 image boots to login in QEMU on the 3090.
W1 **Phone facts + safety** — measure CurrentEL, dump the live DTB, `/proc/iomem`, clocks/regulators state from
   pmOS (the running Linux is our best register-state oracle); back up partitions; confirm enchilada U-Boot.
W2 **Boot chain** — U-Boot for enchilada as Android boot image (`fastboot boot`); EFI diagnostic app (CurrentEL,
   memory map, DTB, framebuffer); then Redox bootloader + kernel + RAM initfs → framebuffer console. Apply/redo
   Sajna's fixes (DTB cell parsing, no pcid-spawner) properly. Exit: Redox shell text on the phone screen.
W3 **Platform core** — DT-driven device discovery in Redox (an `fdt`-based spawner so drivers bind by
   `compatible`), GICv3 + timer + PSCI on SDM845 (single CPU first, then SMP), reserved-memory carve-outs,
   SMMU awareness, SCM calls. Exit: drivers start from DT; interrupts delivered to user-space drivers.
W4 **TLMM pinctrl/GPIO** → W5 **GENI serial engine** (UART first = debug console, then I2C, SPI) →
W6 **Touch** (Synaptics RMI4 s3706b on GENI I2C) → `input:` → Orbital touch. Exit: tap on screen moves Orbital.
W7 **Clocks + power infra** — cmd-db, RPMh (RSC/TCS), GCC/dispcc clocks, interconnect, SPMI + PMIC (PM8998,
   PMI8998, PM8005) regulators, GPIO keys (power/volume), bq27411 fuel gauge, charger. Exit: battery % and
   charging state read on Redox; volume keys as input.
W8 **USB** — DWC3 + qcom glue + QUSB2 PHY, peripheral mode: CDC-NCM gadget (network over the cable — same path
   we use today). Exit: ssh-like shell into Redox over USB from the 3090.
W9 **Display** — keep the bootloader framebuffer; then DSI + DPU + panel `samsung,sofef00-ams628nw01` for
   native control (brightness, blank/unblank). Exit: screen off/on and brightness from Redox.
W10 **Storage** — UFS (UFSHCD + ufs-qcom + PHY) → partitions → RedoxFS on a dedicated partition. Exit: Redox
   boots from internal storage (still selected via fastboot/U-Boot).
W11 **Remote processors** — remoteproc (PAS via SCM), GLINK/SMEM, QRTR, QMI; pd-mapper/rmtfs/tqftpserv
   equivalents. Shared by Wi-Fi and audio. Exit: ADSP and modem firmware booted and talking QRTR.
W12 **Audio** — APR, Q6 AFE/ASM/ADM/routing, SLIMbus NGD, WCD9340 codec, MAX98927 amp, Redox `audio:` scheme.
   Exit: mic capture to a WAV and playback through the speaker on Redox.
W13 **Wi-Fi** — WCN3990 (ath10k SNOC) + QMI firmware services + a Redox Wi-Fi/network stack hookup. Exit:
   Redox on Wi-Fi reaches the 3090.
W14 **Voice phone on Redox** — port the voice stack (llama.cpp, STT, TTS, UI) to Redox userspace; the product
   rules (always visual + audible, explains homelab handoffs) carry over.
Later: GPU (Adreno 630), modem/telephony, sensors, camera, suspend.

## Resources (who does what)
- **Claude (orchestrator):** specs, reviews of every diff, integration, phone sessions (with operator summoned
  via `oneplus-phone`), commits.
- **Codex / Cursor:** spec-to-Rust driver implementations and test harnesses (one driver per job, own worktree).
- **DSH on local lanes (5090 / Spark / 3090):** bulk reading of Linux sources into specs, register-map
  extraction, test scaffolding; small specs per step (lesson from 10-23).
- **Sonnet subagents:** research questions, reference cross-checks. **Opus:** hard bring-up debugging.
- **Perplexity / Grok:** second opinions on contested hardware facts.

## Immediate next actions (this session)
1. W0: install Podman + qemu-system-aarch64; clone repos; build Redox aarch64 and boot it in QEMU (background).
2. W1: when the phone is back online — read-only facts dump (EL, DTB, iomem, regulators), then backups.
3. Spec factory: first specs in parallel — U-Boot enchilada boot image, GENI UART, TLMM, GENI I2C, RMI4 touch,
   DT-based driver spawning in Redox.

## Where things live
- Upstream clones: `~/Developer/forks/<project>`; our changes on branches named `op6/<workstream>`.
- Plans/specs/receipts: this folder (`~/artifacts/handoff/oneplus-redox-20261024/`), specs under `specs/`.
- Public-facing docs and the issue log: continue in `docs/phone-os/` (voice-stack-tryout) until the operator
  creates the public repo.

## v2 deltas from Perplexity Computer's plan (2026-10-24 ~3:30 PM PT) — adopted, see COMPARE-PERPLEXITY-COMPUTER.md
- W3 adds: PDC irqchip, TLMM kernel irqchip + `interrupts-extended` support, SMMU-500 status check, and GICv3 SMP
  (Redox's secondary-CPU path is GICv2-only today).
- W2/W5: early console = framebuffer text writer on simple-framebuffer @0x9d400000 (1080×2280 a8r8g8b8, stride 4320)
  and a `qcom,geni-debug-uart` kernel serial kind (uart9 @0x00a84000).
- W7 moves earlier for buttons: SPMI arbiter daemon (power = PM8998 PON, volume = PM8998 GPIO 5/6).
- W9: brightness via MIPI DCS (sofef00), not WLED.
- Every driver README records a **license decision** (MIT clean-room from DT facts + permissive references, or a
  separate GPL repo if derived from Linux code) before code is written.
- Driver daemons start from Redox `exampled`; MMIO via `/scheme/memory/physical@uc`, IRQs via `/scheme/irq` +
  `/scheme/event`; drivers live in GitLab `redox-os/base` (GitHub drivers mirror archived).
- **License policy (operator, 10-24): MIT clean-room for anything meant for upstream Redox (DT facts, specs, permissive refs: OpenBSD, edk2, JEDEC); code derived from Linux lives in a separate GPL repo.**
