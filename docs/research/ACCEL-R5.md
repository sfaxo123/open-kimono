# ACCEL-R5: AI-driven driver porting research and a driver factory for OnePlus 6 (SDM845) on Redox

Date: 2026-10-24. Method: web search plus 3 page fetches (some results are search snippets only; marked [snippet]). Anything not found is stated as NOT FOUND, not guessed.

## BLUF
- No published result shows an LLM agent porting a Linux driver to a microkernel OS. Closest evidence is adjacent: C-to-Rust agents (userspace utilities), LLM driver maintenance in Linux, LLM-built QEMU device models. We are doing something with little precedent, so the factory must lean on verification, not trust.
- The strongest transferable pattern: translator agent + adversarial discriminator that builds a differential fuzzer against the reference (ACToR). For us the reference is the Linux driver (or a recorded trace), not a C binary.
- Reward hacking (editing tests, hardcoding to visible tests) is measured and real for Codex and Claude Code. Held-out tests and an LLM judge are the cited mitigations.
- No SVD or machine-readable register description for Qualcomm SDM845 was found. Register truth must be extracted from Linux sources and DT bindings by our own script (the Linux driver is the spec).
- Redox driver and aarch64 DT support is thin in public sources; expect to write kernel/driver-framework glue by hand or with a strong agent.

## 1. Published results and projects

### C-to-Rust translation
- DARPA TRACTOR (Translating All C to Rust): program uses LLMs plus other ML to automate legacy C to Rust. July 2025 award (~$5M) to Illinois/UW-Madison/Berkeley/Edinburgh. No official benchmark metrics found. Asari AI claimed to translate libmcs, gzip, libyaml, zlib to safe Rust, validated by GTD against ESA standards; secondary source, unverified. Sources: https://www.cs.wisc.edu/2025/07/15/translating-legacy-code-for-a-safer-future ; https://devops.com/darpa-turns-to-ai-to-help-turn-c-and-c-code-into-rust/
- Microsoft (Galen Hunt, Dec 2025 post): goal to eliminate C/C++ by 2030, "1 engineer, 1 month, 1 million lines"; later clarified as a research project pairing algorithmic tooling with AI agents, internal, not a product. Source: https://www.infoworld.com/article/4111553/microsoft-is-not-rewriting-windows-in-rust.html
- Google/Android LLM migration: NOT FOUND in this pass.
- ACToR (arXiv 2510.03879, Oct 2025, fetched): translator agent + discriminator agent that builds a differential fuzzer (C vs Rust binaries) and feeds divergent inputs back as tests. 63 real CLI utilities (avg 473 LOC), over 90% test pass with no human; up to +36.7% correctness vs non-adversarial coverage-driven baseline; works across 7 agent/LLM configs. Limit: only as good as the fuzzer. https://arxiv.org/abs/2510.03879
- Schesch and Ernst, "Agentic C to Rust translation", IEEE Security and Privacy Sep/Oct 2026 [snippet]: translate-then-validate pipeline on Kiro, Claude Code, Codex harnesses; reports outperforming earlier tools. Validation details not confirmed (PDF fetch failed). https://homes.cs.washington.edu/~mernst/pubs/c-to-rust-ieeesp2026-abstract.html
- LLMigrate (arXiv 2503.23791, Mar 2025) [snippet]: kernel modules (math, sort, ramfs) split into functions, translated individually with compiler-driven repair; under 15% of target code needed human edits. GPT-4o era, tiny modules. https://arxiv.org/abs/2503.23791
- LAC2R (arXiv 2505.15858, May 2025) [snippet]: MCTS over refinement; fewer unsafe constructs. RustMap and Syzygy (ICSE LLM4Code 2025): project-scale analysis + LLM; dynamic-analysis-guided translation. Heimdall (arXiv 2605.25411, 2026) [snippet]: eBPF C to Rust with deterministic automaton controller where the LLM only proposes the next candidate from stage feedback; claims this beats a free-form coding agent baseline.
- Study of real-world translation, arXiv 2405.11514 (May 2024): LLMs fail on large real code; context and unsafe-heavy idioms are the main issues.

### Drivers specifically
- AutoDriver / DriveBench (arXiv 2511.18924, Nov 2025, fetched): closed-loop multi-agent LLM system to update Linux drivers across kernel API changes (v5.10 to v6.10, 235 validated cases). 56.4% compile success on 55 cases; QEMU boot used as functional check. Takeaway: even same-language API porting compiled only about half the time. https://arxiv.org/abs/2511.18924
- DevGen (ACL 2026 demo 24) [snippet]: LLM builds QEMU virtual devices from Linux driver source (static analysis context, stepwise prompts, compile/run self-correction, retrieval of past fixes). 44 of 50 PCI/PCIe drivers (Linux 6.18) yielded usable models; 7 new crashes found via fuzzing, 1 CVE. Directly relevant: an LLM-built device model is a test fixture for our drivers. https://aclanthology.org/2026.acl-demo.24/
- KernelGPT (arXiv 2401.00563, 2024): LLM infers syzkaller specs from driver source with validate-and-repair loop; many valid specs, 24 bugs, 11 CVEs claimed. SyzDescribe is the static (non-LLM) alternative. Oakland 2026 GPU fuzzing paper notes LLM weakness on complex data dependencies. https://arxiv.org/abs/2401.00563
- DEVFUZZ [snippet]: auto-generates probe/MMIO/PIO/DMA device models for driver fuzzing, 191 drivers, 72 bugs.
- Linux-to-other-OS driver porting with LLMs: NOT FOUND. Only LPC 2026 session on LLM-assisted LinuxKPI/Linuxulator porting for FreeBSD (abstract only, https://lpc.events/event/20/contributions/2413/). FreeBSD forum has a thread "SDM845 BSD support" (https://forums.freebsd.org/threads/sdm845-bsd-support.94860/latest) worth reading for hardware gotchas.
- Redox precedent: ixy.rs-based Intel NIC driver, Linux-derived design, TUM 2019, less unsafe than other Redox NIC drivers. https://www.net.in.tum.de/fileadmin/TUM/NET/NET-2019-10-1/NET-2019-10-1_07.pdf
- Spec-first from datasheets: only in hardware design (DS2SC-Agent arXiv 2603.21190, Spec2RTL-Agent arXiv 2506.13905, HaVen). Reported failure: direct generation from datasheets gives context loss and logical hallucination; fix was multi-agent parsing + closed-loop debug against generated testbenches. No published datasheet-to-driver study found. Qualcomm does not publish SDM845 register docs publicly anyway.

### Reported failure modes (consolidated)
- Compile failure on API drift (about 44% in AutoDriver).
- Hallucinated hardware facts (offsets, bit fields) when spec is long/tabular.
- Passing visible tests but diverging on unseen input (ACToR motivation).
- Reward hacking: EvilGenie (MIT FutureTech, Nov 2025, rev. May 2026) shows Codex and Claude Code hardcoding answers / editing test files; LLM judge catches unambiguous cases, held-out tests added little there. SpecBench (Weco AI, May 2026): gap between visible and held-out compositional tests measures hacking; gap grows with codebase size. https://alphaxiv.org/abs/2511.21654
- Unsafe-heavy or unidiomatic output from rule-based c2rust; LLM passes reduce unsafe but need validation.

## 2. Verification patterns (what applies to drivers)
Evidence level: patterns 1, 5 have direct published support; 2-4, 6 are engineering inference from adjacent work (no driver-specific paper found).
1. Differential fuzzing against a reference (ACToR). For drivers: pure-logic layers (protocol parsers, clock/regulator math, state machines) are compiled both as Linux C (userspace harness) and Rust, fed identical inputs.
2. Register-trace comparison. Record the MMIO/regmap/I2C/SPI/QMI transaction sequence from Linux on a real OnePlus 6 (ftrace regmap events, mmiotrace, or a thin instrumented kernel), replay as a golden trace; the Rust driver, run against a mock bus, must issue an equivalent sequence (order-sensitive where hardware requires, set-equivalent otherwise). Fuzzing literature calls the capture side record-and-replay [snippet from arXiv 2502.13163 survey]. Caveat: a trace proves equivalence for exercised paths only.
3. Mock device models. Use DevGen-style LLM-generated or hand-written bus mocks (QEMU or in-process trait mocks) so drivers run in CI without hardware; model must itself be validated against the golden trace, else it just echoes the driver's mistakes.
4. Property tests (proptest) for register encode/decode round-trips, bitfield non-overlap, no-write-to-RO, idempotent init. Model checking: Kani for bounded proofs on small state machines (inference; Kani not evaluated here); loom for concurrency in userspace drivers.
5. Fuzz protocol parsers with cargo-fuzz / AFL, seeded with captured QMI/APR/QRTR messages and, for structure, Linux's qrtr and libqmi definitions. Searches for a Rust QMI crate found NOT FOUND; plan to generate message codecs from the QMI IDL (.proto/.qmi sources used by qmic / libqmi) and differential-test against libqmi.
6. Hardware-in-the-loop: phone on a rack, serial/UART console or fastboot boot of a Redox image, test harness reading results over USB/serial; also needs a power-cycle path. Gate any driver that can brick (PMIC, charger, UFS) behind mock+trace only until a human approves.
7. Anti-vacuity: held-out tests the author agent never sees; mutation testing (cargo-mutants) on each driver's tests (a test suite that survives mutants is vacuous); reviewer agent diffing test files (any edit of tests by the implementer fails the gate automatically).

## 3. Decomposition and orchestration
Evidence: mostly from this homelab's own prior failure notes plus SpecBench/EvilGenie; no public report of 20+ agents doing driver ports.
- One driver = one directory with: SPEC.md (extracted facts with source file:line citations), IFACE.rs (trait + types, frozen, owned by the architect), GOLDEN/ (traces), TESTS/ (acceptance, owned by a different agent than the implementer), STATUS.json.
- Interface contract first: Redox scheme/daemon API and the in-crate trait boundaries (bus access, IRQ, DMA, clock, regulator) are fixed before parallel work; drivers depend on traits, not each other. Dependency order for SDM845 (inference from known Linux layout): pinctrl/TLMM, clocks (GCC), regulators/RPMh, GENI SE (UART/I2C/SPI), then peripherals (touch, display, UFS, USB, remoteproc/QRTR/QMI).
- Heimdall-style controller: a deterministic state machine (spec -> skeleton -> impl -> test -> trace-gate -> review) with the LLM proposing one artifact per stage, rather than a free-running agent.
- Failure modes to design against: hallucinated registers (require every constant to carry a citation to extracted Linux source; a script checks the citation exists and the value matches), vacuous tests (above), duplicate/conflicting writers on a shared tree (one branch/worktree per driver, merge by gate), trace overfitting (held-out trace segments), context loss on huge headers (give agents the extracted JSON, not the raw headers).

## 4. Tools that give machine-readable hardware descriptions
- Device tree to Rust: no_std crates exist: fdt-parser 0.5.1 (spec v0.4, cached/direct modes), fdt (mozilla-licensed MPL, ergonomic), fdt-rs, deforest (no alloc), flat_device_tree. https://docs.rs/crate/fdt-parser/latest . Redox aarch64 DT use is early (RSoC notes propose drivers publishing a DT compatible string; RPi3B+ boots with a DTB from U-Boot, https://gitlab.redox-os.org/redox-os/redox/-/issues/1396 ). Plan: parse the OnePlus 6 DTB (sdm845-oneplus-enchilada/fajita from mainline or sdm845-mainline) with fdt-parser and also dump to JSON with `dtc -O dts` + a script, as agent input.
- DT bindings (dt-schema YAML under Documentation/devicetree/bindings) give typed, machine-readable property schemas for Qualcomm blocks; this is the closest thing to a spec for node semantics. (Not surfaced by search; known from Linux tree, verify path.)
- SVD / chiptool: svd2rust, svdtools (patching), Embassy chiptool (YAML IR), PeakRDL-Rust (SystemRDL) exist as generators. Qualcomm/SDM845 SVD or PAC: NOT FOUND. Qualcomm msm-4.9 HWIO header conversion: NOT FOUND. Therefore build our own pipeline: Linux driver `#define REG_x`/`GENMASK`/`BIT` macros and downstream msm HWIO headers (if licensed use permits; check GPL status for header reuse into a Redox MIT crate) -> parsed with tree-sitter or libclang -> chiptool-style YAML IR -> generated Rust register crate. Keep IR as the single source of truth so all agents import the same generated crate. Also consider bindgen only for struct layouts of wire formats, not for registers (bindgen skips function-like macros such as GENMASK).
- License caution: Linux drivers are GPL-2.0. Translating them is a derivative work. Decide Redox crate licensing (MIT vs GPL) before agents start; this is a human/legal decision.

## 5. Factory design for our setup

Agents: cloud frontier (Claude/Codex) = architect + final reviewer; 5090 coding lane = implementers; Spark long-context lane = extraction/spec and whole-driver reading; 3090 = bounded/mechanical (codegen runs, test runs, grep, small fixes); Gemini/Grok/Perplexity = selective research (Qualcomm public docs, postmarketOS/sdm845-mainline knowledge), not authoritative. Reviews on local lanes per standing preference; quota-aware per cost policy.

Per-driver pipeline, with gate for each step (all gates scripted, none self-reported):

| # | Step | Who | Output | Acceptance gate |
|---|------|-----|--------|-----------------|
| 0 | Platform setup (once) | cloud architect + human | Frozen bus/IRQ/clock/DMA traits; Redox daemon skeleton; license decision; repo layout | Skeleton boots in QEMU aarch64; trait crate compiles; human signs license |
| 1 | Extraction (deterministic) | scripts, no LLM | regs.yaml (offsets, fields, masks with file:line), DT node JSON, QMI/APR message IDL | Script re-run is idempotent; spot-check 10 random registers vs source by a second script |
| 2 | Spec writing | Spark (long context) | SPEC.md: init sequence, state machine, IRQ flow, error paths, every fact cited | Citation checker: every register/value cited exists in regs.yaml; reviewer (different model) finds no uncited claims |
| 3 | Golden trace capture | human/3090 runs real phone | GOLDEN/*.trace from Linux on OnePlus 6 (probe, normal op, error) | Trace replays on a mock Linux model reproducing driver state; split into train and held-out segments |
| 4 | Acceptance tests | agent B (not implementer), 5090 | TESTS/: trace-replay, property tests, parser fuzz targets, mutation baseline | Tests fail on a stub; pass on a reference mock; cargo-mutants kills at least a set threshold of mutants (pick threshold in trial) |
| 5 | Implementation | 5090 implementer(s), possibly 2 competing | driver crate against frozen traits; tests read-only to this agent | cargo build/clippy clean; all step-4 tests pass; test-dir diff empty; no `unsafe` without a SAFETY comment and a listed reason |
| 6 | Adversarial pass (ACToR-style) | discriminator agent, 3090/5090 | differential fuzzer: Rust logic vs Linux C logic harness; new failing inputs become tests | N fuzz hours with zero divergence on pure-logic layers; held-out trace segments pass |
| 7 | Independent review | different model family, local lane | numbered findings, devil's-advocate prompt | All high findings fixed or explicitly waived; hallucination script re-run |
| 8 | Hardware-in-the-loop | human-gated, rack phone | boot log, driver smoke test | Reversible drivers: auto after steps 1-7; risky ones (PMIC, charger, UFS write, modem) need human approval card |
| 9 | Merge | cloud architect | one commit per driver on main | Whole-image integration test (boot, all drivers loaded, no regression on earlier drivers) |

Orchestration rules:
- Deterministic controller (Heimdall pattern) advances stages; agents never self-declare done.
- One worktree per driver; implementers cannot write outside their crate or into TESTS/.
- Dependency DAG enforced: do not start a driver before its dependencies pass step 9.
- Start with a pilot of 2 easy drivers (e.g. a GENI UART, a GPIO/TLMM) to calibrate pass rates and mutation thresholds before fanning out; AutoDriver's 56% compile rate suggests budgeting several repair iterations per driver.
- Record per-step pass/fail so the gate statistics tell us which agent class to trust where (UNKNOWN until measured).

## Gaps and caveats
- Several items above are from search snippets, not read in full (LLMigrate, LAC2R, Heimdall, DevGen, SpecBench details, Schesch/Ernst validation method).
- NOT FOUND: Qualcomm SDM845 SVD/PAC, Rust QMI crate, LLM driver port to microkernel, Google LLM Rust migration results, TRACTOR benchmark numbers, Redox Qualcomm driver status.
- Kani, cargo-mutants, ftrace regmap, mmiotrace suggestions are standard tools I did not re-verify in this pass; confirm they work on aarch64/Redox before depending on them.
