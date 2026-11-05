# Rust migration scope and ownership ledger

## GENI UART registration/state contract, partial inventory — 2026-10-27 5:42 PM PT

Pinned QEMU c51d86c7c5d6a0e980a3e96050ec68229b5cbe1e. Root directly rechecked
hw/char/qcom-geni-uart.c726–841, include/hw/char/qcom-geni-uart.h17–57,
hw/char/meson.build13 and Kconfig63–64 after the Spark packet. Type
`qcom-geni-uart` derives from SYS_BUS_DEVICE; 0x4000 MMIO, little-endian,
valid access width exactly4bytes, one sysbus IRQ, two16word FIFOs and a
`chardev` property. Reset clears registers/interrupt state/FIFOs/command state,
sets SE_IRQ_EN_ALL and recalculates IRQ. Realize installs receive handlers;
finalize destroys FIFOs. Migration v1 records registers, interrupt enables and
status, active command/length fields, FIFO contents and RX-tail metadata.

These are source compatibility obligations, not physical register correctness,
Rust implementation or save/restore runtime proof. IRQ recomputation after
load and external pending-character behavior require separate investigation;
the field list alone does not establish migration correctness. Full register
semantics, board wiring, upstream device coverage and phase1 remain OPEN.
Original RUST-CHECK-20261028T0037-UART-RESULT.md exceeded its250word budget;
only the independently rechecked subset above is adopted.

## OnePlus6 board registration, partial inventory — 2026-10-27 5:04 PM PT

Pinned QEMU c51d86c7c5d6a0e980a3e96050ec68229b5cbe1e, source-only. Root rechecked oneplus6.c361-386, hw/arm/meson.build3-8 and hw/arm/Kconfig41-56 from the Spark packet RUST-CONTINUE-20261028T0000-BOARD-RESULT.md. Class registers machine oneplus6 through oneplus6_info/type_register_static, init oneplus6_init, with configured default cortex-a76, one CPU/max8 and4GiB RAM. These are emulator defaults, not proof of physical CPU identity/fidelity or device RAM availability.

CONFIG_ONEPLUS6 conditionally builds oneplus6.c, oneplus6-cmd-db.c and oneplus6-smem.c; generated DTB-path header creation is outside that conditional in the inspected Meson file. Kconfig defaults y, depends TCG && ARM and selects ARM_GIC, DEVICE_TREE, UNIMP, QCOM_GENI_UART, QCOM_TLMM, QCOM_PDC, QCOM_SPMI_PMIC_ARB, QCOM_GENI_I2C, RMI4_S3706, QCOM_UFSHC, QCOM_SMMU500 and QCOM_GDSC. Full transitive select closure, individual device registrations/semantics and upstream machine coverage remain OPEN.

Root rejects the packet's unsupported physical big.LITTLE description and does not accept its peripheral paths labeled untraced as verified closure. Packet exceeded requested250words and expanded beyond three named files; only the directly rechecked subset above is adopted. No build/runtime/authoring acceptance; next source packet should trace one selected custom device type, not claim the whole machine complete.

## QEMU target-profile semantics, partial inventory — 2026-10-27 4:51 PM PT

Pinned QEMU c51d86c7c5d6a0e980a3e96050ec68229b5cbe1e source, no build or emulator execution. Spark packet QEMU-PROFILE-RESULT.md under artifacts/handoff/redox-perplexity exceeded its requested300-word budget; root accepts only the following directly rechecked core findings. This is not full platform/machine inventory or phase1 acceptance.

- meson.build3317 loads target `.mak` settings;3346 requires TARGET_ARCH,3347-3349 defaults absent TARGET_BASE_ARCH to TARGET_ARCH,3353-3355 defaults absent TARGET_BIG_ENDIAN to n. Explicit profile values survive these defaults. TARGET_LONG_BITS has no default in this block;3384 emits generic numeric settings.
- configure792-820 gates user profiles on host architecture/OS and mode,823-828 constructs defaults,1094-1114 validates explicit target selection. Meson3288-3341 can skip/reject profiles for OS, accelerator or FDT availability. A static list of files is not actual build coverage.
- Meson3400-3413 separately combines device defconfig, Kconfig, host and accelerator settings through minikconf. Machine/device enumeration therefore remains a separate required axis. configs/targets/meson.build lists four TargetInfo C sources, not four supported machines or the whole architecture set.

Original report and raw worker history remain evidence; unreviewed peripheral consumer claims are not adopted. Next inventory packet should enumerate one bounded device-registration family with exact source provenance, then accumulate into the full inventory. No boot/emulator replacement authoring is admitted.

## Selected hashing dependency boundary — 2026-10-27 4:01 PM PT

Identity slice 94162da/4d6dfa6 uses already cached sha2 =0.10.9, default-features=false, force-soft. Exact new versions/checksums are locked; no existing dependency pin was bumped or downloaded. This is a narrow selected-path review, not a whole dependency-security or Rust-only-runtime acceptance. Full source evidence and corrected Spark/root reviews: RUST-CHECK-20261027T2247-* in the existing artifact handoff.

| Boundary | Accepted observation / unresolved requirement |
|---|---|
| SHA256 backend | force-soft selects Rust software compression before architecture-specific backends; sha2-asm is not enabled. Reaudit any feature/pin change. |
| Unsafe Rust | private compress256 remains reachable despite compress public reexport being off. GenericArray u8/U64 layout and block-buffer cursor/splitting invariants matter. Root inspected these and tests concrete size64/alignment1; broader unsafe soundness is not proven. |
| Target-native dependency | cpufeatures is still compiled on x86/aarch64, and libc stays in the ARM graph. ARM cpufeatures contains getauxval FFI even though selected SHA software code does not reference it. Never infer a dependency disappears because a consumer does not call it. |
| Build host | generic-array/version_check probes rustc; compiler/stdlib/allocator/unwind/host contracts remain OPEN. |
| Object evidence | x86_64/aarch64 brain and SHA release objects lacked getauxval/sysctlbyname/sha2_asm imports; ARM cpufeatures had getauxval. Not a final whole-program elimination or phone execution proof. |
| Manifest trust | identity comparison assumes an out-of-band trusted manifest. Hash equality is integrity matching, not signature/authenticity verification or secure file opening. |

Root rejected the audit's initial claims that force-soft removes cpufeatures from Cargo or removes the private compression cast. Spark corrected these within its original cutoff. Native Perplexity's manifest-string resource-gap finding was useful and implemented; its assertion that validation ends at hashing was rejected. Full upstream inventory and native-retirement gates remain OPEN.

## Reviewed recovery-capture source slice — 2026-10-27 3:08 PM PT

Static source only: scripts/phone/capture-recovery.sh at worktree2b14f5a and matching canonical SHA256 bb83038d11108cd08b3fe7e82294fdd6b8788cd778519c7c1bdeb4107695d068. Spark's15-entry JSON is retained under RUST-CHECK-20261027T2152-RECOVERY-CONTRACT.json in artifacts, with ROOT-REVIEW.json qualifications. It is partial inventory, not a completed phase1 graph or execution proof.

| Source lines | Established contract / unresolved Rust replacement requirement |
|---|---|
|1–7|POSIX shell, fail-fast settings, umask077, required first argument, prefix path rejection, existence check then mkdir; no atomic exclusive directory claim or exact-one-argument enforcement.|
|8–16|Reads28 named partitions and first/last256KiB of six disks; local artifact writes. No device identity/ownership check, timeout, size guard or cleanup handler. Future tests require native Rust fake transports, not actual device contact.|
|17–20|lsblk, ls, cat and tar metadata/boot capture dependencies; failures are not a tolerated-missing-file contract. Native Rust replacements required; wrapped legacy utilities are not an accepted result.|
|21–24|SHA256 covers img/bin/tar, omits text/dd logs; chmod and terminal success marker. Complete/partial manifest, private retention and recovery proof remain contracts to implement/test.|

No SSH/network invocation exists in this24-line script itself; outer transport and privilege context remain outside this slice. Do not accept raw graph proposals of wrapped lsblk/ring or unproved symlink exploits. No script/device command was executed. Full upstream architecture/platform/feature inventory and phase1 gate remain OPEN.

This is an inventory/contract ledger; docs/PLAN.md remains the only execution plan. Operator explicitly selected ALL upstream U-Boot and QEMU features/platforms, plus every custom feature, with Rust-only new implementation/tests. Findings collected by three bounded Luna subagents on 2026-10-27; no native Rust boot/emulator replacement exists yet.

## Confirmed full-upstream scope and measured revisions

U-Boot: local snapshot `3e15c2819caa7e14e1e4b2ff758a41cbc58a7311`, based on upstream `8649a54d301a4d0ac526324c7a7ad2574939ff27`; Makefile version2026.10, release tag not independently verified. Git inventory has39,195 paths,6,974 C files,6,576 headers,244 assembly files,425 Python files and no Rust files. Twelve architecture ports and1,542 defconfigs are enumerators, not proof each builds. Drivers cover77 first-level subsystem families, board tree213 vendor/family directories, tools877 tracked paths and test471. Downstream overlay is3files/195insertions at this snapshot; embedded-EFI builder overlays are a separate custom source surface.

QEMU: local snapshot `c51d86c7c5d6a0e980a3e96050ec68229b5cbe1e`, VERSION11.1.2. Import `b5de3fe` records upstream `4fc49f46dc95d4a27de2509e7fceb2931e91faeb`; that import provenance is a source-record claim, not a newly fetched upstream comparison. Git inventory has11,296 tracked paths,4,112 C files,2,827 headers,426 Python files and70 Rust files. There are27 target architecture directories and67 build profiles (29system,33Linux-user,5BSD-user), not67 machines. Exact total machine coverage still needs semantic enumeration. Tests contain3,612 paths. Local overlay after import is60commits/95changedfiles, net16,008insertions/97deletions; all custom devices and shared-UFS changes remain in scope.

Completion requires all upstream architecture/board/device/command/service/configuration rows plus downstream overlays and every project-built capability. Inventory all system/user emulation, TCG/JIT and host accelerator contracts, QMP/QAPI/QOM/HMP/GDB/plugins, migration/VMState/replay, storage/network/character/display/audio/input backends and host tools/build/tests. U-Boot includes every architecture/board/driver, CLI/environment, image/filesystem/network/security/EFI/sandbox feature and host build/generator/test system. A source count, representative target, existing upstream Rust module or successful OnePlus guest cannot pass this gate. Unsupported Rust compiler/host/platform support must remain an explicit blocker. Track upstream deltas after the chosen baseline rather than silently moving it.

## Measured baseline

Project main1263b74 has228 tracked.rs,30.py,35.sh and74.patch files, plus docs/data/build configuration. No standalone tracked.c/.h or build.rs was found in this checkout; that does not mean the complete stack is C-free. Python embeds op6efi C/assembly, patch payloads modify external code, and external U-Boot/QEMU are substantial C/assembly trees. Counted files are not an implementation-completion percentage. Generated/extensionless files and transitive dependencies remain inventory work.

| Surface | Current source/baseline | Required Rust result | Current status |
|---|---|---|---|
| Kernel/userspace/EFI app | Existing Rust, kernel series31 / controlled23+RX runtime | Retain Rust; audit unsafe/FFI/ownership rather than translate again | Historical accepted evidence preserved |
| Trace converter | scripts/oracle/trace/convert.py + tests | Native Rust CLI with full contract parity and Rust tests | Candidate tools/rust-migration; known parity gaps, no caller switch |
| Remaining host tools |30.py/35.sh; enumerate every path and embedded language | Rust validators, generators, packager/control and tests | Inventory batches pending |
| Current embedded U-Boot path |3e15c2819caa7e14e1e4b2ff758a41cbc58a7311, scripts/phone/build-embedded-efi.py, baseline.config | Native Rust boot stage, image format, memory/DTB/framebuffer/console/kernel-initfs handoff and required service parity | Architecture gate pending |
| Other boot variants | patches/u-boot includes older8649a54d base and fastboot-first behavior | Explicitly include/retire each variant by frozen matrix; no accidental version substitution | Reconcile in phase1 |
| Emulator | qemu-oneplus6 c51d86c7c5d6a0e980a3e96050ec68229b5cbe1e, hw/arm/oneplus6.c | Rust CPU/memory/MMU/EL/IRQ/timer/device/host runtime, no QEMU library/subprocess | No implementation yet |
| Firmware/assembly | PBL/XBL/ABL, entry/exception/trampoline assembly | Replacement feasibility or explicit operator-approved boundary | UNRESOLVED; not exempted |
| Build/toolchain/native deps | Rust compiler/LLVM/linker/libstd/system libs, dtc, image tools, Meson/Ninja, Cargo native crates | Rust project tooling plus approved external-dependency contract | Audit pending |
| Shared homelab/vendor agents | Existing Perplexity/DSH/dispatch/transport interfaces | Explicit owned-project versus external-service graph; no silent source copying | Boundary pending |
| Brain specialization | Sole pinned LiquidAI/LFM3-1.7B revision40ebe0aa6662c9d40389c2f5d565b6a2c6aebacd; actual-use corpus not yet curated | Native Rust supervised LoRA first: data validation/splits, gradients/frozen base, bounded host training, adapter loading/export and held-out promotion/rollback | Mandatory PLAN phase8; no trainer/dataset/adapter/fit proof yet; training backend/native boundaries unresolved |

## Boot contract decomposition

The old path is Qualcomm firmware → Android RAM container → C/asm U-Boot → Rust EFI loader → Rust kernel/initfs. Replacing only the EFI app leaves the C boot stage intact. Specify host image/container parsers, Rust ARM64 entry/memory/relocation, FDT and framebuffer ownership, console, kernel/initfs validation, handoff state and recovery protocols separately. Decide whether EFI services are implemented or replaced by a direct reviewed ABI. Fastboot/USB/clock/pinctrl/SMEM/RPMh/GOP/environment/menu features require an explicit matrix. Simulated/fake recovery transport only while phone STOP applies.

## Emulator contract decomposition

Own all upstream instruction execution/translation, architectural state, memory/MMU/exception/interrupt/timer machinery, board/device models and host interfaces. The OnePlus subset includes DTB/image loading/PSCI, UART/framebuffer, TLMM/PDC, SPMI/RPMh, GENI I2C/RMI4 touch and UFS. Existing stub/partial behavior must be identified, not relabeled complete. Audit any proposed Rust engine for architectural and native-dependency coverage. Full upstream parity was SELECTED, so all other architectures/machines/devices/interfaces must also be enumerated and implemented; the subset cannot pass the final gate.

## Accepted Rust foundation increment — 2026-10-27 1:10 PM PT

Pinned LFM3 config validation now exists in crates/lfm3-brain/src/model.rs, integrated46db6c7 after full two-file review against worker base2b14f5a. The independent official-config fixture/API check isd72f4b1. Model+service+fixture tests16PASS; numerical kernels/pretrained execution remain absent. This does not accept0.1.1, source inventory gates, model serving or Rust boot/emulator parity.

## Numerical foundation and dependency audit — 2026-10-27 2:09 PM PT

Kernel reference packet06420c8+b514b4c integratedbdecb09/91c61f6. Scalar matvec/RMSNorm/softmax/sigmoid/SiLU, default zero-tail split-half RoPE and single-channel six-past conv7 now exist, with15 analytic/negative tests. Combined model/service/numerics/public-config-fixture suite31PASS. Root mutation receipts show doubled RoPE exponent and early-f32 SiLU each fail their actual regression, then exact source restoration returns GREEN. No packed/multichannel/full-model/precision parity claim.

Root found unmangled host cos/exp/pow/sin calls despite std-only source. Replaced production math with already cached libm0.2.16 (MIT), exact Cargo.lock checksum, default-features=false and force-soft-floats. Its Rust source, build.rs/configure and selected math routines were inspected; release crate object now references Rust libm symbols and has no unmangled cos/exp/pow/sin/sqrt/floor imports. Metadata has13 packages including this crate, no links declarations and no C inference engine. Rust build-script rustc probes remain build-time tools. This is a scoped dependency audit, NOT proof the whole host executable/toolchain/allocator/unwind is C-free; those broader phase1/6 boundaries remain unresolved. No downloads or existing pin bumps.

Parallel Spark boot-packager graph packet f99494a6 reached its original300second cutoff, verified paused/running=false with no finished graph. Its first-finding note was later recovered under the unexpected generic artifact path INVENTORY-RESULT.md; root rehashed the source and verified specific path:line dependencies, rejecting the claim that all calls funnel through run(). This is partial source inventory, not a completed graph or gate. Preserve worktree/history and do not resume the expired goal. Next inventory attempt needs a smaller, source-only context packet on an admitted faster lane; no widening the cutoff or retrying the identical whole-handoff read.

## Owners and outcomes

- Root: canonical docs, integration, acceptance and exclusive physical ownership. Phone STOP.
- /root/rust_boot_scope: boot inventory and adversarial plan review COMPLETE. Found stale handoff, missing acceptance map, U-Boot baseline ambiguity and underspecified phase3/6 contracts; root reconciled in current docs.
- /root/rust_emulator_scope: emulator inventory and architecture/parity analysis COMPLETE. No Rust emulator engine found; large CPU/MMU scope retained explicitly.
- /root/rust_tooling_scope: inventory COMPLETE; first isolated Rust trace converter candidate delivered only in tools/rust-migration. No other writers overlap. Known integer/regex/CLI gaps block acceptance; no production call sites changed.
- Existing Perplexity task local-E4oCbKIhq98oGmfGXUVMy remains the only local5090 coordinator. Reassign to this plan before further dispatch; old physical/SMP goals and autonomous wakeups are suspended.

No agent is authorized to rewrite all QEMU/U-Boot in one unbounded turn. Phase1 decomposes each milestone into small owned packets with failure-first acceptance, original deadline and live lane admission. Preserve foreign workers; DSH atomic-paused capability absence is not permission to bypass it.
