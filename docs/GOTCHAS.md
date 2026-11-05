# Gotchas — traps that will bite anyone reproducing this

Every worker appends here when something non-obvious costs time. Format: **symptom** → cause → fix/workaround
(where it lives). Newest at the bottom of each section. Companion to the pmOS-era log
`voice-stack-tryout/docs/phone-os/ISSUES.md`.

## Redox kernel on aarch64 / Qualcomm
- **Any Qualcomm interrupt above 255 crashed the old kernel** → IRQ/virq numbers were stored as `u8`; SDM845
  SPIs map to virqs ≥ 386 (GENI uart9 = SPI 354). The resolved full-width change is now
  `patches/kernel-series/0007`–`0009`. Found by K2.
- **DT-only boot (`acpi=off`) panics `MissingRequiredProperty("interrupt-cells")`** → kernel didn't inherit
  `#interrupt-cells` through `interrupt-parent`. Fixed in `patches/kernel-series/0001-…` (original branch `op6/dt-acpi-off`).
- **Devices with `interrupts-extended` were rejected** ("multi-parented device not supported") — the phone's
  TLMM-routed IRQs (e.g. touch) need it. Fixed in the same patch.
- **Only one CPU on GICv3** → Redox secondary-CPU bring-up supports GICv2 only; SDM845 is GICv3. Run `-smp 1`
  until GICv3 SMP is added (plan A3).
- **The phone's interrupt path has two Qualcomm controllers in front of the GIC** (PDC for wake IRQs, TLMM for
  GPIO IRQs) — neither exists in Redox (task K1).
- **No SMMU/IOMMU driver** → device DMA behaviour depends on how firmware left `apps_smmu`; check before any DMA
  driver (plan A11).

## Redox build / QEMU on an Ubuntu host (`docs/workbench/W0-BUILD.md`)
- Podman `:Z` volume suffix fails under this host's runc → build with `USE_SELINUX=0`.
- Upstream `--pids-limit=-1` became `pids.max=1` → container can't fork; pass `--pids-limit=1024`.
- Cookbook TUI panics with `Inappropriate ioctl for device` when output is redirected → `CI=1`.
- `make qemu` doesn't find Ubuntu's `/usr/share/qemu-efi-aarch64/QEMU_EFI.fd` → run QEMU directly with `-bios`.
- Disks on aarch64 QEMU: virtio-blk panics (`MSI-X is not implemented on this architecture`); NVMe times out in
  init; AHCI driver missing from initfs → use the live/initfs path or the drive-free embedded payload (K3).
- "Live" mode still reads the image from a drive → true drive-free boot needs the embedded kernel+initfs payload
  (`patches/bootloader`, `patches/base`, K3).
- **`spmid` can fault before its probe on QEMU** → K7's initial daemon mapped fixed SDM845 MMIO windows before checking for hardware. → K8 reads `/scheme/kernel.dtb` and skips the daemon when `qcom,spmi-pmic-arb` is absent, before any MMIO mapping (`base-k8/drivers/spmid/src/main.rs`).
- **Redox target `spmid` build rejects `FdtError` with `?`** → the pinned `fdt` crate's error does not implement `std::error::Error`. → Convert it to a descriptive string before returning from the daemon (`base-k8/drivers/spmid/src/main.rs`).
- **K8 bootloader build fails at `make -C .../build`** → the fresh bootloader worktree has no ignored `build/` directory. → Create it before invoking Make (`scripts/integrate.sh`).
- **Patch-present check reapplies an already integrated patch under `set -o pipefail`** → `git log | grep -q` can make `git log` exit on SIGPIPE after `grep` finds a match. → Capture commit subjects first and search the captured text (`scripts/integrate.sh`).
- **K8 EFI hash changes on repeated builds of the same commits** → the generated initfs/EFI bytes are not stable across observed runs; the exact source of the difference is unisolated. → Refresh `phone-boot/SHA256SUMS` for each build and keep QEMU evidence tied to that exact payload (`scripts/integrate.sh`, `docs/workbench/K8-INTEGRATION.md`).
- **K9 touch patch will not apply directly after K7** → it adds a second `drivers/mmio-replay/` and edits an older `Cargo.lock`. → Apply `patches/integration-2/base/`, which retains K7's crate and has a regenerated lockfile (`scripts/integrate.sh`).
- **An edited running Bash script can fail with a partial command** → Bash reads script input as it executes, and changing `scripts/integrate.sh` during a build produced `on-2/bootloader: No such file or directory`. → Finish edits before starting the script; rerun from its clean, idempotent entry point.
- **A synthetic simplefb at 0x9d400000 crashes vesad in QEMU virt** → that address is ordinary guest RAM, so Redox rejects `physmap` of an allocator frame (`Operation not permitted`). → For the ramfb QEMU fixture, describe the GOP framebuffer at 0xbc430000, 800x600, stride 3200; the phone's 1080x2280 address remains a separate hardware gate (`docs/workbench/K8B-SIMPLEFB.dts`).
- **vesad discovers the correct ramfb but then aborts with `No such device`** → its graphics scheme registers with `inputd`, which K8's minimal initfs omitted. → Package `inputd` and its service, and require it before vesad in the RAM boot target (`base-k8b/Makefile`, `init.initfs.d/90_ramboot.target`).
- QEMU's EDK2 firmware hands the OS **ACPI, not a DTB** — the EFI diagnostic app reports "DTB absent" in QEMU;
  U-Boot on the phone passes a DTB. Don't read "no DTB" in QEMU as a bug.

## Licensing / upstream
- **Redox does not accept LLM-generated contributions** (`CONTRIBUTING.md:30`) → this is a downstream fork.
- **U-Boot is GPL-2.0 overall**; only some files are BSD-3 (✔ `drivers/clk/qcom/clock-sdm845.c`,
  `drivers/spmi/spmi-msm.c`). Check the SPDX header of every file before using it as a permissive reference.
- **edk2-msm** is BSD-2 except `GPLDrivers/`. `fdt` crate is MPL-2.0 (use `dtb-walker`, MIT, for new code).
  **Slint**'s royalty-free license excludes embedded devices. RedPlanetHQ `core` is AGPL despite a "Core License"
  title. Open Interpreter: the new Rust version is Apache-2.0, the classic Python one AGPL.
- OpenBSD's Qualcomm drivers (ISC) target SC7180/SC8280XP/X1E, not SDM845 — adapt register families, don't
  assume drop-in. FreeBSD's dwc3 is host-mode only; Embassy's CDC-NCM runs on DWC2 — the DWC3 *device-mode*
  driver must be written.

## OnePlus 6 hardware facts that are easy to get wrong
- Touch is Synaptics RMI4 **s3706b** (not Goodix); panel **samsung,sofef00-ams628nw01** (6T uses s6e3fc2);
  PMICs PM8998/PMI8998/PM8005 (no "PM845"); speaker amp MAX98927; USB is USB2 **peripheral-only** per DT.
- Volume keys are **PM8998 GPIOs 5/6** (PMIC, via SPMI), power key is PM8998 PON — not TLMM GPIO keys.
- Brightness on this AMOLED is **MIPI DCS** to the panel, not PMI8998 WLED (disabled).
- The boot framebuffer is `cont_splash_mem` @0x9d400000, 1080×2280 a8r8g8b8, stride 4320 (no-map region).
- Qualcomm's hypervisor owns EL2: kernels run at **EL1**; no KVM/driver-VM tricks.
- A reboot stops at the LUKS passphrase prompt on the phone itself (our pmOS install) — no remote recovery.

## SPMI / PM8998 K7
- **`aarch64-unknown-redox` build stops at missing `core`/`std`** → the default stable toolchain has no std for that target and rustup distributes no precompiled component for it (only `x86_64-unknown-redox`). → WORKAROUND FOUND, NOT AN UPSTREAM FIX (B5, 2026-10-24; wording per Cursor audit B5-4): rustup still distributes no precompiled aarch64-redox std; the project's own cookbook prefix carries it. Use the cookbook toolchain the project already built at `forks/redox/redox/prefix/aarch64-unknown-redox/rust-install` — put its `bin/` on `PATH` and set `LD_LIBRARY_PATH` to the sysroot `lib/` (for `libLLVM.so.21.1`); it carries the prebuilt `rust-std-aarch64-unknown-redox` rlibs. Reproduction and artifact proof: `docs/workbench/B5-REDOX-TARGET.md`. Host-side `cargo check --features host-compile` remains valid for quick iteration.
- **No `exampled` source in the base checkout** → base references it as a separate upstream repository in `drivers/README.md`; no local daemon skeleton exists. → Use base's `inputd::ProducerHandle` and `common::PhysBorrowed` interfaces directly, with a small standalone daemon (`drivers/spmid`).
- **PM8998 GPIO key numbers are one-based** → the OnePlus DT names GPIOs 5/6, while peripheral register banks use zero-based `0x100` strides. → Read GPIO5 at `0xc410` and GPIO6 at `0xc510`; host strict replay checks both (`crates/spmid/tests/arbiter.rs`).

## Display handoff (K4, Cursor)
- **Boot hangs at the Redox bootloader resolution menu** when a GOP exists — it waits for Enter with **no
  timeout**. On the phone there is no keyboard → add a timeout/auto-select (handled in K8).
- edk2-sdm845's enchilada DTB has **no `/chosen/framebuffer` and no splash region** → with that firmware only the
  GOP path works; U-Boot + the mainline DTB (`cont_splash_mem`) is the DT path.
- The kernel's early console depends on GOP; DT-only (no GOP) leaves the panel blank until `vesad` starts.
- simplefb support accepts only `a8r8g8b8`/`x8r8g8b8`. The `fdt` 0.1.5 crate mis-finds `/chosen` when the DTB has
  `FDT_NOP` tags (patched in `patches/base-simplefb/0002`); the same lookup bug may affect `bcm2835-sdhcid`.
- QEMU ramfb through edk2 tops out at 1024×768 — 1080×2280 was proven by RAM dump, not scanout.
## GENI I2C / RMI4 K9
- **Public “Synaptics RMI Interfacing Guide” search hit is RMI3** → it does not specify F12 Data1 register placement or s3706b slot count. → The RMI4 core requires an oracle-validated descriptor and `rmi4d` refuses to start without explicit offset/slot arguments (`specs/rmi4-touch.md`, `drivers/rmi4d`).
- **K9 base branch predates K7's base-side `mmio-replay` copy** → the project checkout has C1, but this base worktree cannot resolve that external path when integrated. → Copy C1 into `drivers/mmio-replay/` in K9 with provenance; re-deduplicate when merging K7 and K9.
- **Host Cargo check does not compile Redox-only MMIO or inputd paths** → installed targets are Linux only. → Host replay tests prove register logic, while a Redox aarch64 toolchain build and QEMU/device run remain explicit integration gates (`docs/workbench/K9-TOUCH.md`).
- **Base Cargo check fails from the project directory on stable Rust (`E0554` in `redox-scheme`)** → `base-k9/rust-toolchain.toml` pins nightly, but `--manifest-path` does not select that toolchain when the current directory is the project checkout. → Run from the base directory or use `cargo +nightly-2026-05-24`; the successful retry is in `docs/workbench/K9-TESTS.txt`.
## Rust crates (trace-events, wave 1)
- **`Some(serde_json::Value::Null)` round-trips to `None`** → serde treats an explicit JSON `null` in an
  `Option` field as absent. Don't build test fixtures with `json!(null)` in optional fields; document the
  equivalence instead. Found by C2.
- **Integration tests can't name `alloc::…`** (E0433) → test crates link std; use plain `Vec`/`String` in
  `tests/`. They *can* `use serde_json` directly — cargo passes the package's deps to test targets.
- **`cargo` is not on PATH on the build boxes** → `~/.cargo/bin/cargo`.

## IRQ width (K6, Cursor)
- After widening IRQs, **QEMU `virt` tops out at virq 287** — it cannot exercise the phone's ≥ 386 range; expect
  `serial_port virq = 386` in the phone boot log as the real check.
- Merging `kernel-irq-width` with `kernel-irqchips` (K1) conflicts in 3 places (queue length, `IRQ_DESC_COUNT`
  vs K1's literal 2048, generic-timer line) → set `IRQ_DESC_COUNT = 2048`; test `every_irqchip_virq_has_a_queue`
  fails if the queue table and descriptor count disagree.
- **`git -C <worktree> format-patch -o patches/...` writes into the worktree** → `-o` is relative to the
  command's working directory after `git -C`; generated patches appeared under the kernel/base/bootloader
  worktrees instead of this project. → Use an absolute output path or copy the generated patches into this
  project's `patches/` and remove only the accidental untracked directories.
- `cpu_stats::add_irq` still takes `u8` (commented out on aarch64/riscv64) — harmless today, remember it.

## Wasm sandbox (B5, Cursor)
- **A runaway Wasm loop crashed the host with `fatal runtime error: stack overflow`** instead of stopping on fuel (50k fuel passed, 1M aborted) → `wasmi` with `default-features = false` drops `auto-dispatch`, so it uses tail-call dispatch even at opt-level 0, where LLVM does not emit sibling calls and every executed instruction grows the host stack. → Always enable wasmi's `auto-dispatch` (or `portable-dispatch`) when trimming features (`crates/sandbox/Cargo.toml`); run tests in debug and release.
- **A custom `wasmi::ResourceLimiter` cannot name its error type** → wasmi 2.0.0 keeps `LimiterError` in a private module. → Depend on `wasmi_core = "=2.0.0"` directly for `wasmi_core::LimiterError` (same crate, same type).
- **The first call of a Wasm function used more fuel than later calls (450 vs 261)** → wasmi's default `CompilationMode::LazyTranslation` bills compiling each function to the fuel of whoever calls it first. → Use `CompilationMode::Eager` so a service's fuel budget doesn't depend on call order (`crates/sandbox/src/service.rs`).
- **Rust guests carry their shadow stack inside linear memory** → a 1 MiB `-zstack-size` gives 17 initial pages, over the sandbox's default 16-page (1 MiB) cap, and the load is refused. (rustc 1.99's default stack produced 1 page.) → Pin `-C link-arg=-zstack-size=16384` in the guest's `.cargo/config.toml` (`crates/sandbox/guests/hello-rs`).

## K12 oracle traces
- **`rwmmio` addresses cannot be replayed directly** → Linux prints kernel virtual addresses, while `mmio-replay` expects physical addresses. → Require an independently verified per-boot VA→PA mapping and fail conversion on an empty mapped window (`scripts/oracle/trace/convert.py`).
- **Regmap/QRTR/APR trace lines are not MMIO accesses** → they describe bus or firmware protocols, and some kernels expose only metadata or no tracepoint group at all. → Preserve raw protocol captures separately; convert only mapped `rwmmio` accesses and report missing payloads as a gap (`scripts/oracle/trace/README.md`).

## Remote-processor core (K13)
- **A rejected duplicate SMEM model insert silently replaced the original item** → `BTreeMap::insert` mutates before returning the old value. → Check `contains_key` before insertion; the negative host test now confirms the original item survives (`crates/qcom-rproc/src/smem.rs`).
- **Generic SDM845 DT declares ADSP PAS but marks it disabled** → treating a `compatible` match as readiness would start secure calls without board enablement, firmware, clocks or memory. → `PasId` requires explicit caller allowlist; hardware adapter must check the live DT and prerequisites (`specs/qcom-rproc.md`).
- **Global SMEM item lookup read the wrong TOC entry during the first host fixture** → the OpenBSD header has four u32 fields after `version[32]`, not five. → Set TOC offset to 208 bytes and test lookup against a byte-level fixture (`crates/qcom-rproc/src/smem.rs`).
## UFS read-only core (K10, Cursor)
- **Golden MMIO trace saved as `.jsonl` silently vanishes from the commit** → the repo `.gitignore` ignores
  `*.jsonl` everywhere. → Keep fixtures as `*.trace` (same JSONL content; `crates/ufs/tests/fixtures/`).
- **`Ufshc::new(&mut model, …)` does not compile** → `mmio-replay` implements `Mmio` for `MemModel`,
  `Recorder` and `Replayer` but has no blanket `impl Mmio for &mut T`. → Tests add `impl Mmio for &mut UfsModel`
  (legal: `&mut` is a fundamental type) and pass `Recorder` by value, recovering it via `into_parts()`.
- **A register trace cannot replay a DMA driver on its own** → UFS responses arrive in DMA memory, not
  registers, so a strict `Replayer` alone returns "done" with an empty response. → Wrap it in a harness that
  runs the device model on each `UTRLDBR` write (`crates/ufs/tests/replay.rs`); a phone capture must likewise
  record the response UPIUs (ftrace `ufshcd_upiu`) next to the register reads.
- **Linux re-initialises the QMP PHY around every HCE enable; we cannot** → no PHY driver; the GCC BCR reset
  also resets the PHY. → Never assert `GCC_UFS_PHY_BCR`; after HCE the driver polls DME `MPHY_TX_FSM_STATE`
  (0x41) == HIBERN8 and fails with `PhyNotReady` instead of hanging in link startup (`specs/ufs.md` §2, §8).
- **The local Linux checkout has no `include/ufs/`** (sparse) → UniPro attribute IDs (TX_LCC_ENABLE 0x2C,
  PA_LOCAL_TX_LCC_ENABLE 0x155E) are not greppable locally; raw.githubusercontent rate-limits (HTTP 429).
  → Fetch from `git.kernel.org/.../plain/include/ufs/unipro.h`; kernel.org HEAD == local `5b0764a`.
- **UTRLCLR is inverted**: write **0** in a slot bit to clear it (`~mask`), not the mask; `UTRLCNR` (0x64) is
  UFSHCI 3.0+ only and must not be written on SDM845 (reports UFSHCI 2.0) even though FreeBSD writes it.
- **pmOS root (LUKS) lives on this UFS** → never reset the host controller from Linux for an oracle capture;
  passive devmem reads, sysfs descriptors and ftrace only (`specs/ufs.md` §7).
## Timeline app (C4, wave 1)
- **OrbClient `Window::events()` blocks the frame loop for ~30 s** → in sync
  mode it calls `wait_event()` (blocking); the first calls only returned because
  window-shown events were queued. → `window.set_async(true)` right after
  `Window::new`; then `events()` polls. (`apps/timeline/src/main.rs`)
- **Process never exits after `main` returns under `SDL_VIDEODRIVER=dummy`** →
  orbclient's `Window` Drop joins its event thread, and that join hangs with the
  dummy driver. → finish with `std::process::exit(0)` after the last screenshot.
- **Headless SDL runs need two things** → `sudo apt-get install -y libsdl2-dev`
  (pkg-config sdl2 missing otherwise) and `SDL_VIDEODRIVER=dummy`; off-screen
  `orbclient::Surface` works without any video init, so PNG evidence renders
  even when `Window::new` fails.
- **Hand-rolled PNG CRC vectors from memory are wrong** → my recalled IHDR CRC
  failed the test while the writer was correct. → verify vectors against
  `python3 -c "import zlib; print(hex(zlib.crc32(b'IHDR')))"` before hard-coding.

## B1 trace journal
- **A service filter cannot identify generic mic/AG-UI events from the old envelope**
  → `Event` had span IDs but no producer identity → add optional `service`, keeping
  older JSON valid and using the field for both filtering and rate limits.
- **A slow subscriber cannot own an unbounded delivery queue** → it could stall
  producers or grow RAM indefinitely → subscriptions keep only a cursor into the
  bounded ring and receive an exact `journal_dropped` event after a gap.
- **Blocking inside `SchemeSync::read` would freeze producers** → one scheme request
  loop also handles `trace:publish` writes → return `EAGAIN` on an idle reader;
  a future async scheme implementation must add readiness notifications.
## USB device mode / DWC3 (K11, Cursor)
- **`embassy-usb` never calls `Bus::enable()`; the device stays `Unpowered`** → the stack only enables the bus after `Bus::poll` returns `Event::PowerDetected`, and on this phone VBUS is not sensed (QSCRATCH overrides it). → The driver reports `PowerDetected` once on the first poll (`crates/dwc3-gadget/src/controller.rs`, `take_bus_event`).
- **The DWC3 core DMAs through `apps_smmu` SID 0x740** (`sdm845.dtsi:4236`) → event ring, TRB and buffer addresses are IOVAs, equal to physical addresses only if firmware left that stream in bypass; Redox has no SMMU driver. → Check SMR/S2CR for SID 0x740 in the oracle capture before the first phone run (`specs/dwc3-gadget.md` §5).
- **No permissive source for most QSCRATCH glue** → OpenBSD `qcdwusb.c` (at `20cb344`) only enables power/clocks/resets; Fuchsia sets just `HS_PHY_CTRL` bit 20; `GENERAL_CFG` (UTMI as pipe clock), `SW_SESSVLD_SEL` and `SS_PHY_CTRL` appear only in GPL Linux/U-Boot. → Used as oracle facts, no code copied, flagged for oracle capture (`specs/dwc3-gadget.md` §3, §8).
- **EP0 stays at 512-byte packets after enumeration starts** → the core's EP0 is configured with MPS 512 before the speed is known; at high speed it must be re-issued as DEPCFG(modify) with 64 on connect-done, or control transfers packetize wrongly. → `on_connect_done` (the model asserts EP0 MPS 64).
- **Linux and Fuchsia disagree on the GEVNTCOUNT acknowledgement** → Fuchsia also sets bit 31 (`EVNT_HANDLER_BUSY`, DWC_usb31 only); Linux writes the plain byte count. → Plain count (`controller.rs::poll`); confirm against the oracle trace.
- **A timed-out endpoint command made a pending read retry Start Transfer on every poll** → the async read treated a failed start as "still idle". → A failed start now returns `EndpointError::Disabled` and records `Dwc3::fault` (`driver.rs`, test `stuck_command_at_runtime_is_bounded`).
- **A host-side test helper hung on the CDC-NCM notification** → 8 bytes on an 8-byte interrupt endpoint is a full packet, so "read until short packet" waits forever. → Read interrupt endpoints one packet at a time (`tests/common/mod.rs::in_packet`).
- **`cargo` in `op6-refs/embassy` silently installs Rust 1.97** → that checkout's `rust-toolchain.toml` pins it and rustup auto-installs. → Depend on crates.io `embassy-usb-driver` 0.2.2 / `embassy-usb` 0.6.0 instead of path dependencies into `op6-refs`.
- **CDC-ACM in `embassy-usb` 0.6.0 needs a `critical-section` implementation** (`CriticalSectionMutex` for line coding) → link error without one. → Tests use `critical-section` with feature `std`; the Redox daemon needs the same (std is available there).
- **A module named `core` in a `no_std` crate shadows `::core` at the crate root** → renamed to `controller` (`crates/dwc3-gadget/src/controller.rs`).
## voice-bridge (C5, wave 1)
- **A line-based Wyoming fake server dies with `JSONDecodeError: Extra data`** → the data block is NOT
  newline-terminated: the header declares `data_length`/`payload_length` and those blocks are read as
  exactly that many bytes (no 4-byte length prefix either). Binary audio payloads contain `0x0A` bytes,
  so splitting on `\n` desyncs the stream. → Parse by the header's declared lengths (see
  `crates/wyoming/src/event.rs` `decode`).
- **`TcpStream::split()` does not exist on this toolchain** (rustc 1.99 std) → use `stream.try_clone()`
  for the writer and `BufReader::new(&mut stream)` for the reader (`apps/voice-bridge/src/main.rs`).
- **`wyoming::events::Eventable` is not dyn-compatible** (its methods require `Self: Sized`) → pass
  typed events through generic helpers (`fn send_ev<Wr: Write, E: Eventable>`), never `&dyn Eventable`.
- **A RIFF reader mis-reads the next chunk tag after an odd-size unknown chunk** → RIFF pads odd chunks
  to word boundaries; the pad byte is not counted in the chunk size. → Skip `(size + 1) & !1` bytes
  (`apps/voice-bridge/src/wav.rs`).
- **A TTS reply whose total audio is an odd byte count writes a WAV the reader then rejects** → an odd
  total ends mid-16-bit-sample; the writer's pad byte is legal RIFF but the PCM16 reader (rightly)
  refuses a half-sample. → `run_tts` rejects odd totals explicitly before writing anything.
## C5 merge onto current main (orchestrator, 2026-10-25)
- **`apps/voice-bridge` compiled on its own branch but failed on `main`** (`E0063: missing field
  \`service\` in initializer of \`trace_events::Event\``) → B1's `169f277 trace-events: add producer
  identity and drop notice` added the `service` field to the envelope after C5 branched; a struct
  literal without `..Default::default()` breaks at merge time, not at branch time. → `Journal` now
  stamps `service: Some("voice-bridge")` on every event (`with_service` to override), plus two tests
  asserting the producer id reaches the wire. → **Merge each branch onto current `origin/main` and
  run the tests there before calling it green; a green branch is not a green merge.**
## A2UI renderer (C6, wave 1)
- **`EventOption::Mouse { x, y }` does not compile** (E0769 "tuple variant
  written as struct variant", and `Button` "has no field named `left`") →
  orbclient's `Mouse`, `Button` and `TextInput` are **tuple** variants wrapping
  `MouseEvent`/`ButtonEvent`/`TextInputEvent`. → `EventOption::Mouse(m)` with
  `m.x`/`m.y`, `EventOption::Button(b) if b.left`, `EventOption::TextInput(t)`
  where `t.character: char`. (`crates/a2ui-render/src/main.rs`)
- **`Window::new` does not return `None` when there is no display — it panics**
  (`called Result::unwrap() on an Err value: "x11 not available"`) → orbclient
  unwraps `SDL_Init` before it can fail into the `Option`. → Probe the
  environment first (`SDL_VIDEODRIVER`/`DISPLAY`/`WAYLAND_DISPLAY`) and take
  the off-screen path when none is set; for a real event loop with no screen,
  set `SDL_VIDEODRIVER=dummy`. (`display_available()` in the same file)
- **PNG decode corrupts only some rows** → the Paeth predictor computes
  `a + b - c`, which overflows `u32` (debug panic, silent wrap in release). →
  Do the predictor and the filter deltas in `i64`, cast once at the byte —
  in the decoder **and** in any test-side encoder. (`crates/a2ui-render/src/png.rs`)
- **A pixel test that compares `Color`s can pass while the images differ** →
  `orbclient::Color`'s `PartialEq` deliberately ignores alpha. → Compare
  `Color::data` (the raw `0xAARRGGBB` u32) in golden/pixel assertions.
  (`crates/a2ui-render/tests/golden.rs`)
- **"Is there text at this position?" assertions fail on a single scanline** →
  unifont strokes are 1 px and uneven, so one row of a glyph may be empty. →
  Count ink over the whole rect and assert a threshold (`> 100` lit px), not a
  single row or column. (`crates/a2ui-render/tests/view.rs`)
- **Assertions on a trace-events JSONL record read `Null`** → the record is
  flat but the vocabulary tag is **inside** `"payload"`:
  `{"seq":0,"ts_us":1,"payload":{"kind":"ag_ui","type":"CUSTOM","name":…}}`.
  → Print one real line before writing the assertions; check
  `v["payload"]["kind"]` / `v["payload"]["type"]`, not `v["kind"]`.
- **Every row of an A2UI `List` template renders empty** → this resolver has no
  notion of "the item itself": `{"path": "."}` is not a supported binding. →
  Template items must be objects with named keys and the template child binds
  relatively, e.g. item `{"text": "01. …"}` with `{"path": "text"}`.
  (`crates/a2ui-render/fixtures/broken.json`)
- **Headings cannot be made larger** → orbclient draws unifont at a fixed 8×16
  with no scaling or weight. → A heading is styled the only way this screen
  can: uppercase, orange, extra top spacing, and a rule under `h1`. The golden
  test pins that so it cannot regress into an invisible difference.
- **A golden test on PNG bytes breaks on unrelated tooling changes** → the
  encoder's zlib settings change with the crate version. → Decode the committed
  golden with the crate's own decoder and compare **pixels**; bless with
  `A2UI_BLESS=1 cargo test --test golden the_four_documents`, which rewrites
  the goldens and then fails on purpose so a bless run is never mistaken for a
  green comparison.

## svcd Service IR executor (B4, Cursor)
- **D1's battery test cannot pass if the simulator stops at the last event** → its second alert is expected 5 s *after* the final sample (`debounce_ms: 5000`; last event 500 000 ms, alert 505 000 ms). → `sim::drive` keeps the virtual clock running for the service's longest `debounce_ms` after the last event.
- **`max_wakeups_per_minute: 0.1` read as a token bucket fails D1's own battery test** (three wakeups 185–305 s apart). → svcd uses a sliding 60 s window allowing `ceil(limit)` wakeups and counts only non-user triggers (timer/sensor/bus), not voice/touch.
- **`total_duration_sec` became the string `"300"` and failed the integer state schema** → a template that is exactly one `{{ expr }}` must evaluate to a typed value; only mixed text interpolates (`src/template.rs`). D1 also mixes a second syntax, `"$.state.x"`, which is a typed path reference.
- **"Battery is down to 19.0 percent."** → serde_json prints `19.0`; interpolation prints integral floats without the decimal (`template::display`).
- **D1's timer speaks a normal-priority confirmation but requests only `audio.playback.alarm`** → a dialog accepts either playback capability (critical prefers alarm); a strict tts-only rule rejects the worked example.
- **D1's nav example is invalid**: irreversible speech without an idempotency key, bare `{{destination}}` (should be `params.destination`), and its `ui_action` test payload uses A2UI wire keys (`surfaceId`, `name`) while the schema text implies snake_case. → svcd rejects the first two (precise errors in `tests/validate.rs`) and accepts both payload shapes.
- **Shipping `jsonschema` in the daemon pulls ~90 crates (ICU, two regex engines) and a 6.5 MB binary** → it is a dev-dependency used as an oracle against D1's schema; the daemon validates with typed serde + hand-written checks (19 crates, 2.1 MB stripped).
- **`serde_path_to_error` reports "missing field" at the parent object** → append the field name parsed from the message so the pointer names the missing field.
- **serde turns an explicit JSON `null` into `None` for `Option<Value>`** → `replace` with `"value": null` looked like a missing value. → `#[serde(deserialize_with = "present")]` keeps null distinct from absent (`src/ir.rs`).
- **A compensated Wasm call that succeeded but cost too much fuel was never compensated** → the compensation was recorded after the budget check. → Record it as soon as the host call returns `Ok` (`src/exec/effects.rs`, `tests/budget.rs::wasm_fuel_over_budget_stops_the_service`).
- **The sentence matcher was quadratic on long utterances** (each slot tried every end offset) → slots end only at word/digit boundaries and utterances over 1024 characters go straight to `ovos.intent.fallback`.
- **The sandbox ABI's entry points take no arguments** → `SandboxHost` passes the rendered `args_mapping` in the service's KV under `args` and reads `output` back; the component needs `kv.get`/`kv.set` grants.

## E1 Redox timeline demo
- **A file tailer fails on an idle `trace:subscribe` read** → the scheme returns
  `EAGAIN`, unlike EOF on a regular file. → Read the scheme in bounded chunks,
  retain partial JSONL lines, and stop that poll on `WouldBlock`
  (`apps/timeline/src/feed.rs`).
- **Orbital cannot run from the drive-free RAM initfs alone** → its packaged
  binary needs `/lib/ld.so.1`, and even a static rebuild needs absolute
  `/usr/share/ui` and `/usr/share/fonts` paths; those paths have no mounted
  root filesystem in the RAM shell. → Use the `desktop-minimal` live image for
  the graphical E1 demonstration; keep the drive-free trace publisher as a
  separate boot check (`scripts/e1-live.sh`).
- **A QEMU screenshot can be all black just after the producer logs success**
  → Orbital has not finished drawing. → `scripts/e1-qemu.sh` waits for orange
  timeline pixels before accepting the screendump.
- **An indexed PNG looked black in the evidence viewer despite orange pixels**
  → ImageMagick stored the screendump as a palette image that the viewer did
  not display consistently. → Save the final screenshot as RGB PNG with Pillow
  and inspect that file (`scripts/e1-qemu.sh`).
- **A live image appeared to contain the upstream kernel after a successful
  FUSE copy** → the RedoxFS mount process had not exited before QEMU read the
  image. → Unmount, wait for `redoxfs`, remount, and byte-compare the kernel,
  initfs, and app binaries before booting (`scripts/e1-live.sh`).
- **QEMU booted the image's old EFI loader despite PXE/TFTP being configured**
  → firmware chose the disk first. → Give the PXE NIC `bootindex=1` and the
  disk `bootindex=2` to load the E1 live bootloader; its GOP mode auto-selects
  when no keyboard is available (`scripts/e1-qemu.sh`).

## Kernel boot console from the DT (K14, Cursor; merged by the orchestrator)
- **A DT `simple-framebuffer` sitting in RAM the bootloader calls "free" is not safe to write**
  → on UEFI the bootloader's own loader data lives inside Free memory-map areas, so `free`
  alone cannot vouch for the buffer. → `check_placement()` accepts it only when a
  `reserved-memory` node contains it, and separately rejects overlap with the kernel's
  boot-data ranges (kernel/stack/env/hwdesc/areas/bootstrap) or anything past the
  bootloader's 8 GiB identity map (`src/dtb/simplefb.rs`).
- **The enchilada splash node is not under `/chosen`** → the vendor tree references
  `cont_splash_mem` from a top-level framebuffer node. → Prefer enabled `/chosen` children,
  then fall back to any enabled `simple-framebuffer` node; resolve the base through the
  `memory-region` phandle before `reg` (`src/dtb/simplefb.rs::find`).
- **Writing the framebuffer before the kernel has page tables uses the bootloader's
  PHYS_OFFSET identity map, whose cache attributes are unknown** → per-write cache
  cleaning is needed at first but wrong once the kernel remaps the region write-combining.
  → Register exactly the visible buffer as `Reserved` so `startup::memory::init` maps it
  write-combining, then call `graphical_debug::write_combining_mapped()` to stop cleaning
  (`src/devices/graphical_debug/mod.rs`).
- **The DT stride property is bytes, not pixels** → a padded stride (1080 px → 4320 B)
  silently misrenders if used as pixels. → Validate `stride % 4 == 0` and `stride ≥ width*4`,
  then divide by 4 (host test `padded_stride_is_converted_to_pixels`).
- **Merge check (benign this time):** `op6/fbcon` branched from `op6/integration-3` while
  `op6/e1-demo` had moved on; `git am` of `patches/kernel-fbcon/` onto `op6/e1-demo` applied
  cleanly and the full `scripts/integrate.sh --qemu` gate (82 host tests + QEMU assertions,
  new fbcon greps included) passed on the merged tree before refreshing the payload hash.

## Remote link over satellite uplink (2026-10-24 ~9 PM PT)
- **Uploads to the phone stall completely over the mesh (scp/ssh pipes hang), small commands work** → the phone's
  `vpn0` (WireGuard) MTU is 1500, too large for the tunnel + satellite uplink path; big packets vanish. Fix:
  `sudo ip link set vpn0 mtu 1280` (not persistent; NordVPN may reset it). After the fix: RTT ~58 ms,
  down ~0.5 Mbit/s, up ~0.1 Mbit/s — fine for SSH/logs/snapshots, slow for boot images.
- **Phone on the MacBook's USB (remote):** run phone work ON the Mac (`oneplus-phone` skill `phone-via-mac.sh`) —
  Mac↔phone USB is 1.6 ms / ~100+ Mbit/s while 3090↔Mac over satellite uplink is ~58 ms / ~0.1–0.5 Mbit/s. macOS self-assigns
  169.254.x to the phone link and the phone's NordVPN firewall drops it; set <host-usb-ip>/16 on the Mac. Don't call
  `python3` on a Mac without developer tools (install dialog pops up on the operator's screen).

## caps-check power ledger (2026-10-25, branch `caps/release-lifecycle`)
- **A "power off" that also rewinds the on-time meter = budget refills to 0 (fixed by `WakeLedger::throttle`)**
  → `WakeLedger::release` was the only power-off in `tools/caps-check/src/power.rs`, and its doc-comment assigned it
  to the safety floors too ("called when a grant dies … **and** by the safety floors, which may throttle below a
  routine's budget (R8)"). But `release` *also* wipes the domain's durable `Meter` list, so it cannot do double
  duty: a thermal-ceiling throttle that went through it left the **grant live** and its on-time meter **at zero**
  again. Repro before/after the fix (bill after power comes back, 4 s already spent of a 5 s budget):
  `release` → `[(1, 0)]` and `spend(2 s)` accepted → repeated dips refill the window → **"on forever"**;
  `throttle` → `[(1, 4000000000)]` and `spend(2 s)` refused → ceiling intact. R8's own floor ("a grant may not
  outlive its deadline with the block still powered") is what the conflated call quietly allowed.
- **Two lifecycle verbs, keep them apart:** `WakeLedger::throttle(domain)` = temporary power-off — closes *this*
  wake window, meter survives, the still-live grant may re-arm and gets only what is **left**. `WakeLedger::release(domain)`
  = grant *retirement* — meter cleared, because the next grant on that domain is a new authority with its own budget.
  Same shape in `grant.rs`: `GrantTable::release(grant)` retires a *grant*; do not call it for a rail (R7).
- **A pause that refills a budget defeats it the same way a retry does:** C-6/F-1 closed "re-taking a refused hold
  resets the meter"; this was the same ceiling reached by a different door, so it survived the first pass. Any new
  "power off / pause / suspend" helper must leave `on_time` and `closed` intact — `sleep` already refuses to hand
  a budget back, and R12's `spent_by_grant` bill would start claiming the most expensive grant spent nothing.
- **Run the suite in `--release` too, not just debug:** `checked_add`/`saturating_add` in `power.rs` is what keeps
  an unaddable overspend from reading as *affordable*; verified in both profiles (34 model tests at the throttle
  fix, 38 after the bill fix), no wrap today.
- **A bill read from the live `Hold` is a power gauge, not a bill (fixed same day, `560621c` follow-up):**
  `spent_by_grant` scanned `domains[].hold`, so any power-off that *closes* a window — throttle, sleep, budget
  refusal — dropped the grant off the bill even though its `Meter` survived and the grant was still live (4 s of a
  5 s budget spent → thermal dip → bill says "0 s"). The fix moves the read to `self.meters`; keep it there.
  `wake_source()`/`is_on()` are the *right now* questions and may read holds; `spent_by_grant()` is the *what did
  it cost* question and must not.
- **Never sum live hold + durable meter** — `charge` keeps them equal (both `hold` and `spend` mirror the hold
  into the meter, and `charge` takes `.max`), so adding both bills the same on-time twice. One meter per
  `(domain, grant)` = one authoritative number, which is what makes "the ceiling and the bill agree" true by
  construction instead of by test. Check any rework against: `[(7, 5s)]` for one grant on `adsp`+`codec` (2 s + 3 s
  merged into **one** line), before and after throttling both rails, and again after re-arming them.
- Doc drift found doing this: §6 called the query `spent_by_service()` while the code implemented `spent_by_grant()`
  — per *grant* is the working spelling (a grant can hold several rails, which is exactly why they merge).

## Emulator touch end to end (EMU-TOUCH, 2026-10-26)
- **On the phone DTB no TLMM/PDC irqchip ever initialised, and the timer/serial IRQs failed to translate** → the root `interrupt-parent` is the GIC and the GIC lists its own maintenance PPI, so it looked like a child of itself and was never a root. → A self-parented controller is a root, as in Linux `of_irq_init` (`patches/kernel-series/0013`). QEMU `virt` hides this because its GIC node has no `interrupts`.
- **The GIC saw INTID 647 high, but the CPU never read `ICC_IAR1`** → Redox's GICv3 reused the GICv2 distributor init and left SPIs in Group 0, which is FIQ with the system-register interface; only Group 1 is enabled. → Set `GICD_IGROUPR` for all SPIs (`patches/kernel-series/0017`). Check with `-trace gicv3_icc_iar1_read`, not only `gicv3_dist_set_irq`.
- **Initialising the full phone irqchip tree also enabled LMH/MDSS/SPMI/codec summary SPIs with no driver** → the `Null` placeholder never EOIs, and an active SPI at the shared 0xa0 priority blocks all other device IRQs. → Keep those lines masked (`patches/kernel-series/0018`).
- **`rmi4d` panicked like the old `spmid`** (`memory root fd not initialized`) → `common::init` must run before `PhysBorrowed::map`, and it panics if called twice, so call it once in `main`, not in a shared mapping helper.
- **A QMP `btn` up alone does not lift the finger in `rmi4-s3706`** → the model only rewrites a slot on a sync that carried ABS. → Send the release with the same `abs` X/Y (`scripts/emu-op6/touch-e2e.sh`).
- **`*.log` is gitignored repo-wide** → commit serial evidence as `*.txt` (`docs/workbench/evidence/emu-touch/`).
- **A QMP unix socket under `out/emu-op6/<run-id>/` can exceed the 108-byte `AF_UNIX` path limit** → put it in `mktemp -d /tmp/...`.

