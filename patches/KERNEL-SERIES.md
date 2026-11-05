# Downstream kernel series

Pinned base: `a0a8ff96b065bd8f33ca3e0742fb09f246c1e32a` (parent of the first `op6/e1-demo` kernel commit). Apply `patches/kernel-series/*.patch` in filename order with `git am`. `scripts/integrate.sh` uses the same order and skips subjects already present on its pinned `op6/e1-demo` checkout. These 46 patches are the only active downstream kernel series; old kernel patch folders are retained in Git history.

The self-parent fix is patch 0013 only. It keeps the EMU-TOUCH topology helper and host tests, and thus also covers the EMU-BUTTONS inline guard. Patch 0017 is rebased on the GICv3 redistributor parser: it sets Group 1 after distributor initialization. Patch 0021 adds diagnostics behind the `wy_sleep_diag` feature, which is off by default.

| Order | Purpose | Origin lane | Test |
|---|---|---|---|
| 0001 | Inherited DT interrupt domains | kernel | Fresh `git am`; QEMU boot |
| 0002 | SDM845 PDC decoder | kernel-irqchips | PDC unit tests |
| 0003 | PDC and TLMM domains | kernel-irqchips | OnePlus touch route |
| 0004 | Scheduler test Kstack | kernel-geni-uart | Kernel build |
| 0005 | GENI debug UART | kernel-geni-uart | OnePlus serial boot |
| 0006 | GENI UART DT selection | kernel-geni-uart | OnePlus serial boot |
| 0007 | Full-width IRQ descriptors | integration-3/kernel, resolving kernel-irq-width | Kernel build; touch SPI 647 |
| 0008 | Full-width GICv3 INTID | integration-3/kernel, resolving kernel-irq-width | Touch SPI 647 |
| 0009 | GENI UART receive IRQ | integration-3/kernel, resolving kernel-irq-width | OnePlus serial boot |
| 0010 | DT simple framebuffer discovery | kernel-fbcon | Generic QEMU simplefb |
| 0011 | Graphical boot console | kernel-fbcon | Generic QEMU simplefb |
| 0012 | AArch64 simplefb boot console | kernel-fbcon | Generic QEMU simplefb |
| 0013 | Self-parented controller as root, wake-route log | touch-e2e/kernel; folds EMU-BUTTONS guard | `irq_topology.rs` host tests; touch boot |
| 0014 | GICv3 redistributor private IRQs | EMU-BUTTONS kernel-gicv3-private-irq | `model.rs` tests; timer PPI |
| 0015 | Reject malformed DT redistributor stride | orchestrator kernel-gicv3-private-irq | `model.rs` tests |
| 0016 | Per-controller CPU affinity registry | orchestrator kernel-gicv3-private-irq | `model.rs` tests; Wyoming single CPU |
| 0017 | GICv3 shared IRQs in Group 1 | touch-e2e/kernel, rebased after 0014–0016 | `gic_groups.rs` tests; touch IRQ |
| 0018 | Keep driverless summary IRQs masked | touch-e2e/kernel | OnePlus serial/trace |
| 0019 | First timer PPI log | EMU-BUTTONS kernel-gicv3-private-irq | Button serial marker |
| 0020 | Retain lock-contended due timers | kernel-sleep-wake | Kernel build; Wyoming guest if admitted |
| 0021 | Opt-in bounded sleep IRQ diagnostics | kernel-sleep-diag | Kernel build with feature off |
| 0022 | Pure affinity routing encoders (APs remain gated) | orchestrator host-only prerequisite | 30 standalone model tests; no runtime proof |
| 0023 | One-bit W1S/W1C interrupt enable/disable | orchestrator regression fix | Actual-method host RED/GREEN, including IRQ 31/32; runtime pending |

| 0024 | Defer AArch64 serial callback EOI until after handler borrow | Perplexity5090/Cursor; Codex/Luna review | 4 production-method host tests, negative control, A64 check/link; no runtime proof |

| 0025 | Preparatory IRQ-save mutex; unsafe closure scope, not wired to IRQ_CHIP | Perplexity5090/Cursor; Codex/Luna correction/review | 6 actual-source host tests, 2 semantic negative controls, 4 compile fixtures, A64/RV codegen, A64 check/link; no runtime proof |

| 0026 | TLMM one-cleared-pending-pin step used by existing handler | Perplexity5090/Cursor; Codex/Luna review | Actual old/new trace parity,10host tests,4behavioral negative controls,A64check/link; outer core ownership/runtime unchanged |

| 0027 | GICv3 no-log RD lookup with owned diagnostic; existing wrapper log/Option retained | Perplexity5090/Cursor; Codex/Luna and Spark evidence review | 42host tests incl30existing model,120wrapper parity cases,4behavioral controls,A64check/link; global ownership/runtime unchanged |

| 0028 | PLIC scalar MMIO addresses; no mapped register references; preserve !Send/!Sync | Perplexity5090/Cursor partial; Codex completion; Luna/Spark review | 8actual-source tests/444parity cases,4compiled behavioral controls,4negative+2positive trait fixtures,RV64+A64check/link; no global/runtime proof |

| 0029 | BCM2835 owned pending/claim step, existing registered-handler path | Codex inline; Luna review; Spark evidence with errata | Debug/release4tests each,1500actual-source parity cases/profile,4compiled controls/profile,A64check/link,fresh29patch tree equality; no global/runtime proof |

| 0030 | BCM2836 inclusive last IRQ descriptor count | Codex inline; Luna review | Actual initializer baselineFAIL/corrected3PASS,3compiled controls,A64check/link,fresh30patch tree equality; source-only |

| 0031 | GENI RX four-byte packing and configuration-aware test model | Codex; same Perplexity5090 review with root errata | Baseline hostFAIL/corrected20PASS; controlled23+RX interactive emulator help/echo/pwd/ls; full31 A64 link and fresh patch-tree equality. No physical RX proof |

| 0032 | Identify PSCI SYSTEM_RESET (0x84000009) at both AArch64 reset entrypoints; conduit selection already exists in pinned base | R-1 reset | Fresh `git am`; emulator serial marker and reset behavior |

| 0033 | Opt-in QCOM SCM download-mode cookie clear before AArch64 reset; feature is off by default | R-1 reset | AArch64 kernel build with feature enabled; Linux SCM ABI source review; no hardware call proof |
| 0034 | qcom SCM: read the x1 result, pass 0x10 to SET_DLOAD_MODE (Linux qcom_scm-smc.c) | R-1 review (Claude) | Kernel build with qcom-dload-mode-clear |
| 0035 | qcom SCM availability flag is static, not const (the clear never ran) | R-1 review (Claude) | QEMU: clear attempted, refused, PSCI fallback |
| 0036 | qcom SCM: emit smc #0 as .inst (as psci.rs) | R-1 review (Claude) | Kernel build with qcom-dload-mode-clear |
| 0037 | qcom SCM: retry the yielding availability call on INTERRUPTED | R-1 review (Claude) | QEMU ttl + reboot PASS |
| 0038 | qcom SCM: print `qcom SCM: download-mode cleared` on clear success at both reset entrypoints | P2 observability (root) | Kernel build with qcom-dload-mode-clear; QEMU gate serial shows the success line or the failure warn |
| 0039 | kreset: 5 s timer hold after the SCM result print so a panel capture can read it (feature-gated; emergency_reset unchanged) | REVIEW-20261031-1930 F-1 fix (root) | Kernel build with qcom-dload-mode-clear; QEMU gate serial shows the hold line and the reset still fires (elapsed +~5 s) |
| 0040 | kreset: opt-in PON reboot-reason write over the SPMI arbiter v5 before the PSCI reset (feature qcom-pon-reboot-reason, default OFF; env REDOX_PON_REBOOT_REASON; warn-and-continue; emergency_reset unchanged) | P4 (root; research p4-pon-research.md) | Host tests 14/14 (rustc --test on the module); kernel build with the feature; 8 GiB gate non-regression (QEMU has no arbiter model); write proven only at the attended device run |
| 0041 | PON: refuse (sid, register) pairs the arbiter encodings cannot represent (validate_target before any bus I/O); preserve FDT region spans and prove every arbiter offset against them up front (ArbBounds, capped discovery, channel-extent check) | P4 correction (root; monitor review 044647e) | Host tests 26/26 incl. the reviewer's two controls RED on 0040 / GREEN on 0041; kernel build with the feature; 8 GiB gate non-regression; write proven only at the attended device run |
| 0042 | qcom SCM: clear the download-mode cookie during boot (qcom_scm::init, feature qcom-dload-mode-clear), not only at reset - a watchdog bite, panic, or power cut then lands in the normal boot instead of CrashDump Mode | P3 F-5 (root; REVIEW-20261031-2230) | Kernel build with the feature; QEMU gate serial shows the boot-time 0038 line or its warn; device proof at the attended P3 re-run (bite lands in pmOS) |
| 0043 | APSS watchdog handoff (feature qcom-wdt-handoff, default OFF): FDT-discovered KPSS watchdog (watchdog@17980000); at the end of device init, disarm by clearing the WDT_EN enable bit (RMW + readback) or leave armed when REDOX_WDT_MODE=leave (bootloader 0006) | P3 F-4 (root; REVIEW-20261031-2230) | Host tests 9/9 (layout selection, env parse, EN math, disarm/leave); kernel build with the feature; QEMU gate serial handoff line; device proof at the attended P3 re-run |
| 0044 | Build fix: iterate the fdt crate's StringList directly (it is itself the iterator; no .iter()) in the qcom_pon and qcom_wdt probes' kernel glue | Build correction (root; first full-series kernel build) | Kernel build with qcom-dload-mode-clear + qcom-wdt-handoff compiles; 8 GiB gate |
| 0045 | Watchdog handoff placement fix: run qcom_wdt::handoff() from arch start() after args.bootstrap() (it reads init_env(), which panics before BOOTSTRAP is installed), not at the end of init_devicetree | Gate correction (root; first P3-fix gate panic) | Kernel build with the features; 8 GiB gate serial shows the handoff line and a clean boot to shell |
| 0046 | Watchdog handoff placement fix #2: run qcom_wdt::handoff() from kmain immediately after BOOTSTRAP.call_once (args.bootstrap() only builds the value; the Once installs inside kmain) | Gate correction (root; P3-fix gate round 2) | Kernel build with the features; 8 GiB gate serial shows the handoff line, no panic, clean boot to shell |
| 0047 | Watchdog probe region validation (WDT-BOUNDS-01): WdtRegion::new refuses a non-word-aligned reg base and a wrapping base + span (checked_add), span minimum folded into the same entry, mirroring 0041's ArbBounds discipline; refusal skips the node before any MMIO | Monitor audit 327db53; root disposition 3926e33 | Host tests 13/13 (9 existing + 4 new); mutation RED / restore GREEN; apply-check on pristine through-0046 tree; kernel build + 8 GiB gate in the P3-fix rebuild |
| 0048 | PON probe slave-ID parse: read a pmic's reg as a U32List and take the FIRST cell via pure sid_from_cells (the SDM845 SPMI arbiter's #address-cells = <2> makes reg 8 bytes; the single-u32 parse failed on length and the probe silently skipped every pmic) | A1 gate-serial finding (root, 2026-11-01) | Host tests 27/27 (1 new); mutation RED / restore GREEN; host replica of the probe walk vs the pinned DTB; gate serial must show the init line and an arbiter-stage refusal under QEMU |

The base-side `patches/integration/base/0011`–`0013` and `patches/touch-e2e/base/` remain separate because they patch a different upstream repository. The bootloader series remains separate too.

The separately preserved experimental-irq-owned-dispatch/prototype.patch is NOT
part of this series and must not be applied by integrate.sh. It is an unwired
generic registry/runner experiment without real controller adapters; its host
tests do not establish production migration or runtime equivalence. See its README.
