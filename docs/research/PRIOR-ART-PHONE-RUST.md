# Prior art: Rust OS / phone-OS code we can reuse for Redox on OnePlus 6 (SDM845)

Researched 2026-10-24. Licenses and push dates came from the GitHub API, crates.io API, GitLab API
and raw file SPDX headers, read the same day, except where marked **(from memory)**.
"Activity" is the last push date.

Legend: OK = permissive (MIT/Apache/BSD/ISC/0BSD), COPY = lift code, PORT = re-implement
from a reference, STUDY = read only, GPL = do not copy into a permissive tree.

## 0. Baseline: what Redox already has (the starting point)

| Item | Finding |
|---|---|
| Canonical repos | GitLab, not the GitHub mirrors. `redox-os/kernel` active 2026-10-24; `redox-os/base` holds drivers; `redox-os/redox` active 2026-10-24. |
| License | MIT (kernel, relibc, orbital, redox, base `LICENSE`). Anything we write fits the same tree. |
| Kernel aarch64 | `src/arch/aarch64/device/{cpu,irqchip,generic_timer,psci,rtc,serial}.rs`. irqchips are `gic`, `gicv3`, `irq_bcm2835`, `irq_bcm2836`, `null`. Recent commits: huge-page physmap (2026-09-13), page-fault handling (2026-09-24), IRQ fixes (2026-07/08). |
| DTB handling | Kernel `src/dtb/{irqchip,serial,mod}.rs`. It parses the DTB with `repnop/fdt` (MPL-2.0, pinned git rev; commit "Update fdt crate" 2026-10-21). Bootloader also uses `fdt`. |
| Boot | `redox-os/bootloader` has an `aarch64-unknown-uefi` target (active 2026-10-22). So the OnePlus 6 needs a UEFI layer (see section 4, edk2-msm). |
| Drivers in `base/drivers` | audio: ac97d/ihdad/sb16d. graphics: vesad, virtio-gpud, ihdgd, fbcond. input: ps2d, usbhidd. storage: ahcid, nvmed, ided, **bcm2835-sdhcid**, usbscsid, virtio-blkd. usb: xhcid, usbhubd. Also pcid, acpid. |
| What is missing for SDM845 | No UFS, DWC3 (Qualcomm USB glue), GENI (UART/I2C/SPI), SPMI, TLMM pinctrl, SMMU, MDSS/DSI panel, RMI4 touch, ADSP/Q6 audio, WiFi/modem. All of that is greenfield on Redox. |
| Hardware table | `HARDWARE.md` lists only a Raspberry Pi 3B+ (0.8.0, UART console) as aarch64 hardware. No Qualcomm entry. Nobody has published a Redox-on-Snapdragon port that I could find. |
| 2026 news | Phoronix: Jan 2026 cargo/rustc on Redox; Apr 2026 real-hardware fixes; Jun 2026 GTK3 backend for Orbital, USB gamepads; Aug 2026 initial AArch64 multi-core, io_uring-like API, NUMA, QEMU on Redox. |

Redox-derived forks: I found none that are mobile. GitLab only shows personal copies. Treat the OnePlus 6
port as first of its kind.

## 1. Rust OS / embedded projects: lift-able kernel and driver code

### 1a. Best sources for SDM845 peripheral code (not Rust, but permissive)

The big finding: Linux drivers are GPL-2.0, but **OpenBSD and FreeBSD already wrote clean Qualcomm
drivers under ISC/BSD**. Port them to Rust as references, no GPL contamination.

| Source | License | Activity | What it covers | Reuse notes |
|---|---|---|---|---|
| OpenBSD `sys/dev/fdt/qciic_fdt.c`, `qcuart_fdt.c`, `qcspmi.c`, `qcpmic.c`, `qcgpio_fdt.c`, `qcsmem.c`, `qcscm.c`, `ufshci_fdt.c`, `qcdwusb.c` (github.com/openbsd/src) | ISC (OpenBSD standard; headers show Kettenis/Wildt/Glocker copyrights, 2017-2026) **(license text from memory; headers confirmed)** | `qcuart_fdt.c` 2026 | GENI I2C (`qcom,geni-i2c`), GENI UART incl. debug-uart, SPMI pmic-arb, SMEM, SCM, UFS HCI (`jedec,ufs-2.0`), Qualcomm DWC3 glue (`qcom,dwc3`) | Most were written for Snapdragon X (X1E). SDM845 deltas: pmic-arb is v5 not v7; TLMM binding is `qcom,sdm845-pinctrl` (OpenBSD lists sc7280/sc8280xp/x1e80100 only). Same IP family, so PORT with register-offset fixes. `qcdwusb.c` is the closest hit for the OnePlus 6 USB-C port. |
| FreeBSD `sys/dev/iicbus/controller/qcom/geni_iic.c` | BSD-2-Clause (header confirmed, P-H Kamp 2025) | 2025 | GENI I2C written from scratch, no vendor docs (review D49676) | Good for touch I2C bring-up. |
| NetBSD `sys/dev/fdt/dwc3_fdt.c` | BSD-2-Clause style (header: McNeill 2018) | 2022 header | Generic DWC3 FDT attach | Reference for DWC3 init order. |
| Linux `drivers/{soc/qcom/qcom-geni-se.c, i2c/busses/i2c-qcom-geni.c, tty/serial/qcom_geni_serial.c, spmi/spmi-pmic-arb.c, usb/dwc3/*, ufs/host/ufs-qcom.c, pinctrl/qcom/pinctrl-sdm845.c, gpu/drm/panel/panel-samsung-sofef00.c, input/rmi4/*}` | GPL-2.0 / GPL-2.0-only (SPDX confirmed) | live | Everything the OnePlus 6 needs, tested on this exact phone | **GPL: STUDY only.** Use for register facts and init sequences; do not paste. Clean-room discipline: one person reads, another writes, record sources. |
| Linux `sdm845.dtsi`, `sdm845-oneplus-enchilada.dts` | GPL-2.0 (SPDX confirmed) | live | The hardware description. Redox's kernel already consumes a DTB. | Use the **DTB binary as data** at boot (as pmOS and edk2-msm do). Do not copy the DTS into our repo. |

Panel note: the OnePlus 6 panel driver in Linux is `panel-samsung-sofef00.c` (GPL-2.0-only). It is DSI
command-mode init, so we only need the init command bytes plus DSI host (MDSS) bring-up. Getting pixels
before a real DSI/DPU driver exists is realistically via the **UEFI GOP framebuffer from edk2-msm**
(see section 4), which Redox `vesad`/`fbcond`-style drivers can already drive.

### 1b. Rust building blocks (all permissive unless noted)

| Crate / project | License | Activity | Use for | Notes |
|---|---|---|---|---|
| `aarch64-paging` | MIT OR Apache-2.0 | 2026-10-22 | Page tables for kernel/loader | Active, Google/Arm origin. Useful if the loader needs its own MMU setup. |
| `arm-gic` | MIT OR Apache-2.0 | 2026-09-25 | GICv3 (SDM845 uses GICv3) | Cross-check/replace Redox `gicv3.rs`. |
| `arm-psci`, `smccc` | MIT OR Apache-2.0 | 2025-10 / 2026-05 | PSCI/SMC calls for CPU bring-up | Redox already has `psci.rs`; SMC calls also needed for Qualcomm SCM. |
| `arm-pl011-uart`, `safe-mmio` | MIT OR Apache-2.0 | 2026-08 / 2026-09 | Safe MMIO register access | `safe-mmio` is a model for our GENI register structs. |
| `tock-registers` | MIT/Apache-2.0 | 2025-10 | Typed register definitions | Best way to describe GENI/UFS/DWC3 registers. |
| `fdt-raw` | MIT OR Apache-2.0 | 2026-03 | FDT parsing (permissive alternative) | Redox uses `fdt` (MPL-2.0, file-level copyleft, fine to depend on). `fdt-parser` and `flat_device_tree` are also MPL-2.0. `dtb-walker` is MIT but stale (2022). Google `dtoolkit` license shows NOASSERTION on GitHub: check before use. |
| `embedded-hal` / `embedded-hal-async` / `embedded-io` | MIT OR Apache-2.0 | 2024 / 2025 | Trait layer so Rust I2C/HID/sensor crates run on our GENI driver | Write a GENI `embedded_hal::i2c::I2c` impl once, then reuse off-the-shelf device crates. |
| `uefi` / `uefi-raw` (rust-osdev/uefi-rs) | MIT OR Apache-2.0 (GitHub shows Apache-2.0) | 2026-10-19 | UEFI apps for aarch64 | Redox bootloader uses its own `redox_uefi`. uefi-rs is the broader-featured alternative. |
| `rust-osdev/bootloader` | Apache-2.0 | 2026-08-18 | Reference | x86-centric; limited value. |
| `buddy_system_allocator`, `talc`, `linked_list_allocator` | MIT / MIT / Apache-2.0-MIT | 2026 | Early-boot allocators | Only if bypassing the Redox kernel allocator in a stage-1 tool. |
| `smoltcp` | 0BSD | 2026-08-17 | Tiny TCP/IP | Use for the USB-gadget debug network before Redox's netstack is up (Redox netstack itself is smoltcp-based, I believe **(from memory)**). |

### 1c. Rust OS projects (to learn from, not much to copy)

| Project | License | Activity | Reuse notes |
|---|---|---|---|
| Tock (tock/tock, libtock-rs) | Apache-2.0 / MIT dual (`LICENSE-APACHE` + `LICENSE-MIT` present) | 2026-10-23 | Cortex-M/RISC-V; `kernel/` HIL traits and capsule patterns are good API models. Not AArch64 phone class. STUDY. |
| Hubris (oxidecomputer) | MPL-2.0 | 2026-10-24 | Microkernel with per-driver tasks, good driver-task and IPC design. Cortex-M only. File-level copyleft. STUDY. |
| Hermit (hermit-os/kernel, hermit-rs, uhyve) | Apache-2.0 / MIT-style | 2026-10-15 | Has aarch64 support but unikernel on hypervisors; its `virtio` and `pci` code is reusable but irrelevant to SDM845. STUDY. |
| Theseus (theseus-os/Theseus) | MIT | 2024-09 (stale) | Research OS, x86-only practically. Skip. |
| seL4 + rust-sel4 | seL4: BSD-2 (kernel) / GPL-2 mix **(from memory; GitHub shows NOASSERTION)**; rust-sel4 BSD-2 | 2026-10 | seL4 already supports SDM845? Not verified: check seL4 platform list before relying on it. Valuable as a second reference for how the SDM845 SMMU/GIC is brought up under a microkernel. |
| zCore (rcore-os) | MIT | 2026-09-06 | Rust Zircon-like kernel with aarch64 support; examples of driver traits. STUDY. |
| oreboot | GPL-2.0 | 2026-07 | Rust firmware, GPL. STUDY only. |
| Fuchsia (Google) | BSD-3-Clause **(from memory)** | live | Has SDM845-adjacent code? Not verified. Zircon driver framework designs are relevant to a driver-per-process model. |

### 1d. Asahi Linux Rust

| Item | License | Activity | Notes |
|---|---|---|---|
| `AsahiLinux/linux` `drivers/gpu/drm/asahi/*.rs` | **`GPL-2.0-only OR MIT`** (SPDX in `driver.rs`, confirmed) | 2026-10-22 | Dual-licensed on purpose "so the work can be reused in other OSes". MIT half is copyable in principle, but it depends on Rust-for-Linux kernel crate abstractions (GPL) and targets Apple AGX, not Adreno 630. Value = patterns for GPU firmware queues and MMU in Rust, not code. Per-file SPDX must be checked before any lift. |
| Rust-for-Linux (`kernel` crate) | GPL-2.0 | live | GPL. STUDY. |
| `AsahiLinux/m1n1` | GPL-2.0 per GitHub (README says MIT, check file headers) **(partly from memory)** | 2026-10-24 | Apple-specific bootloader. Skip. |
| `AsahiLinux/docs` | NOASSERTION (mixed) | 2026-10-24 | Good writing on hardware bring-up methodology. |

Adreno 630 GPU: no Rust driver exists. Mesa freedreno/turnip (MIT) is the only open reference
**(from memory)**. A GPU on Redox is far off. Plan for CPU rendering (see section 2).

### 1e. USB device stacks (gadget mode for debug networking / serial)

The OnePlus 6 gadget needs a **DWC3 device-mode controller driver**; no Rust crate implements DWC3.
Crates below supply the class layer once a controller driver exists.

| Crate | License | Activity | Notes |
|---|---|---|---|
| `embassy-usb` (embassy-rs/embassy) | MIT OR Apache-2.0 | 2026-10-24 (repo) | Async stack. Classes: CDC-ACM, **CDC-NCM** (+ `embassy_net` glue), HID, MIDI, MSC. `embassy-usb-driver` is the trait to implement for DWC3. `embassy-usb-synopsys-otg` (MIT OR Apache-2.0, 2026-05) is for DWC2/OTG-FS, a different core than DWC3, but same vendor; register-level reference for the endpoint model. |
| `usb-device` + `usbd-serial` | MIT | 2024-03 / 2024-04 (repo pushes 2026-06 / 2025-10) | Sync stack, CDC-ACM. Simpler, less maintained. |
| `usbd-hid` | MIT OR Apache-2.0 | 2026-09-02 | HID descriptors. |
| `usbd-ethernet` | MIT | 2025-05 | Small. |

Pragmatic path: implement `embassy-usb-driver::Driver` for the SDM845 DWC3 in device mode, run CDC-NCM for
a Redox-to-host IP link and CDC-ACM for a console. Use OpenBSD `qcdwusb.c` plus NetBSD `dwc3_fdt.c` for
glue and init, Linux `drivers/usb/dwc3` as a behavior reference only. Redox's `xhcid` handles host mode
only. Note SDM845 uses a Synopsys DWC3 with Qualcomm QMP/Femto PHYs, needing PHY init sequences (GPL
Linux phy drivers are the only reference found; check OpenBSD `qcusbphy`-style drivers).

### 1f. Rust touch / input

OnePlus 6 touch is a Synaptics RMI4 device over I2C (Linux modules list confirms `rmi_core`, `rmi_i2c`
in the pmOS initfs, **see section 4**). No Rust RMI4 crate found. Plan: GENI I2C driver + `embedded-hal`
I2C + a small RMI4 function-F11/F12 reader. Linux `rmi4` is GPL-2.0-only (STUDY). `usbhidd` in Redox base
is the model for exposing events via `inputd`.

## 2. Userland pieces for a phone UI

| Project | License | Activity | Framebuffer story | Redox fit |
|---|---|---|---|---|
| **Slint** | GPL-3.0-only OR Slint Royalty-free 2.0 OR Slint Software 3.0 (crates.io). **Royalty-free option excludes embedded**. | 2026-10-24 | Best-in-class software renderer, linuxfb, MCU targets. | **FLAG:** license is a real problem for a shipped embedded device unless we go GPLv3 or buy a license. Do not make it a dependency by default. |
| **iced** | MIT | 2026-10-21 (crate 0.14, 2025-12) | wgpu or `tiny-skia` software renderer; Redox has a `wip/wayland/iced-wayland` recipe and COSMIC (libcosmic, MPL-2.0) is built on iced. | Good: software rendering via `tiny-skia`. Redox already packages `libcosmic`. |
| **egui** | MIT OR Apache-2.0 | 2026-10-22 | `egui` + `softbuffer`/custom painter; immediate mode, easy touch. | Very portable; good for first UI. |
| `tiny-skia` | BSD-3-Clause | 2026-02 | CPU 2D rasterizer, no GPU or libc quirks | Best low-level base for our own compositor. |
| `softbuffer` | MIT OR Apache-2.0 | 2025-12 | Raw pixel buffer presentation; Redox recipe `softbuffer-wayland` exists, Orbital backend feasible | OK |
| `cosmic-text`, `fontdue` | MIT OR Apache-2.0 (fontdue also Zlib) | 2026-04 / 2026-10 | Text shaping and glyph raster | `cosmic-text` is already a Redox recipe. |
| Floem (lapce) | MIT | 2026-09-24 | wgpu/vello | Needs GPU. Later. |
| Dioxus / Blitz | Apache-2.0 / MIT style (GitHub: Apache-2.0) | 2026-10-24 | Blitz = Rust HTML/CSS renderer on vello (GPU) | Later, if we want web-style UI. |
| Smithay (compositor toolkit) | MIT | 2026-10-20 | Wayland compositor library, uses libinput/DRM/GBM | Needs Linux DRM/evdev. Redox has `libwayland`, `wlroots`, `smallvil`, `anvil` (Smithay's demo) as **wip** recipes, so a Wayland path is being explored by others. Not ready for us; the Orbital path is simpler. |
| Redox Orbital | MIT | 2026-09-27 | Native Redox display server; GTK3 backend added Jun 2026 | Our default UI base. Touch input support is the gap to check. |
| `smithay-client-toolkit` | MIT | 2026-07 | Wayland client | Only with a Wayland compositor. |

Redox recipes present in `redox-os/redox` `recipes/` (checked 2026-10-24): `wip/mobile/{phosh, phosh-mobile-settings, phosh-osk-stevia, android-tools}`, `wip/wayland/{niri, sway, wlroots, phoc, smallvil, anvil, libwayland, wayland-rs, winit-wayland, iced-wayland, libcosmic-wayland}`, `cosmic/*` (edit, files, settings, term, store), `libs/{mesa, libdrm, sdl2, cairo, pango, gstreamer, ffmpeg6, libopus}`, `wip/ai/{nnx, rustgpt, tgs}`, `wip/sound/{speech-dispatcher, mbrola, rodio-adjacent: rodioplay}`. "wip" means not working; treat as hints about what is feasible, not as ready packages.

### Audio, STT, TTS, LLM

| Project | License | Activity | Compiles without glibc? | Redox notes |
|---|---|---|---|---|
| `cpal` | Apache-2.0 | 2026-10-20 | Backends: ALSA, CoreAudio, WASAPI, etc. **No Redox backend.** | We would add a Redox audio-scheme backend (small). `rodio` (MIT OR Apache-2.0) sits on top; Redox has `rodioplay` recipe. |
| `candle` (HF) | MIT OR Apache-2.0 (GitHub: Apache-2.0) | 2026-10-14 | Pure Rust CPU backend, NEON; no CUDA needed | Strong candidate: LLM + Whisper + other models in pure Rust, so it only needs `std` on relibc. Verify the `gemm`/`rayon`/`half` deps build on `aarch64-unknown-redox`. |
| `burn` | MIT OR Apache-2.0 | 2026-10-24 | Pure Rust; `ndarray` or `wgpu` backends | OK. |
| `tract` | MIT OR Apache-2.0 | 2026-10-15 | Pure Rust ONNX inference | Good for small STT/VAD/wakeword models. |
| `rten` | MIT OR Apache-2.0 | 2026-10-02 | Pure Rust ONNX runtime | Same. |
| `ort` (ONNX Runtime bindings) | MIT OR Apache-2.0 | 2026-10-19 | Needs C++ ONNX Runtime | Harder (C++ stack). |
| `llama.cpp` | MIT | 2026-10-24 | C/C++, no glibc hard dependency, builds for aarch64, NEON/i8mm | Needs `libstdc++` on Redox (recipe `libstdcxx-v3` exists). Best CPU performance today. Bindings: `llama-cpp-2` (MIT OR Apache-2.0, 2026-10-15), `llama-cpp-rs` repo Apache-2.0. |
| `whisper-rs` | **Unlicense** (public domain), **archived 2025-07 on GitHub** though crates.io has 0.16.0 (2026-03) | | wraps whisper.cpp (MIT) | Use whisper.cpp directly or via `candle` Whisper. Check crate source repo has moved. |
| `sherpa-onnx` | Apache-2.0 | 2026-10-24 | C++ with ONNX Runtime; STT + TTS + VAD + keyword spotting, built for embedded aarch64 | Best all-in-one voice stack for a phone. `sherpa-rs` (MIT) is archived 2026-03. |
| `piper-rs` | MIT | 2026-05 | TTS via ONNX | OK; TTS voices have their own licenses, check each. |
| `mistralrs` | MIT | 2026-04 | Rust LLM runtime | Heavy deps, harder on Redox. |

Reality check for Redox + relibc: pure-Rust crates that only need `std` are the realistic first ports
(candle, tract, rten, tiny-skia, egui, iced). C/C++ stacks (llama.cpp, sherpa-onnx, onnxruntime) are
possible via cookbook recipes but need `libstdc++` and pthread/atomics behavior in relibc to be solid.
The Redox AI recipes in `wip/ai` (nnx, rustgpt, tgs) show someone has tried, status unknown.
Snapdragon 845 has 4x Kryo 385 Gold + 4x Silver (A75/A55-class, ARMv8.2 with dotprod) **(from memory)**:
use int8 quantization with NEON dotprod; the Hexagon 685 DSP and Adreno 630 are unusable without Linux-style
firmware/driver work.

## 3. Other Redox userland worth knowing

- Orbital (display server, MIT) plus `orbutils`, `orbterm`, `installer-gui` in `recipes/gui`.
- COSMIC apps on Redox: `cosmic-edit`, `cosmic-files`, `cosmic-settings`, `cosmic-term`, `cosmic-store`, `cosmic-text`, `libcosmic` (MPL-2.0). `cosmic-comp` is GPL-3.0 (GitHub): do not copy.
- Networking: `netstack` (in `base`), `dhcpd`, plus `curl`, `openssh`, `redox-ssh`, `rsync`, `wget`, `nginx` recipes. Wireless: none known. Wi-Fi on SDM845 is a Qualcomm WCN3990 (ath10k-style SNOC) needing firmware and a driver; no BSD implementation seen. Plan for USB-gadget networking or USB-Ethernet first.
- Dev tools on-device: `rust` and `gitoxide` recipes, `redoxer`, `llvm21`, `python312`. Good for self-hosting later.
- Phosh already has a Redox wip recipe, evidence a Wayland mobile shell is imaginable, but GPL (see section 4).

## 4. Other phone OS efforts: what to reuse

| Project | License | OnePlus 6 / SDM845 relevance | Reuse notes |
|---|---|---|---|
| **postmarketOS** `pmaports` | APKBUILD packaging is MIT-style per device package (`license="MIT"` in `device-oneplus-enchilada`, confirmed); kernel/patches carry their own licenses | Device pkg `device-oneplus-enchilada` v31, maintainer Casey Connolly; kernel `linux-postmarketos-qcom-sdm845` (mainline-based). Active 2026. | Gold mine of *facts* (see below). `pmbootstrap` is GPL-3.0 **(from memory)**. |
| **edk2-msm** (edk2-porting) | **BSD-2-Clause**, `GPLDrivers/` dir is GPL-2.0 | Has `Platform/Oneplus/sdm845/{enchilada,fajita}.dsc` + FdtBlob. Last push 2026-03-07. 2598 stars. | **Highest-value find.** It gives UEFI on the OnePlus 6 (the OP6 "Renegade Project" firmware), including a GOP framebuffer, UFS and USB access, and passes a DTB. Redox's aarch64 UEFI bootloader can then run unmodified. Verify exactly what works on OP6 before relying on it (wiki.renegade-project.cn). Don't link the `GPLDrivers`. |
| **lk2nd** | BSD-3 style (GitHub "other") | MSM8916-era, SDM845 support unverified | Alternative fastboot-based loader. |
| Phosh | GPL-3.0-or-later **(from memory)** | Runs on OP6 on pmOS | GPL. Study UX only. |
| Plasma Mobile | GPL/LGPL mix **(from memory)** | Runs on OP6 | Study only. |
| FuriOS (Furilabs) | Mixed (Droidian-based, libhybris) **(from memory)** | Targets Android-vendor-kernel devices | Mostly irrelevant, Halium-style. |
| Ubuntu Touch / Lomiri (UBports) | GPL-3/LGPL-3 **(from memory)** | OP6 is a supported device (Halium) | Study only. |
| Sailfish | Mer/Lipstick open (LGPL-2.1-ish / BSD parts), Silica closed **(from memory; Lipstick GitHub shows no license metadata)** | Not OP6-focused | Skip. |
| LuneOS / webOS-ports | Apache-2.0 **(from memory)** | Old | Skip. |
| B2G / KaiOS | MPL-2.0 **(from memory)**; B2G/gaia repos inactive since 2020-21 | Feature-phone class | Skip. |
| unl0kr (pmOS on-screen keyboard unlock) | GPL-3.0 (droidian fork shown) | Framebuffer touch UI with LVGL | Design reference only. |

### OnePlus 6 hard facts from postmarketOS (pmaports `main`, read 2026-10-24)

- Panel 1080x2280, boot via fastboot, boot image with **appended DTB** (`qcom/sdm845-oneplus-enchilada`), flash offsets base 0x0, kernel 0x8000, ramdisk 0x01000000, second 0xf00000, tags 0x100, page size 4096. UFS storage with **4096-byte sectors**. `sparse` flashing.
- Console `console=ttyMSM0,115200`. Comment notes it works around a race with display init (freedreno issue 46).
- initfs modules show the SoC pieces needed: `i2c_qcom_geni`, `rmi_core`/`rmi_i2c` (touch), `qcom_spmi_haptics`, `qcom_spmi_rradc`, `qcom_smbx` (charger), `bq27xxx_battery_i2c` (fuel gauge).
- Depends on `firmware-oneplus-sdm845` (proprietary blobs for the adsp, venus, cdsp, wifi, GPU zap), `hexagonrpcd` (sensors via ADSP), `soc-qcom-modem`, `soc-qcom-qbootctl` (A/B slot control; **slot metadata must be set or the bootloader boots the other slot**), `alsa-ucm-conf-sdm845` (audio routing via Q6 DSP), `systemd-boot`.
- Another repo, `nia-e/sdm845-abootctl` (GPL-3.0, stale 2021), documents the same A/B control.

I could not fetch the pmOS wiki status table (the wiki URL redirected to an unfamiliar host, so I did
not follow it). Use the wiki feature table manually before planning drivers.

## 5. Gaps and cautions

- No Rust driver exists anywhere for GENI, UFS, DWC3, SPMI, TLMM or MDSS. Expect to write them; the BSD ports are the reference.
- No Redox entry for Qualcomm. The first milestone (UEFI -> Redox kernel -> serial/framebuffer output) is plausible via edk2-msm but unproven; edk2-msm's last push is 2026-03.
- Qualcomm bring-up depends on proprietary firmware blobs from the phone's own partitions (modem, adsp, wifi, GPU zap). We cannot redistribute them.
- Unverified (marked "from memory"): licenses of Phosh, Plasma Mobile, Ubuntu Touch, Sailfish, LuneOS, B2G, Fuchsia, seL4 platform list, whisper.cpp/sherpa model licenses. Check before shipping.
- Redox's `fdt` crate is MPL-2.0 (file-level copyleft) and a pinned git rev; acceptable as a dependency.

## 6. Top 10: take this first

1. **edk2-msm `Platform/Oneplus/sdm845/enchilada.dsc` + FdtBlob** (BSD-2, github.com/edk2-porting/edk2-msm). Gets UEFI, framebuffer, UFS/USB access and a DTB on the phone so Redox's existing `aarch64-unknown-uefi` bootloader is the boot path. Confirm OP6 feature status first.
2. **postmarketOS `device-oneplus-enchilada` deviceinfo/APKBUILD + the `linux-postmarketos-qcom-sdm845` kernel config** (MIT packaging): offsets, A/B (`qbootctl`), 4K UFS sectors, firmware list, DTB name.
3. **The compiled mainline `sdm845-oneplus-enchilada.dtb`** as boot data (do not copy the GPL DTS source). Redox's kernel already parses DTBs; extend `src/dtb` for `qcom,*` nodes.
4. **OpenBSD `qcuart_fdt.c` + `qcspmi.c` + `qcgpio_fdt.c`** (ISC): port to Rust for the GENI debug UART first (console), then pinctrl/TLMM and SPMI (PMIC buttons, power key, charger). Adjust for SDM845 versions.
5. **`tock-registers` + `safe-mmio`** (MIT/Apache) as the register-description style for all new drivers; plus **`embedded-hal`** traits so a GENI I2C driver unlocks existing sensor/touch crates.
6. **OpenBSD `qciic_fdt.c` / FreeBSD `geni_iic.c`** (ISC / BSD-2) for GENI I2C, then a small RMI4 touch driver (Linux `rmi4` as behavior reference only, GPL) feeding Redox `inputd`.
7. **`embassy-usb` + `embassy-usb-driver`** (MIT OR Apache-2.0) over a new DWC3 device-mode driver (OpenBSD `qcdwusb.c`, NetBSD `dwc3_fdt.c`): CDC-ACM console + CDC-NCM network gives a debug link from the host without Wi-Fi.
8. **OpenBSD `ufshci_fdt.c` + Redox `nvmed`/`driver-block` structure** (ISC): UFS storage for the Redox root filesystem. Highest-effort driver; keep boot from RAM disk/USB until it is ready.
9. **UI stack: `tiny-skia` (BSD-3) + `cosmic-text`/`fontdue` + `egui` or `iced` (MIT)** on Orbital/UEFI framebuffer with software rendering. Avoid Slint unless licensing is resolved (GPLv3 or paid for embedded).
10. **Voice stack: `sherpa-onnx` (Apache-2.0; STT, TTS, VAD, wake word) and/or `candle` (MIT/Apache; pure Rust Whisper and small LLMs) plus `llama.cpp` (MIT)**, with a `cpal` Redox audio-scheme backend. Start by proving a pure-Rust `candle` or `tract` model runs on `aarch64-unknown-redox` in QEMU; audio hardware (Q6 DSP via ADSP firmware) is the hard part, so use a USB audio dongle/gadget first.

## Source index

- Redox: gitlab.redox-os.org/redox-os/{kernel,base,redox,bootloader}; HARDWARE.md; Phoronix Redox monthly reports (Jan/Apr/Jun/Aug 2026).
- OpenBSD: github.com/openbsd/src `sys/dev/fdt/`; FreeBSD: github.com/freebsd/freebsd-src `sys/dev/iicbus/controller/qcom/`; NetBSD: github.com/NetBSD/src `sys/dev/fdt/dwc3_fdt.c`.
- edk2-msm: github.com/edk2-porting/edk2-msm; docs wiki.renegade-project.cn.
- postmarketOS: gitlab.postmarketos.org/postmarketOS/pmaports (`device/community/device-oneplus-enchilada`).
- Linux license headers: raw.githubusercontent.com/torvalds/linux/master (paths in section 1a).
- Asahi: github.com/AsahiLinux/linux `drivers/gpu/drm/asahi`; asahilinux.org/about.
- Crates data: crates.io API (queried 2026-10-24). Slint licensing: crates.io + slint.dev agreements.
