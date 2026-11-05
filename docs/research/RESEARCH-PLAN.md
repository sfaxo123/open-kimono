# Research plan — rebuilding the OnePlus 6 phone OS from the ground up (Redox OS candidate)

Started 2026-10-24 ~12:45 PM PT. Input: Gemini draft (GEMINI-DRAFT.md). Method: 5 parallel research tracks
(Sonnet subagents, web search, cite sources with dates), then cross-check the decisive claims with Perplexity and
Grok, then a synthesis with a recommendation and an execution plan. Every claim gets a source + date; anything
unverified is marked UNKNOWN.

## Decisive questions (the answer to these decides the architecture)
1. Does Redox OS boot on aarch64 hardware today (late 2026), how (UEFI? device tree?), on which boards, and how
   complete is its ARM userspace (drivers, graphics, input, audio, networking)?
2. How do we get a non-Linux kernel running on the OnePlus 6: ABL/fastboot, mainline U-Boot (Qualcomm SDM845),
   UEFI (edk2 / Renegade Project used for Windows on ARM on enchilada), simplefb/GOP framebuffer handoff, EL1 vs
   EL2 (Qualcomm hypervisor), serial/UART access, bricking risk and recovery (EDL/9008).
3. Driver-by-driver cost on SDM845 vs. what the voice-phone goal needs: display, touch, USB (gadget for debug!),
   UFS, PMIC/battery, Wi-Fi (WCN3990 needs remoteproc + QMI + firmware), audio + mic (ADSP/Q6 — needed for the
   voice assistant), GPU, modem.
4. Alternatives that reach a "custom, shareable, intelligent phone OS" sooner: Linux kernel + our own Rust
   userspace/shell; Genode; seL4; Fuchsia; other Rust OSes; hybrid (Redox userspace concepts on Linux).
5. Prior art: anyone running Redox/other microkernels on Qualcomm phones or on SDM845 specifically.

## Tracks
- T1 Redox on aarch64 — status, boot path, driver model (schemes, `pcid`/`acpid`, FDT), roadmap, community.
- T2 SDM845/OnePlus 6 boot firmware — U-Boot, edk2/Renegade UEFI, fastboot boot images, UART, EDL recovery.
- T3 SDM845 driver inventory — per subsystem: Linux driver(s), LOC, firmware/DSP dependency, difficulty.
- T4 Alternatives + prior art — other OSes on phones, Rust OS work on aarch64, Linux+custom userspace phones.
- T5 Cross-check — Perplexity and Grok answer the decisive questions independently; disagreements listed.

## Output
`FINDINGS-T1..T5.md`, then `SYNTHESIS.md` (recommendation, risks, what we keep from the current Linux build),
then `PLAN.md` (phased execution, each phase with a demo-able milestone and an exit test).
