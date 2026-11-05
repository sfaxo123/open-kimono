# simplefb — Boot framebuffer handoff (OnePlus 6 / SDM845)

Block: boot splash framebuffer handoff from firmware (ABL/edk2-sdm845) to Redox
vesad/fbcond/Orbital. This is a *memory-only* block: the driver consumes a
linear framebuffer that firmware already scans out; it owns no MMIO registers.

Source revisions (all `path:line` cites below are at these commits):
`linux@5b0764a`, `u-boot@8649a54d`, `openbsd-src@20cb344`,
`edk2-sdm845@d7fa8bd`, `redox/bootloader@cc922ec`, `redox/kernel@8186c379`,
`redox/base@d2ffc4ca`, `redox/redox@e079d707b`.
Note: `linux` and `u-boot` checkouts are sparse; cited driver files were read
via `git show HEAD:<path>` at the recorded commit.

## 1. Block

- DT `compatible`: `"simple-framebuffer"` — node `framebuffer` under `/chosen`
  in `linux:arch/arm64/boot/dts/qcom/sdm845-oneplus-common.dtsi:75-82`
  (`chosen` opens at :68). Linux instantiates it early via
  `of_get_compatible_child(of_chosen, "simple-framebuffer")` +
  `of_platform_device_create()` — `linux:drivers/of/platform.c:581-594` @5b0764a.
- Node properties (enchilada = OnePlus 6):
  - `memory-region = <&cont_splash_mem>` — `…-common.dtsi:77`
  - `format = "a8r8g8b8"` — `…-common.dtsi:79`
  - `stride = <(1080 * 4)>` = 4320 **bytes** — `…-common.dtsi:80`
  - `width = <1080>` — `…-common.dtsi:81`
  - `height = <2280>` — `linux:arch/arm64/boot/dts/qcom/sdm845-oneplus-enchilada.dts:43-45`
  - No `reg` property: the address comes from the reserved-memory phandle;
    Linux resolves it with `of_reserved_mem_region_to_resource(np, 0, …)` —
    `linux:drivers/video/fbdev/simplefb.c:179`, binding confirms
    memory-region overrides reg — `linux:Documentation/devicetree/bindings/display/simple-framebuffer.yaml:71-74`.
- Reserved memory: `cont_splash_mem: splash@9d400000 { reg = <0 0x9d400000 0 0x02400000>; no-map; }`
  — `…-common.dtsi:126-129`. 36 MiB reserved; the active image is only
  1080×2280×4 ≈ 9.85 MiB (rest is firmware slack).
- `a8r8g8b8` = 32 bpp, R@16/8, G@8/8, B@0/8, A@24/8 —
  `linux:include/linux/platform_data/simplefb.h:16,24` (SIMPLEFB_FORMATS).
- Dependencies (must NOT be re-initialized while the splash scans out):
  MDSS at `ae00000` — `linux:arch/arm64/boot/dts/qcom/sdm845.dtsi:4607-4610`,
  MDP `ae01000` — `:4635`, DSI0 `ae94000` — `:4787`; panel enabled via
  `&mdss`/`&mdss_dsi0`/`display_panel` — `…-common.dtsi:558,562,570`.
- Interrupts / clocks / resets / supplies / pinctrl: **none on the framebuffer
  node itself** (only the panel/regulators have them, lines above). The
  simplefb driver's clock-pinning helper (`simplefb_clocks_get`,
  `linux:drivers/video/fbdev/simplefb.c:223-235`) is a no-op here because the
  node has no `clocks` property — see gaps, §8.

## 2. What firmware leaves configured

Downstream chain (XBL → ABL): ABL brings up DSI0 + panel and blits a splash
into DRAM at 0x9d400000; the downstream kernel cmdline names it
(`cont_splash_mem=…@0x9d400000` — not in local repos; upstream DTS hard-codes
the same address, `…-common.dtsi:126-129`, and edk2 agrees, below).

edk2-sdm845 (the local UEFI for this phone) makes the handoff explicit:
- Splash address PCD: `PcdMipiFrameBufferAddress|0x9d400000` —
  `edk2-sdm845:sdm845Pkg/sdm845Pkg.dsc:142` (decl `sdm845Pkg.dec:54-56`;
  per-device override example, commented, for OP6T: `Devices/beryllium.dsc:18`
  shows the address can vary per device — 0x9D469780).
- Enchilada mode: `PcdMipiFrameBufferWidth|1080`, `…Height|2280` —
  `edk2-sdm845:sdm845Pkg/Devices/enchilada.dsc:24-25`.
- `SimpleFbDxe` reads the PCDs (`SimpleFbDxe.c:167-169`), publishes GOP mode
  `PixelBlueGreenRedReserved8BitPerColor` (= BGRA in memory = a8r8g8b8),
  `FrameBufferBase/Size` (`:208-221`), write-back-invalidates the data cache
  over the FB (`:236-241`), and installs `gEfiGraphicsOutputProtocolGuid`
  (`:249-251`) — `edk2-sdm845:sdm845Pkg/Drivers/SimpleFbDxe/SimpleFbDxe.c`.
- The region is kept out of UEFI RAM: `{0x9D400000, 0x02400000,
  EFI_RESOURCE_MEMORY_RESERVED}` — `edk2-sdm845:sdm845Pkg/Include/Configuration/DeviceMemoryMap.h:95-96`
  (same base+size as the Linux `cont_splash_mem` node).
- enchilada builds embed the Linux DTB (`sdm845-oneplus-enchilada.dtb`) in the
  FV — `edk2-sdm845:sdm845Pkg/Devices/enchilada.fdf:554` — and the Redox
  bootloader picks it up from the UEFI `DEVICE_TREE_GUID` config table
  (`redox/bootloader:src/os/uefi/dtb.rs:7`, `src/os/uefi/mod.rs:229-236`).

U-Boot (reference only; no OP6 board, only db845c/axolotl/starqltechn —
`u-boot:arch/arm/dts/sdm845-u-boot.dtsi`): U-Boot *writes* the handoff into the
kernel DTB with `fdt_setup_simplefb_node()` (sets reg/width/height/stride/format)
— `u-boot:boot/fdt_support.c:2086-2125` — and its own consumer driver resolves
`memory-region` the same way — `u-boot:drivers/video/simplefb.c:26-34`.

What the Redox driver must (re)do: nothing to the display hardware. It must
(a) discover base/width/height/stride/format, (b) keep the region reserved and
mapped write-combining, (c) never reset MDSS/DSI/clocks, or the panel blanks.

## 3. "Register map"

No MMIO registers belong to this block. The authoritative field map is the
handoff record, in three dialects:

| Field | Linux DT (`chosen/framebuffer`) | UEFI GOP mode data | Redox boot env |
|---|---|---|---|
| base | `memory-region` → `cont_splash_mem.reg` = 0x9d400000 (`…-common.dtsi:77,127`) | `Mode->FrameBufferBase` (`edk2:SimpleFbDxe.c:219`) | `FRAMEBUFFER_ADDR` (`redox/bootloader:src/main.rs:643`) |
| width | `width = <1080>` (`:81`) | `HorizontalResolution` (`:208`) | `FRAMEBUFFER_WIDTH` (`main.rs:644`) |
| height | `height = <2280>` (enchilada.dts:44) | `VerticalResolution` (`:209`) | `FRAMEBUFFER_HEIGHT` (`main.rs:645`) |
| stride | 4320 **bytes** (`:80`) | `PixelsPerScanLine` = 1080 **pixels** (`:213`) | `FRAMEBUFFER_STRIDE` = 1080 **pixels** (`redox/bootloader:src/os/uefi/video_mode.rs:42`, `src/main.rs:646`) |
| format | `"a8r8g8b8"` (`:79`) | `PixelBlueGreenRedReserved8BitPerColor` (`:214-215`) | implied BGRA32 (`redox/kernel:src/devices/graphical_debug/mod.rs:59`) |

⚠ Stride units differ: Linux DT = bytes (4320); Redox env = pixels (1080,
from `PixelsPerScanLine`), and Redox consumers multiply by 4 themselves
(`redox/base:drivers/graphics/vesad/src/scheme.rs:171,179`). A DTB-driven
Redox path must convert 4320 bytes → 1080 pixels.

Pixel layout (little-endian word): B[7:0] G[15:8] R[23:16] A[31:24] —
`linux:include/linux/platform_data/simplefb.h:24`; identical table in
`openbsd-src:sys/dev/fdt/simplefb.c:55` (`{ "a8r8g8b8", 32, 16, 8, 8, 8, 0, 8 }`).

## 4. Sequences

### 4.1 Boot handoff (reference, what must survive)
1. ABL/XBL: DSI0 + panel up, splash blitted to 0x9d400000 (§2).
2. edk2 `SimpleFbDxe.entry`: read PCDs → fill GOP mode → cache WB-invalidate →
   install GOP (`edk2:SimpleFbDxe.c:167-251`).
3. Redox bootloader enumerates GOP outputs/modes
   (`redox/bootloader:src/os/uefi/display.rs:5-14`, `src/os/uefi/video_mode.rs:30-55`),
   calls `SetMode` and re-reads `FrameBufferBase` **before** snapshotting the
   UEFI memory map (`src/main.rs:566-573`).
4. Bootloader maps the FB into the kernel page tables using UEFI memory-map
   attributes (`src/arch/aarch64.rs:172-235`, size = `stride*height*4`,
   `src/main.rs:635-641`) and writes `FRAMEBUFFER_ADDR/VIRT/WIDTH/HEIGHT/STRIDE`
   into the kernel env (`src/main.rs:643-647`).
5. Kernel `graphical_debug::init(env)` parses those vars, stores
   `(phys, virt, stride*height*4)` and creates the boot `DebugDisplay`
   (`redox/kernel:src/devices/graphical_debug/mod.rs:12-62`).
6. Kernel re-creates page tables keeping the FB mapped **write-combining** at
   the same virt (`redox/kernel:src/startup/memory.rs:526-545`), and registers
   DTB `/reserved-memory` regions (incl. `cont_splash_mem`) as
   `BootloaderMemoryKind::Reserved` so the allocator never hands them out
   (`src/dtb/mod.rs:369-373`; DTB passed via `DTB_ADDR` env,
   `src/arch/aarch64/start.rs:118-124`).
7. Userspace `vesad` reads the same env vars from `/scheme/sys/env`
   (`redox/base:drivers/graphics/vesad/src/main.rs:16-49`), maps the FB RW
   **write-combining** through the kernel memory scheme
   (`src/scheme.rs:170-183` → `drivers/common/src/lib.rs:163-208`,
   `/scheme/memory/physical@wc`), and serves scheme `display.vesa`
   (`main.rs:82`).
8. `fbcond` attaches as an inputd VT consumer (`/scheme/input/consumer`,
   then `open_display`) and draws the text console
   (`redox/base:drivers/graphics/fbcond/src/main.rs:47-62`;
   `drivers/inputd/src/lib.rs:37-42,57-66`; service unit
   `redox/base:init.d/fbcond@.service`). Orbital (GUI shell, external crate —
   `redox/redox:recipes/gui/orbital/recipe.toml`) renders on the same display
   scheme.

### 4.2 Linux oracle (what to compare against)
1. `of_platform_default_populate_init` finds `chosen/simple-framebuffer`,
   disables sysfb, creates the platform device
   (`linux:drivers/of/platform.c:581-596`).
2. `simplefb_probe` → `simplefb_parse_dt`: width/height/stride/format, then
   memory-region → resource (`simplefb.c:136-187`); falls back to
   `platform_get_resource` only if that fails (`:552-556`).
3. `request_mem_region` (`:562`), `ioremap_wc` (`:602`), `register_framebuffer`
   (`:640`). Match table `simple-framebuffer` (`:674-678`).
   (The `screen_info`/sysfb path — `linux:drivers/firmware/sysfb_simplefb.c:27-62`
   — is x86/EFI-stub only; on this DT phone it is disabled at step 1.)

### 4.3 Error recovery / shutdown
- No device to reset. On driver unload Linux releases clocks (none here) and
  `unregister_framebuffer` (`simplefb.c:671`). Redox: vesad exit leaves the FB
  contents; panel stays on (firmware DSI config untouched).
- Failure mode to guard: any driver that resets GCCS/GDSC clock controllers or
  the SMMU stream mappings for `ae00000`-`ae9fff` blanks the panel and the
  splash dies — keep those blocks unclaimed until a real MDP driver exists.

## 5. Interrupts / DMA / power

- **Interrupts:** none. Scanout is driven by MDSS hardware configured by
  firmware; the handoff block is pure memory. No `/scheme/irq` usage.
- **DMA:** none from our side (MDSS fetches the splash itself). CPU writes only.
- **Caches:** the FB must be mapped WC (or UC) — Linux `ioremap_wc`
  (`simplefb.c:602`), Redox `physical@wc` (`redox/base:drivers/common/src/lib.rs:176-181`,
  kernel side `redox/kernel:src/scheme/memory.rs:234`), bootloader page tables
  inherit UEFI attributes (`redox/bootloader:src/arch/aarch64.rs:216-222`).
  If anything ever maps it cacheable, flush before scanout like edk2's
  `WriteBackInvalidateDataCacheRange` (`edk2:SimpleFbDxe.c:239-241`).
- **Power:** panel + DSI clocks are firmware-owned; no regulator/clock calls
  from this driver. Region must stay `Reserved` in the kernel memory map
  (`redox/kernel:src/dtb/mod.rs:369-373`) and out of UEFI RAM pools
  (`edk2:DeviceMemoryMap.h:95-96`).
- **SMMU:** not involved for CPU access; do not attach the MDSS SMMU context
  banks until MDSS itself is driven.

## 6. Redox interface

- **Kernel piece:** keep `graphical_debug` boot console (env-driven,
  `src/devices/graphical_debug/mod.rs`) and the WC re-map in
  `startup/memory.rs`. The kernel already exposes the DTB to userspace as
  `/scheme/dtb` (`redox/kernel:src/scheme/dtb.rs`) — that is the missing
  discovery channel: today **no Redox component reads
  `chosen/framebuffer` from the DTB**; FRAMEBUFFER_* env comes only from UEFI
  GOP (`redox/bootloader:src/main.rs:631-651`).
- **User-space daemon:** `vesad` (base `drivers/graphics/vesad`) stays the
  display provider, scheme name `display.vesa` (`main.rs:82`). For the
  DTB-handoff variant, add a discovery step in vesad (or a tiny
  `simplefb-discover` starter from `exampled`): open `/scheme/dtb`, walk
  `/chosen/framebuffer`, resolve `memory-region` → `/reserved-memory/splash@9d400000`
  reg, convert stride bytes→pixels, then proceed exactly as today.
- **Mappings:** `/scheme/memory/physical@wc` + `mmap(offset=phys)` —
  `redox/base:drivers/common/src/lib.rs:163-208`. Never map the whole 36 MiB;
  map `stride*height*4` (≈9.85 MiB).
- **Console/GUI:** `fbcond` per VT (`init.d/fbcond@.service`) and Orbital on
  top of `display.vesa` — unchanged.
- **IRQ:** none needed.

## 7. Verification

1. **Host tests:** unit-test the DTB walker against the real blob
   `edk2-sdm845:sdm845Pkg/FdtBlob/sdm845-oneplus-enchilada.dtb` (assert
   width=1080, height=2280, stride=4320, format=a8r8g8b8, base=0x9d400000 via
   the memory-region phandle). Model: a byte-array framebuffer + blit model,
   compare rendered glyphs vs OpenBSD `rasops`-style output.
2. **Linux oracle on the pmOS phone:**
   `ls /proc/device-tree/chosen/framebuffer/`,
   `hexdump -C /proc/device-tree/chosen/framebuffer/{width,height,stride,format}`,
   `cat /proc/device-tree/reserved-memory/splash@9d400000/reg`,
   `fbset -fb /dev/fb0` (expect 1080x2280, 32bpp, length 4320), and
   `devmem2 0x9d400000 w` (non-destructive read) to confirm the live splash.
   Capture a few frames to a file for the recorded-trace test.
3. **edk2 cross-check:** boot edk2-sdm845 shell, `dh -p GraphicsOutput` then
   `Mode.FrameBufferBase` — must read 0x9d400000 (PCD at `sdm845Pkg.dsc:142`).
4. **On-phone acceptance:** `fastboot boot` the Redox image; acceptance = boot
   log visible via `graphical_debug` on-panel, vesad reports
   `1080x2280 stride 1080 at 0x9D400000`, fbcond shows a prompt, no panel
   blank within 10 min, and the splash's last frame is not corrupted before
   vesad's first paint.

## 8. License path

**MIT clean-room.** Primary references and their licenses:
- `openbsd-src:sys/dev/fdt/simplefb.c` (ISC, header :1-14) — format table,
  property parsing, match rules: **safe to port closely**. Commit 20cb344.
- `edk2-sdm845:sdm845Pkg/Drivers/SimpleFbDxe/SimpleFbDxe.c` — edk2 default
  BSD-2-Clause-Patent: usable as a direct model for the GOP-side handoff.
- `u-boot:boot/fdt_support.c:2086` and `drivers/video/simplefb.c` —
  **GPL-2.0+**: consulted for the DTB-fixup and memory-region resolution
  *behavior only*; no code copied.
- `linux:drivers/video/fbdev/simplefb.c`, `simplefb.h`, DTS files — GPL-2.0:
  consulted as the behavioral oracle; DTS values are hardware facts restated
  here, no code copied.
- Redox crates (bootloader/kernel/base) are MIT — extending them in-tree is
  license-neutral.
No GPL-only source had to be read for implementation logic (OpenBSD covers
the parsing surface), so no separate GPL crate is needed.

**Gaps / conflicts (resolve before implementing):**
1. **Discovery gap:** Redox bootloader finds the FB only via UEFI GOP. With
   edk2-sdm845 that works (SimpleFbDxe); with stock ABL it is unverified
   whether GOP is exposed to third-party bootloaders — the DTB path
   (`/scheme/dtb` → `chosen/framebuffer`) is the robust fallback and does not
   exist yet in any Redox component.
2. **Stride unit conflict:** DT stride = 4320 bytes vs Redox env stride =
   pixels (1080). Any DTB-driven path must divide by bpp/8; getting this wrong
   skews the image 4×.
3. **No `clocks` on the framebuffer node** — Linux's clock-pinning
   (`simplefb.c:223-235`) is a no-op on this phone; nothing in any OS pins the
   DSI/MDSS clocks for the splash. The handoff only survives because firmware
   clock state is never reset — fragile if a clock-controller driver loads
   early.
4. **Downstream ABL cmdline not in local repos:** the `cont_splash_mem=`
   cmdline value is inferred from the upstream DTS hard-code
   (`…-common.dtsi:126-129`) and edk2 PCD (`sdm845Pkg.dsc:142`); both agree on
   0x9d400000, but `beryllium.dsc:18` shows per-device variation exists —
   re-verify on-device, don't hard-code blindly.
5. **Region size:** 36 MiB reserved vs ~9.85 MiB used; bootloader maps
   `stride*height*4` only (`main.rs:639`) — fine, but the DTB path must not
   `mmap` the whole reserved node.
6. Orbital source is not in `~/Developer/forks` (external gitlab crate,
   `recipes/gui/orbital/recipe.toml`) — its display attach path is inferred
   from the `display.vesa` scheme contract, not read.
