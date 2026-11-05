# K3 provenance

The following upstream MIT files were adapted in this branch. Their existing
copyright and license notices remain in `LICENSE`.

| Upstream URL and commit | Path | SPDX | Use |
| --- | --- | --- | --- |
| https://gitlab.redox-os.org/redox-os/bootloader.git @ `cc922ec0bb3f9dd31cce589bb513ab2f15ac9746` | `Cargo.toml`, `mk/aarch64-unknown-uefi.mk`, `src/main.rs` | MIT | Adapted for embedded kernel/initfs boot |

The RAM boot path is original K3 MIT code. No donor file was copied.
# K3 provenance

The following upstream MIT files were adapted in this branch. Their existing
copyright and license notices remain in `LICENSE`.

| Upstream URL and commit | Path | SPDX | Use |
| --- | --- | --- | --- |
| https://gitlab.redox-os.org/redox-os/base.git @ `d2ffc4ca46d68f83b33c1e929e68dd7a7409668c` | `Cargo.toml`, `Cargo.lock`, `Makefile`, `init/src/main.rs`, `.gitignore` | MIT | Adapted for the RAM-only init target and build |

`ramsh/`, `init.initfs.d/30_ramsh.service`, and
`init.initfs.d/90_ramboot.target` are original K3 MIT code; no donor code was
copied into them.

## K5 build inputs

No upstream source file was added to this repository. The ignored `out/phone/`
artifacts were built from these pinned local sources:

| Upstream URL and commit | Path | SPDX | Use |
| --- | --- | --- | --- |
| https://source.denx.de/u-boot/u-boot.git @ `8649a54d301a4d0ac526324c7a7ad2574939ff27` | `qcom_defconfig`, `qcom-phone.config`, U-Boot sources | GPL-2.0-or-later and per-file SPDX | Built U-Boot binaries; no source copied into this repo |
| https://github.com/torvalds/linux.git @ `5b0764a53235d3c77e1ed674e836e1b3fe1a4b1f` | `arch/arm64/boot/dts/qcom/sdm845-oneplus-enchilada.dts` and includes; `include/uapi/linux/input-event-codes.h` | GPL-2.0 | Compiled DTB; missing sparse-checkout header was read from the same Git commit into ignored build inputs |
| https://gitlab.redox-os.org/redox-os/bootloader.git @ `19dd2e5008bf8dfd01769c767f17ff115c45c124` | K3 `op6/ramboot` tree | MIT | Built embedded Redox EFI payload from a local archive in ignored `out/phone/` |
| https://gitlab.redox-os.org/redox-os/base.git @ `fd6e5e6339a44390ac85750bb1512b3b3b6cd636` | K3 `op6/ramboot` tree | MIT | Built RAM initfs from a local archive in ignored `out/phone/` |
| https://android.googlesource.com/platform/system/tools/mkbootimg @ Ubuntu `mkbootimg` package `1:34.0.4-1build3` | `mkbootimg` | Apache-2.0 | Build-only copy in ignored `out/phone/`; removed an unconditional import of optional GKI signing support absent from the package; only header v0 packaging was used |

## K7 SPMI button driver

| Upstream URL and commit | Path | SPDX/header | Use |
| --- | --- | --- | --- |
| https://github.com/OWNER/redox-oneplus6.git @ `bdd317609934719b247875b6f7d5b5829da93be9` | `crates/mmio-replay/{Cargo.toml,src/*}` → base `drivers/mmio-replay/` | MIT | Copied manifest and source for an independent base build; original file headers retained |
| https://github.com/OWNER/redox-oneplus6.git @ `226b26e26a30f1ae537d785be2e5f852344b68fc` | `crates/spmid/{Cargo.toml,src/lib.rs,tests/arbiter.rs}` → base `drivers/spmid-core/` | MIT | Copied K7 core and tests; only relative dependency location retained |
| https://source.denx.de/u-boot/u-boot.git @ `8649a54d301a4d0ac526324c7a7ad2574939ff27` | `drivers/spmi/spmi-msm.c` | BSD-3-Clause header | Spec-only reference for v5 register map and command format; no code copied |
| https://github.com/openbsd/src.git @ `20cb34434d36871c39729f7369062cba28fd06cb` | `sys/dev/fdt/qcspmi.c`, `qcpon.c`, `qcpmicgpio.c` | ISC headers | Spec-only reference for PON/GPIO status; no code copied |
| https://github.com/torvalds/linux.git @ `5b0764a53235d3c77e1ed674e836e1b3fe1a4b1f` | `arch/arm64/boot/dts/qcom/{sdm845.dtsi,pm8998.dtsi,sdm845-oneplus-common.dtsi}` | GPL-2.0 | Oracle facts only (window addresses, PMIC addresses, buttons); no code copied |
| https://gitlab.redox-os.org/redox-os/base.git @ `d2ffc4ca46d68f83b33c1e929e68dd7a7409668c` | `Cargo.toml`, `Cargo.lock` | MIT | Adapted workspace manifests for the new crates; no upstream source copied |

`drivers/spmid/src/main.rs` and `crates/spmid/` are original K7 MIT code. `drivers/spmid/src/main.rs` uses the existing base `inputd` and `common` interfaces; it is not a copy of upstream `exampled`.

## K8b simple framebuffer integration

| Upstream URL and commit | Path | SPDX/header | Use |
| --- | --- | --- | --- |
| https://gitlab.redox-os.org/redox-os/base.git @ `d2ffc4ca46d68f83b33c1e929e68dd7a7409668c` | `drivers/graphics/vesad/src/main.rs`, `Makefile`, `init.initfs.d/` | MIT | Adapted existing base files for DT framebuffer discovery and RAM boot service |
| https://github.com/torvalds/linux.git @ `5b0764a53235d3c77e1ed674e836e1b3fe1a4b1f` | `Documentation/devicetree/bindings/display/simple-framebuffer.yaml` | GPL-2.0 | Binding semantics only; no source copied |
| https://github.com/openbsd/src.git @ `20cb34434d36871c39729f7369062cba28fd06cb` | `sys/dev/fdt/simplefb.c` | ISC | Framebuffer format names only; no source copied |

## K9 GENI I2C and RMI4 touch

| Upstream URL and commit | Path | SPDX/header | Use |
| --- | --- | --- | --- |
| https://github.com/OWNER/redox-oneplus6.git @ `bdd317609934719b247875b6f7d5b5829da93be9` | `crates/mmio-replay/{Cargo.toml,src/*}` → base `drivers/mmio-replay/` | MIT | Copied existing C1 crate for independent base build |
| https://github.com/openbsd/src.git @ `20cb34434d36871c39729f7369062cba28fd06cb` | `sys/dev/fdt/qciic_fdt.c` | ISC header | Spec-only GENI FIFO/register reference; no source copied |
| https://github.com/freebsd/freebsd-src.git @ `81f86c9a9417d7ad7445d46739e19a26ab916e6b` | `sys/dev/iicbus/controller/qcom/geni_iic.c` | BSD-2-Clause header | Spec-only GENI FIFO/register reference; no source copied |
| https://github.com/torvalds/linux.git @ `5b0764a53235d3c77e1ed674e836e1b3fe1a4b1f` | `arch/arm64/boot/dts/qcom/{sdm845.dtsi,sdm845-oneplus-common.dtsi}` | GPL-2.0 | Device facts only; no code copied |
| https://code.googlesource.com/linux/torvalds/linux/+/10dd1a736d557e310a77117832874729a0175d57/drivers/input/rmi4/rmi_f12.c | `drivers/input/rmi4/rmi_f12.c` | GPL-2.0-only | Oracle cross-check of F12 Data1 size/field semantics only; no code copied |
| https://device.report/m/4e2b689190f1cfe92e335cad5ee2db9d9d917df139b5358d6412a9a33f5d8f68 | Synaptics RMI interfacing guide | Public document, no software SPDX | RMI family background only; retrieved guide is RMI3, not authoritative for F12 |
| https://gitlab.redox-os.org/redox-os/base.git @ `d2ffc4ca46d68f83b33c1e929e68dd7a7409668c` | base `Cargo.toml`, `Cargo.lock`, `drivers/inputd/src/lib.rs`, `drivers/common/src/lib.rs` | MIT | Adapted manifests and used existing input/MMIO mapping APIs; no source copied |

`crates/geni-i2c/` and `crates/rmi4-touch/` are original K9 MIT code. Their manifests, sources, and tests were copied into base `drivers/{geni-i2c-core,rmi4-touch-core}/` with only path changes. The two daemon entrypoints are original K9 MIT code.

## K8c IRQ-width integration

No external source file was copied. K6's downstream kernel commits were
rebased onto the K1/K8b kernel integration; the merge resolution retains
K1's 2048 descriptors and handler reservation behavior.

| Upstream URL and commit | Path | SPDX/header | Use |
| --- | --- | --- | --- |
| https://github.com/openbsd/src.git @ `20cb34434d36871c39729f7369062cba28fd06cb` | `sys/dev/ic/qcuart.c` | ISC | Spec-only GENI RX IRQ bits and acknowledgment order; no code copied |
| https://source.denx.de/u-boot/u-boot.git @ `8649a54d301a4d0ac526324c7a7ad2574939ff27` | `include/soc/qcom/geni-se.h` | GPL-2.0+ | Spec-only SE interrupt enable bits; no code copied |
| Arm GICv3 architecture | `ICC_IAR1_EL1.INTID` field | n/a | Spec-only 24-bit INTID width and special IDs; no code copied |
# trace-events provenance (wave 1, worker C2)

No upstream code was copied into `crates/trace-events`. The crate is original
MIT code; two upstream sources were read **spec-only** (names/vocabulary
adopted, no file copied):

| Upstream URL and commit | Path | SPDX | Use |
| --- | --- | --- | --- |
| https://github.com/open-telemetry/semantic-conventions.git @ `3e2580475b1fa266fe85a1d1ea7fe9c41a4783b5` | `model/gen-ai/deprecated/registry-deprecated.yaml`, `model/gen-ai/deprecated/spans-deprecated.yaml` | Apache-2.0 | Spec-only: `gen_ai.*` attribute keys, `gen_ai.operation.name` enum (9 values), span-name rule `{operation} {target}` (conventions since moved to semantic-conventions-genai; names unchanged) |
| https://github.com/ag-ui-protocol/ag-ui.git @ `73b04fdaec1f1b170a5efeeda3c050247a26075b` | `spec/1.0/schema.json` | MIT | Spec-only: 31 `EventType` discriminators, camelCase field names, required-vs-optional field lists, `TokenUsage`/`Interrupt`/outcome shapes |

## B5 Wasm sandbox

`crates/sandbox/` (library, tests, `guests/hello-rs`) is original B5 MIT code; no upstream source was copied. Dependencies are pulled from crates.io, not vendored.

| Upstream URL and commit | Path | SPDX/header | Use |
| --- | --- | --- | --- |
| https://github.com/wasmi-labs/wasmi.git @ `2970aa871cc1001b57b267ccecdcd1e42306199e` (tag `v2.0.0`, local `op6-refs/wasmi`) | crates.io `wasmi`, `wasmi_core` `=2.0.0` | MIT OR Apache-2.0 | Dependency; API read from the published 2.0.0 source (`Config`, `Linker::func_wrap`, `ResourceLimiter`, `build.rs` dispatch selection); no code copied |
| crates.io `wasmparser` 0.228.0, `spin` 0.9.9, `libm` 0.2.16, `bitflags` 2.13.2 | transitive dependencies of wasmi | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT; MIT; MIT; MIT OR Apache-2.0 | Dependency only |
| crates.io `wat` 1.261.0 | dev-dependency (WAT test fixtures) | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | Test-only dependency |
# Provenance

Every copied or adapted third-party file in this repo is recorded here:
upstream URL + commit, upstream path, SPDX license, and how it was used
(`copy` = verbatim, `adapt` = ported/modified, `spec-only` = reimplemented
from the public spec/output, no file copied).

## crates/wyoming — Rust port of the Wyoming protocol

- Upstream: https://github.com/rhasspy/wyoming.git @ `08a0c25` (v1.11.0)
- SPDX: `MIT` (upstream `LICENSE` is MIT; crate re-licensed MIT)
- Local checkout used: `$HOME/Developer/forks/op6-refs/wyoming`

| Upstream path | Local path | Mode | Notes |
|---|---|---|---|
| `wyoming/event.py` | `crates/wyoming/src/event.rs` | adapt | framed codec (`encode`/`encode_into`/`decode`); encoder byte-identical to `write_event` (`json.dumps(ensure_ascii=False)` separators + escapes) |
| `wyoming/audio.py` | `crates/wyoming/src/events/audio.rs` | adapt | AudioStart/AudioChunk/AudioStop + AudioFormat |
| `wyoming/vad.py` | `crates/wyoming/src/events/vad.rs` | adapt | VoiceStarted/VoiceStopped |
| `wyoming/ping.py` | `crates/wyoming/src/events/ping.rs` | adapt | Ping/Pong |
| `wyoming/error.py` | `crates/wyoming/src/events/error.rs` | adapt | ErrorEvent (text + optional code) |
| `wyoming/timer.py` | `crates/wyoming/src/events/timer.rs` | adapt | TimerStarted/Updated/Cancelled/Finished |
| `wyoming/user_event.py` | `crates/wyoming/src/events/user_event.rs` | adapt | UserEvent |
| `wyoming/snd.py` | `crates/wyoming/src/events/snd.rs` | adapt | Played |
| `wyoming/asr.py` | `crates/wyoming/src/events/asr.rs` | adapt | Transcript/Transcribe/TranscriptStart/Chunk/Stop + VadSensitivity |
| `wyoming/wake.py` | `crates/wyoming/src/events/wake.rs` | adapt | Detection/Detect/NotDetected |
| `wyoming/tts.py` | `crates/wyoming/src/events/tts.rs` | adapt | Synthesize*/SynthesizeVoice/SynthesizeTextFormat |
| `wyoming/intent.py` | `crates/wyoming/src/events/intent.rs` | adapt | Recognize/Intent/Entity/NotRecognized/IntentsStart/Stop |
| `wyoming/handle.py` | `crates/wyoming/src/events/handle.rs` | adapt | Handled/NotHandled/HandledStart/Chunk/Stop |
| `wyoming/pipeline.py` | `crates/wyoming/src/events/pipeline.rs` | adapt | RunPipeline/PipelineStage + stage validation |
| `wyoming/satellite.py` | `crates/wyoming/src/events/satellite.rs` | adapt | 6 empty satellite events |
| `wyoming/info.py` | `crates/wyoming/src/events/info.rs` | adapt | Describe/SelectProgram/Info + artifact hierarchy |
| (all typed modules) | `crates/wyoming/src/events/mod.rs` | adapt | `Eventable` trait + `EventKind` dispatch over all 46 event types |
| `tests/test_event.py` | `crates/wyoming/tests/framing.rs` | adapt | framing tests ported 1:1 + byte-exact parity + edge cases |
| (generated by reference) | `crates/wyoming/tests/fixtures/golden.wyoming`, `golden.json` | copy (generated) | 59-event stream written by upstream `write_event` via `tests/gen_golden.py` |
| — | `crates/wyoming/tests/gen_golden.py`, `tests/check_from_rust.py`, `examples/export_golden.rs` | new | cross-check harness; imports the reference read-only (no upstream code copied into these files) |

Not ported (out of scope, gaps reported): `wav_to_chunks`/`AudioChunkConverter`
(DSP), async TCP client/server helpers, `util/dataclasses_json.py` (Python-only
vendored dependency).

Protocol semantics were additionally verified against the reference at
runtime (both directions, byte-identical): see `docs/workbench/wyoming-crosscheck.md`.

## K13 qcom-rproc

No donor source file was copied. `crates/qcom-rproc` and `specs/qcom-rproc.md` are original MIT code informed by these pinned references:

| Upstream URL and commit | Path | SPDX | Use |
| --- | --- | --- | --- |
| https://github.com/openbsd/src @ `20cb34434d36871c39729f7369062cba28fd06cb` | `sys/dev/fdt/qcscm.c`, `qcpas.c`, `qcsmem.c` | ISC (file headers) | Spec-only PAS/SMEM/GLINK behavior and constants; no source copied |
| https://github.com/torvalds/linux @ `5b0764a53235d3c77e1ed674e836e1b3fe1a4b1f` | `arch/arm64/boot/dts/qcom/sdm845.dtsi`, `net/qrtr/af_qrtr.c`, `drivers/rpmsg/qcom_glink_native.c`, `drivers/soc/qcom/qmi_interface.c` | GPL-2.0 (oracle) | DT facts and wire-format cross-check only; no source copied |
| This repository @ C1 | `crates/mmio-replay` | MIT | Host-model style reference only; no source copied or runtime dependency |
## K10 UFS read-only core (worker K10, Cursor)

No upstream code was copied. `crates/ufs/` (`ufs-core`) and `specs/ufs.md` are original MIT work;
struct layouts, register offsets and sequences were read from these sources:

| Upstream URL and commit | Path | SPDX | Use |
| --- | --- | --- | --- |
| https://github.com/freebsd/freebsd-src.git @ `81f86c9a9417d7ad7445d46739e19a26ab916e6b` | `sys/dev/ufshci/{ufshci_reg.h,ufshci.h,ufshci_ctrlr.c,ufshci_uic_cmd.c,ufshci_dev.c,ufshci_req_sdb.c,ufshci_req_queue.c,ufshci_ctrlr_cmd.c,ufshci_sim.c}` | BSD-2-Clause | Spec-only (primary): UFSHCI register map + bits, UTRD/PRDT/UPIU layouts, HCE/UIC/link-startup/UTRL/NOP/fDeviceInit/descriptor sequences |
| https://fuchsia.googlesource.com/fuchsia @ `729cefa69788350583738b1e5890f10710f64dff` | `src/devices/block/drivers/ufs/{ufs.cc,registers.h,uic/uic_commands.h}` | BSD-3-Clause-style (Fuchsia LICENSE) | Spec-only cross-check: init ordering, IS clear after link startup, LU enumeration via unit descriptors, no READ(12) |
| https://git.kernel.org/pub/scm/linux/kernel/git/torvalds/linux.git @ `5b0764a53235d3c77e1ed674e836e1b3fe1a4b1f` | `arch/arm64/boot/dts/qcom/{sdm845.dtsi,sdm845-oneplus-common.dtsi}`, `drivers/ufs/host/ufs-qcom.{c,h}`, `drivers/ufs/core/ufshcd.c`, `include/ufs/{unipro.h,ufshci.h}` (the two headers fetched from kernel.org at that HEAD; the local checkout is sparse) | GPL-2.0 (DT: GPL-2.0/BSD dual per file) | Oracle/facts only: DT node, Qualcomm vendor register offsets/bits (CFG1 QUNIPRO_SEL, HW_VERSION, DME_VS_CORE_CLK_CTRL, MPHY_TX_FSM_STATE, TX_LCC_ENABLE), ordering of the vendor hooks, UTRLCLR polarity, retry counts |
| https://source.denx.de/u-boot/u-boot.git @ `8649a54d301a4d0ac526324c7a7ad2574939ff27` | `board/qualcomm/qcom-phone.env`, `configs/qcom_defconfig`, `drivers/iommu/qcom-hyp-smmu.c`, `drivers/phy/qcom/phy-qcom-qmp-ufs.c` | GPL-2.0+ / GPL-2.0 | Oracle/facts only: U-Boot probes UFS at boot (`preboot=scsi scan`) and leaves the UFS SMMU stream in S1 bypass |

`tests/fixtures/bringup_gpt.trace` is generated by the crate's own register/UPIU model (not a
phone capture); regenerate with `UFS_BLESS=1 cargo test`.
## Timeline app provenance (wave 1, worker C4)

No upstream code was copied into `apps/timeline`. The crate is original MIT
code. Third-party usage is dependency-only (no vendored or adapted files):

| Upstream | Version | SPDX | Use |
| --- | --- | --- | --- |
| https://crates.io/crates/orbclient | 0.4.6 | MIT | Dependency: window/`Renderer`/off-screen `Surface` for the one-screen UI (SDL backend on Linux) |
| `crates/trace-events` (this repo, C2) | path dep | MIT | Dependency: event schema + JSONL parsing for the feed and model |
| https://crates.io/crates/flate2 | 1.x | MIT OR Apache-2.0 | Dependency: zlib IDAT for the screenshot PNG writer |
| https://crates.io/crates/serde_json | 1.x | MIT OR Apache-2.0 | Dependency: JSON values in tool-result previews |

## B1 trace journal

`crates/tracejournal/` and the `traced` scheme code are original MIT code. The
Redox base worktree vendors this project's MIT `crates/trace-events/{Cargo.toml,src/*}`
and `crates/tracejournal/{Cargo.toml,src/lib.rs}` under `traced/vendor/`, from
branch `services/trace` commits `169f277` and `5380697` respectively. No
external source was copied for B1.

## K11 DWC3 device mode (USB gadget)

`specs/dwc3-gadget.md` and `crates/dwc3-gadget/` (library, tests, controller model) are original K11 MIT code; no upstream source was copied.

| Upstream URL and commit | Path | SPDX/header | Use |
| --- | --- | --- | --- |
| https://fuchsia.googlesource.com/fuchsia @ `729cefa69788350583738b1e5890f10710f64dff` (local `op6-refs/fuchsia`) | `src/devices/usb/drivers/dwc3/{dwc3-regs.h,dwc3-types.h,dwc3.cc,dwc3-ep0.cc,dwc3-endpoints.cc,dwc3-events.cc,dwc3-commands.cc}` | BSD-3-Clause (Fuchsia `LICENSE`) | Spec-only: register offsets/fields, TRB and event layouts, init/EP0/command sequences, Qualcomm `HS_PHY_CTRL` VBUS bit; no code copied |
| https://github.com/openbsd/src.git @ `20cb34434d36871c39729f7369062cba28fd06cb` | `sys/dev/fdt/qcdwusb.c` | ISC header | Read for glue; it only enables power/clocks/resets at this commit; nothing adopted |
| https://github.com/torvalds/linux.git @ `5b0764a53235d3c77e1ed674e836e1b3fe1a4b1f` | `arch/arm64/boot/dts/qcom/{sdm845.dtsi,sdm845-oneplus-common.dtsi}` | GPL-2.0 | Device facts only (addresses, IRQs, quirks, dr_mode, maximum-speed, SMMU SID) |
| https://github.com/torvalds/linux.git @ `5b0764a53235d3c77e1ed674e836e1b3fe1a4b1f` | `drivers/usb/dwc3/{dwc3-qcom.c,gadget.c,gadget.h,core.h}` | GPL-2.0 | Oracle facts only: QSCRATCH `GENERAL_CFG`/`SS_PHY_CTRL`/`SW_SESSVLD_SEL` bits, GEVNTCOUNT ack value, transfer resource index read back from `DEPCMD[22:16]`; no code copied |
| https://source.denx.de/u-boot/u-boot.git @ `8649a54d301a4d0ac526324c7a7ad2574939ff27` | `drivers/usb/dwc3/dwc3-generic.c` | GPL-2.0 | Cross-check of the same QSCRATCH facts only; no code copied |
| crates.io `embassy-usb-driver` 0.2.2 (repo https://github.com/embassy-rs/embassy, local `op6-refs/embassy` @ `b88d22c60df594c7d5a4964277cbb108b565ed97`) | `embassy-usb-driver/src/lib.rs` | MIT OR Apache-2.0 | Dependency; traits implemented, not copied |
| crates.io `embassy-usb` 0.6.0, `embassy-futures` 0.1.2 | `src/lib.rs`, `src/class/{cdc_ncm,cdc_acm}` | MIT OR Apache-2.0 | Dev-dependencies: the real USB stack + classes run on the driver in host tests; read for control-pipe call order; not copied |
## voice-bridge (C5, wave 1)

No upstream source file was copied or adapted. All code under
`apps/voice-bridge/` is original MIT code written for this project. The
Wyoming wire semantics were derived from the in-repo `crates/wyoming`
crate (itself recorded above as a port of rhasspy/wyoming) and the
trace journal uses the in-repo `crates/trace-events` crate; neither was
modified.
## A2UI renderer provenance (wave 1, worker C6)

No upstream code was copied into `crates/a2ui-render`. The crate is original
MIT code. The A2UI specification was **read, not vendored** — the schemas and
component semantics below were consulted in the local clone and re-implemented
from scratch in Rust (no generated code, no vendored JSON).

| Upstream URL and commit | Path | SPDX | Use |
| --- | --- | --- | --- |
| https://github.com/google/A2UI.git @ `409ded29ccac3eebc541979ae5320771703248e8` | `specification/v0_9/json/{server_to_client.json,client_to_server.json,common_types.json,sample.json}`, `specification/v0_9/docs/{a2ui_protocol.md,basic_catalog_implementation_guide.md,a2ui_custom_functions.md}` | Apache-2.0 | **Spec-only**: message envelope (`createSurface`/`updateComponents`/`updateDataModel`/`deleteSurface` + required `version`), flat component list with `id`/`component` and a `root`, `ChildList` (id list or `{componentId,path}` template), DynamicString/Number (`literal*`, `path`, `call`/`args`), JSON-Pointer data binding with relative paths inside templates, the basic-catalog function names, and the client→server `action`/`error` envelope (`name`, `surfaceId`, `sourceComponentId`, `timestamp`, `context` with bindings resolved) |
| https://github.com/google/A2UI.git @ same commit | `specification/v0_9/catalogs/minimal/catalog.json`, `specification/v0_9/catalogs/basic/catalog.json`, `specification/v0_9/catalogs/{minimal,basic}/examples/*.json` | Apache-2.0 | Spec-only: what a conforming client must draw — the **minimal** catalog is exactly Text, Column, Row, Button, TextField — and the shape of real documents the fixtures imitate (Column-rooted pages, Card groups, List templates, TextField variants, Image sizes) |
| https://github.com/ag-ui-protocol/ag-ui.git @ `73b04fd` | event vocabulary | MIT | Spec-only: interactions are journalled as AG-UI `CUSTOM` records (`name` + `value`) through `crates/trace-events` (C2) |

Adapted **within this repo** (same MIT project, not third-party):

| Source path | New path | SPDX | Use |
| --- | --- | --- | --- |
| `apps/timeline/src/png.rs` (C4) | `crates/a2ui-render/src/png.rs` | MIT | Adapted: the hand-rolled PNG chunk/CRC writer, plus a new decoder (`read_png`/`decode_png`) so the golden tests compare decoded pixels |
| `apps/timeline/src/view.rs` (C4) | `crates/a2ui-render/src/view.rs` | MIT | Adapted: the black+orange OrbClient palette, 8×16 unifont text drawing, and the off-screen screenshot path |

Dependencies (no vendoring): `orbclient` 0.4.6 (MIT), `serde_json` 1.x and
`flate2` 1.x (MIT OR Apache-2.0), `crates/trace-events` (this repo, MIT).

`Progress` is a documented **extension**, not a claim about the spec: neither
the minimal nor the basic catalog defines a progress bar, so this client
accepts the community `MaterialProgressBar` shape (value 0..100, `min`/`max`,
`mode`) under the names `Progress`, `ProgressBar` and `MaterialProgressBar` —
see `crates/a2ui-render/src/scene.rs`. Anything else unknown renders a visible
`unsupported: X` chip, never silence.

## B4 svcd — Service IR executor (worker B4, Cursor)

No upstream code was copied. `crates/svcd/` is original MIT work implementing
`docs/design/SERVICE-IR.md` (D1, this repository). Upstream projects below
were used **spec-only, through D1's description**; B4 did not read or copy
their source.

| Upstream URL and commit | Path | SPDX | Use |
| --- | --- | --- | --- |
| This repository @ `f6e2c2e` (D1 merge) | `docs/design/SERVICE-IR.md` §2.1 schema, §8.1–8.3 examples | MIT | Spec. `crates/svcd/tests/fixtures/{service-ir-v1.schema.json,svc.*.json}` are verbatim copies of its JSON blocks (a test fails if they drift) |
| https://github.com/home-assistant/intents @ `e77dd13507b57182380c831efa124cd13282bbbc` | `sentences/en/HassStartTimer/*.yaml` (as cited by D1) | CC-BY-4.0 | Sentence-template syntax (`[opt]`, `(a\|b)`, `{list:slot}`) re-implemented in `src/intent.rs`; the timer fixture's sentences are D1's adaptation — attribution: Home Assistant Intents, CC-BY-4.0 |
| https://github.com/NousResearch/hermes-agent @ `931cf3365739da38fe32d30e12d94e7df8056973` | `cron/executions.py`, `cron/scheduler_tick.py` (as cited by D1) | MIT | Concepts only: execution state machine with immutable terminal rows, unique effect keys, advance-before-dispatch (`src/ledger.rs`, `src/exec/power.rs`) |
| https://github.com/OpenVoiceOS/ovos-bus-client @ `edc21374ac7725d58f10da18ea257048ef2f2dee`, https://github.com/OpenVoiceOS/ovos-core @ `e3d075ac185ce4ca06a010fbe893eb20052cf305` | `message.py`, `intent_services/*` (as cited by D1) | Apache-2.0 | Message shape `{type, data, context}`, `ovos.intent.handler.{start,complete,error}`, `ovos.intent.fallback`, `speak` topic names |
| https://github.com/google/A2UI @ `409ded29ccac3eebc541979ae5320771703248e8`, https://github.com/thesysdev/openui @ `9d73d17d2bbac565672a85e78f6eb7740cb0d9ca` | `specification/v0_9/json/*`, `packages/a2ui/src/statement-patch.ts` (as cited by D1) | Apache-2.0 / MIT | Message names (`createSurface`, `updateComponents`, `updateDataModel`, `deleteSurface`) and `$name = …` statement upsert in `src/exec/ui.rs` |
| http://howardhinnant.github.io/date_algorithms.html | `days_from_civil` | Public domain | Algorithm re-typed in `src/time.rs` |

Runtime crates: `serde`/`serde_json`/`serde_path_to_error` (MIT OR Apache-2.0),
`blake3` (CC0-1.0 OR Apache-2.0), `trace-events` and `sandbox` (this repo, MIT).
Test-only: `jsonschema` (MIT), `wat` (Apache-2.0 WITH LLVM-exception).
## E1 live-image packaging

No external source was copied into tracked E1 files. The untracked QEMU live
image packages Redox's aarch64 `orbital` and `orbdata` binaries/assets via the
Redox `desktop-minimal` recipe. The Orbital source inspected for its command
and font paths is `https://gitlab.redox-os.org/redox-os/orbital` at
`a6132b1c2add31def08b4b537271492c17e3e343` (`src/main.rs`, `src/config.rs`,
`src/scheme.rs`, MIT). The asset source is
`https://gitlab.redox-os.org/redox-os/orbdata` at
`58d1140176430f222ac0749c8b83349f2c4da916` (packaged assets include
their license notices). Our timeline code remains MIT and uses OrbClient as a
Cargo dependency.
# Provenance — third-party code and documentation consulted

Rule (from `docs/TASKS-WAVE1.md`): every file in this repo that is **copied** or **adapted** from an
upstream project gets a row here — upstream URL + commit, the upstream path, the SPDX licence, and whether
the result is a verbatim copy, an adaptation, or spec-only (read for behaviour, not copied). A missing row
means "we wrote it"; an **unrecorded** third-party source is worse, because the next agent then cannot tell
original work from inherited risk.

## Wave 1, task D2 — `docs/design/CAPS-SANDBOX.md` + `tools/caps-check/` (crate + `examples/*.json`)

**Nothing in this deliverable is copied or adapted code.** `tools/caps-check` is original (SPDX `MIT`, no
dependencies, `#![forbid(unsafe_code)]`), written to *check* the design's rules; the design document *quotes*
four donor projects and `specs/pdc.md`. Every row below is therefore **spec-only**, and the SPDX column is
still filled in: whoever later lifts one of these functions inherits that licence, which is the reason the
copyleft traps are named separately under the table.

Reference clones live in `~/Developer/forks/op6-refs` (shallow, `--depth 1`, listed with commits in
`docs/REFERENCES-INDEX.md`). "Path" is what was actually read. `[?]` marks a claim not verified against the
local clone — say so rather than implying a file was read when the clone does not contain it.

| upstream | commit | path consulted | SPDX | kind | what it became |
|---|---|---|---|---|---|
| Fuchsia (component manager / CML) | `729cefa6` (sparse: `src/devices` + build metadata only) | the published component-manifest + capability-routing docs (`use`/`offer`/`expose`, `assembly: {short-lifecycle}`, the *narrowing* rule, `service-decl`/`instance-override` precedence, `framework:components/ffx-component-test`, `basic_shim:wire_name` in `src/devices/blink/`); **`src/lib/component_config` and the component docs are not in the sparse clone** — read from fuchsia.dev, not from the clone [?] | BSD-3-Clause (repo `LICENSE`) | spec-only | R9 (a manifest with an unknown key is a build error, not a shrug), R10 (declare-it-or-cannot-use-it, and *narrowing only*), §5.5's manifest + routing shape, §5.3's grant states (`issued/live/parked/expired`) and §5.4's `caps:check` answer shape |
| wasmi | `2970aa8` (workspace `version = "2.0.0"`) | `crates/wasmi/Cargo.toml` (deps: `wasmparser` 0.241, `spin` 0.10), `crates/wasmi/src/lib.rs:83`, `func/func_type.rs:20`, `func/caller.rs:37-66`, `resolver.rs` + `store/envs.rs` (`FuncTypeRegistry`, `clone`), `store/mod.rs:270,365-374` (`CallHook`, `resume`), `engine/resumable.rs:24` (`ResumableCall::{Finished, HostTrap, OutOfFuel}`), `error.rs:37-45` (`Trap::{HostFunction, OutOfFuel, StackOverflow}`), `engine/limits/engine/limiter.rs:36,76` (`InstanceLimiter`, default `max_instances_size = 20`), `engine/limits/stack.rs` (native stack, not a Wasm call-stack limit), `module/custom_section.rs:12-17` (`CustomSection::allow_at_entrypoint`), `README.md`, `.github/workflows` | MIT **OR** Apache-2.0 (both in `crates/wasmi/Cargo.toml`) | spec-only | R6/R11 (both engines *can* deny mid-call, so "no half-start" is implementable), R7 (wake = `Lp`, not `Fm`; the 570 µs/WASM-150 ms/30 s numbers are *not* a budget → R7's `max`/`deadline`), R10's two-knob ceiling, §5.4's `caps:check` host function, and the *evidence* for "neither engine rejects Wasm **components**" (R11) |
| tinywasm (fallback engine) | `7160f7f` (workspace `version = "0.11.0"`) | `crates/parser/src/module.rs:96-98` (**`Encoding::Component => Err(InvalidEncoding)`** — the second half of the R11 evidence), `crates/types/src/lib.rs` (`ParseLimits`, `CallStackLimits`, `FuncContext::state_mut`), `engine.rs:45` + `dispatch.rs:83` (`ExecProgress::Suspended` — a parked call is *indistinguishable* from a slow one), `engine/limits.rs:14` (`ResourceLimiter`: **memory + table only**), `Cargo.toml` (`no_std`, `libm`), `crates/wasi/src/net.rs:86-171` (exact-host `Policy` *plus* rate limiting) | MIT **OR** Apache-2.0 (`Cargo.toml`) | spec-only | the same as wasmi, plus: no `*`-style destination wildcard in *either* donor (R1), and *no* instance/table/global count caps in tinywasm → R6's caps are engine-specific, so R7's time bound is the enforcement that works on **both** |
| wasmCloud `wash-runtime` | `d4a3b66` | `crates/wash-runtime/Cargo.toml:135-139` (`wasmtime = "48.0.7"` + cranelift → "wasmtime 48" in §3.2/§5.4), `crates/wash-runtime/src/engine/guest_memory.rs` (the per-call `wasmtime::ResourceLimiter` — `StoreMemoryLimiter`, and its "no grant ⇒ deny" default), `sockets/host_network.rs` + `sockets/host_{tcp,udp}*/` (connection-time destination check), `timeouts.rs` (per-request/response/idle deadlines), `engine/linked_call.rs` + `engine/abandon.rs` (in-flight cancellation: "a new interaction with the same `operation` id cannot pass while the old one is still considered live") | Apache-2.0 | spec-only | R6 (check per host-function call *and* per grant, with a `revalidate_after` staleness clock, because a cache that can go stale is not proof), R4 (revocation must *cancel* in-flight work), R1's exact-origin ceiling, R5's one-line-per-decision audit trail |
| open-interpreter (**the Rust/Codex-based fork**, not the old Python `execute_shell` version) | `ac1fcc8` | `codex-rs/network-proxy/src/network_policy.rs:14` (`ProxyTarget::{Host, Cidr}` — "or prose" was rejected on purpose), `codex-rs/http-client/src/network_policy.rs` (one exact `host[:port]`, and *revocation invalidates in-flight*), `codex-rs/utils/absolute-path/src/lib.rs:184` (`test_windows_path_on_unix`), `codex-rs/protocol/src/models.rs:194` (`call_id` round-trip = the *why* join key) | Apache-2.0 (repo `LICENSE`; the fork's `codex-rs/` tree is Apache-2.0 too) | spec-only | R1 (exact destinations; a host *lookalike* is the hole), R3 (a `read_only` grant is a ceiling, not a hint), R4, R5 (one event per decision, with the reason attached) |
| extism | `051b765` | `runtime/src/pdk.rs:214-240` (a host function that answers `deny` / `unavailable`, then answers `allow` once the host's own policy says `Ok`) | BSD-3-Clause | spec-only | R11 (a host function is *allowed to* deny, so it is the right enforcement seam — unlike `execute_shell`), §5.4's `caps:check` + `Unavailable`, R5's "log *and* say it" |
| Goose (recipe side only) | `2b17adc` | `goose/crates/goose/providers/src/groq/mod.rs` (`chat_messages`, `ChatMessageRole::Tool`), `recipe.rs:49` (`Recipe` = `{instructions, steps}`), `recipe generators` (`main.rs:75-105` emits **only** `{instructions, steps}` — R10's evidence), `scheduler.rs` (`resume_from=resume_from.jsonl`), `crates/goose-mcp/src/{cron.rs:14-15, cron.rs:96, sandbox.rs}` (an *unbounded* 15-minute `max_duration` + `sleep_and_check`), `crates/goose-mcp/Cargo.toml` → `goose/crates/goose-mcp`, `crates/dagrunator/Cargo.toml` → `goose/crates/dagrunator`, `crates/dagrunator/src/{pkg/dag.rs:273, pkg/dataloader.rs:177, recipe.rs:12, main.rs:53, expressions/gtemplate.rs:79}` | Apache-2.0 | spec-only | the §7 worked examples' *comparison class* (what "timer 15 min always on" and "mic 24 s continuously" look like when the substrate cannot say no), and R7/R12's "least power, start on demand" — Goose is *not* a donor for the grant model itself |
| our own `specs/pdc.md` + `docs/research/ACCEL-R3.md` §11 | n/a (project-internal) | PDC = "wakeup source stays armed while clocks and bus votes are released"; ACCEL-R3 §11's "wake with no grant → None = wasted" | MIT (project) | spec-only (internal) | `Fm` / `Lp` / `Off`. **`Lp` is our invention**, recorded as such in §5.6 and §8.3.2: it is *derived from* those two notes and from the `baml_sgm_cmd2` 4-state ladder (`OFF/RET/LPM_A/LPM_B`, `sleep_count` vs `resume`), it is **not** a Linux regulator-domain port, and R8's floor has no donor at all |

### Licence traps, for whoever writes the code (nothing here takes code from these)

- `~/Developer/forks/op6-refs/core` (RedPlanetHQ `core` @ `4a5418d`) — **AGPL-3.0** → study only, never copy
  (`docs/REFERENCES-INDEX.md` review flag).
- `fdt-rs` (repnop/`fdt` @ `8fa4388`) — **MPL-2.0** → file-level copyleft; prefer `dtb-walker` (MIT) for code.
- `freebsd-src` @ `81f86c9` — BSD-2-Clause; `sys/dev/ufshci/ufshci_ctrlr.c` carries a per-file SPDX.
- `edk2-msm` @ `e195262` — `GPLDrivers/` excluded; anything under `u-boot` needs a per-file SPDX check
  (GPL-2.0 overall).
- wasmi and tinywasm are **dual** MIT/Apache-2.0: fine to *link against*, and the Apache leg is what makes a
  future "port this helper" step non-controversial — but a `tools/caps-check` → engine port still needs the
  `Lp` wake-input arm re-derived from `specs/pdc.md`, which has no upstream at all.

### What was *not* verified, and how to check it

- `kernel/src/scheme/{memory.rs:253, irq.rs:277, debug.rs:76}` (the uid-only gates) and `time.rs`/`acpi.rs`
  existence: checked against `~/Developer/forks/redox` @ `45bca4e3`.
- "No IOMMU/SMMU code, no PMIC/regulator/clock driver, only two DT-driven platform drivers (`hwd`'s DTB
  dumper, `bcm2835-sdhcid`)": **from `docs/research/FINDINGS-T1.md`** (lines 43-44, 56, 61, 116), not from a
  fresh grep — the "47 lines" figure is that file's line 54, not a re-measurement of `hwd`.)
- `relibc/redox-rt/src/sys.rs:608-620` (namespace-link open), `base/init/src/service.rs:111-114`
  (`register_scheme_to_ns`), `kernel/src/context/{context.rs:188, memory.rs:304}` (`TODO: replace with
  capabilities!`): all three exist in that checkout; the *reading* of the first two (name→scheme resolution is
  per-process, and a link can be opened directly) is **[?]** — it is the assumption behind §8.3.3, so it gets
  checked before `caps:` becomes a kernel boundary.
- The design's whole "no Wasm component parser exists" claim rests on the two `[V-code]` cites in the second
  and third rows above.

# EMU-TOUCH provenance (`patches/touch-e2e/`, `crates/rmi4-touch`)

All new code is original MIT. No upstream file or GPL text was copied. These references supplied behaviour facts only:

| Upstream URL and commit | Path | SPDX/license | Use |
| --- | --- | --- | --- |
| https://github.com/torvalds/linux/tree/5b0764a53235d3c77e1ed674e836e1b3fe1a4b1f | `drivers/of/irq.c` (`of_irq_init`) | GPL-2.0 | spec-only: a controller whose interrupt parent is itself is a root |
| same | `drivers/irqchip/irq-gic-v3.c` (`gic_dist_init`) | GPL-2.0 | spec-only: SPIs are placed in Group 1 through `GICD_IGROUPR` |
| same | `include/linux/soc/qcom/geni-se.h` | GPL-2.0 | spec-only: M_IRQ status bit names cited in the EMU-TOUCH gaps |
| local `qemu-oneplus6` `462dbe3` (read-only) | `hw/input/rmi4-s3706.c`, `hw/intc/qcom-pdc.c`, `hw/gpio/qcom-tlmm.c`, `ui/input.c` | GPL-2.0-or-later | spec-only: emulator register layout, input handler and wiring used by the MIT harness |

# CV2 SMART-TURN provenance (`docs/workbench/user-simulations/key-tests/wakeword/smartturn_engine.py`, deployed artifact)

| Upstream URL and commit | Path | SPDX/license | Use |
| --- | --- | --- | --- |
| https://huggingface.co/pipecat-ai/smart-turn-v3 @ f766f81d3cfdf7737ac64aad813d91bbfd56bf93 | `smart-turn-v3.2-gpu.onnx` (deployed to `~/agentic/smartturn/model/` on the phone by `smartturn-setup.sh`; NOT committed to this repo) sha256 `ab8dc64b88713f90b571c15b714bd1330e6c883cad8763dacf65c9376dc539be` | BSD-2-Clause | Pipecat Smart Turn v3 turn-completion model (Whisper-tiny encoder + linear head, FP32 build): judges at each VAD pause whether the utterance-so-far is complete. Advisory only - capture falls back to the CV0 endpointer whenever it is unavailable. FP32 chosen over the int8 `-cpu` build (sha256 `2bb02631...67e4f`) after measurement: the int8 build's borderline probabilities are runtime-dependent on aarch64 onnxruntime (reference 0.64 -> 0.96 on-device), the FP32 build matches the reference to 4 decimals at ~300 ms/prediction (CV2-RESULTS.md). |
| https://github.com/pipecat-ai/smart-turn @ 4786657 | `inference.py`, `audio_utils.py` | BSD-2-Clause | spec-only: preprocessing contract (last-8 s window, zero-pad at start, WhisperFeatureExtractor log-mel) replicated in NumPy inside `smartturn_engine.py`; validated against the reference pipeline to 1.1e-5 max feature difference with identical output probabilities. No upstream code copied. |
| https://pypi.org/project/onnxruntime/ (wheel installed by `smartturn-setup.sh` into `~/agentic/smartturn/venv`, version pinned there) | `onnxruntime` Python package | MIT | upstream runtime artifact: executes the model inside the `voiced-smartturn` warm service. Runs, never modified. |
