# K14 — kernel early framebuffer console from the device tree

Verified 2026-10-25 01:50 UTC (Perplexity local-5090 orchestrator review; the Cursor worker's
`op6/fbcon` branch stopped committing at 00:40 UTC and its dispatch job is gone). Reviewed the diff,
not the report. No phone access, no push.

## Source and patch commits

- Kernel branch `op6/fbcon` in `~/Developer/forks/redox/kernel-k14`, based exactly on
  `op6/integration-3` (`d73469ef`): `fa1cb26f` (dtb: discover a simple-framebuffer),
  `afab4022` (graphical_debug: scrolling, scaled, coloured boot console),
  `36790235` (aarch64: boot console on the DT simple-framebuffer).
  15 files, +1189/−91.
- Exported to the project as `patches/kernel-fbcon/` (3 patches) and wired into
  `scripts/integrate.sh` after `integration-3/kernel`. Applied onto `op6/e1-demo` with `git am`
  with **no conflicts** (commits `8d8c9a24`, `0fc288b1`, `67d53aaa`).

## What it does (reviewed)

- `src/dtb/simplefb.rs`: finds an enabled `simple-framebuffer` node — `/chosen` children first,
  then any node (the enchilada tree references `cont_splash_mem` from outside `/chosen`); base via
  `memory-region` phandle before `reg`; byte stride → pixels; only `a8r8g8b8`/`x8r8g8b8`; raw
  property reads so a malformed node is rejected, never panics this early in boot.
  `check_placement()` refuses a buffer outside the bootloader's 8 GiB identity map, over the
  kernel's own boot data (kernel/stack/env/hwdesc/areas/bootstrap ranges), or in bootloader-free
  RAM without a `reserved-memory` node claiming it — the never-touch-no-map rule.
- `src/devices/graphical_debug/debug.rs`: cell-buffer console with a built-in 8×16 unifont,
  2× scaling on phone-sized panels, scrolling, line wrap, orange prefixes/warnings + white
  messages, stride padding never written.
- `init_dtb()` in `graphical_debug`: runs right after `dtb::serial::init_early`, writes through the
  PHYS_OFFSET identity map, registers exactly the visible buffer as `Reserved` so
  `startup::memory::init` maps it write-combining and keeps it out of the allocator/physmap, and
  stops per-write cache cleaning once that map is live. The DT record wins over a GOP framebuffer
  (mismatch logged), matching vesad's precedence.

## Verification (run on current `op6/e1-demo` + all patch series)

- Host tests in `kernel-k14`: `cargo test --target aarch64-unknown-linux-gnu` (aarch64-unknown-linux-gnu
  linker + `qemu-aarch64-static` runner, `--skip context::switch::tests::test_work_stealing`):
  **82 passed, 0 failed, 1 filtered** (was 59 at K8c; +23 K14 tests). Includes the golden glyph
  bitmap, scroll, wrap, colour, stride-padding, phone-panel scaling tests and the simplefb
  discovery/placement negative cases (malformed nodes, framebuffer in free RAM without carve-out,
  overlap with boot data, region too small, unaligned base). Log: ignored
  `out/integration/k14-host-tests.log`.
- `make check ARCH=aarch64`: exit 0 (`out/integration/k14-make-check.log`).
- `bash scripts/integrate.sh --qemu`: exit 0 with the new fbcon assertions added to the gate.
  Kernel log line 38: `device tree simple-framebuffer 800x600 stride 800 at BC430000` — the boot
  console is live **before** `Redox OS starting...`, and `Registering Reserved memory BC430000`
  shows the carve-out path. No `ignoring device tree simple-framebuffer` line. All previous
  assertions still pass: `ram:~$`, `timeline-demo-publish: sent 28 events to trace:publish`,
  `spmid: no SPMI arbiter in device tree; skipping`, `vesad: simplefb 800x600 … from device tree`.
  Tracked log: [`K14-QEMU.txt`](K14-QEMU.txt) (CRs stripped, 10,333 bytes, SHA-256
  `417b6623512451426c584f8d408ae68a48ea0aa975c890d0b34c573ad637dc9c`).
- Refreshed payload: `~/artifacts/handoff/oneplus-redox-20261024/phone-boot/redox-ramboot.efi`
  SHA-256 `47b194b742de02ec44739d2f34dfafa166a98a965e15490d4a4f2f52c149d96d` (was `3b025a82…` at
  K8c), `SHA256SUMS` verified.

## Gaps

- No `screendump` of kernel text on the panel: the drive-free QEMU gate runs `-display none` and
  the E1 live-image script screenshots only after Orbital draws. The on-screen glyph rendering is
  proven by host golden-cell tests; a visual QEMU capture is a phone-night nice-to-have.
- The phone's real `cont_splash_mem` 0x9d400000 1080×2280 path is exercised only through the
  op6-shaped host test fixture; on hardware the placement check must accept it against ABL's
  memory map — first boot attempt will show it (the fbcon itself is the diagnostic).
