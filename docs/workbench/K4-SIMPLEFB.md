# K4 — simplefb: DT `simple-framebuffer` → vesad / fbcond / Orbital

**Result (2026-10-24 PT):** vesad now reads `/scheme/kernel.dtb`, takes the first enabled
`simple-framebuffer` child of `/chosen` (base via `memory-region`, else `reg`; DT stride bytes →
pixels), and serves `display.vesa` from it. fbcond and Orbital draw on it unchanged. Verified on QEMU
`virt,acpi=off,gic-version=3 -cpu max` with ramfb and with the OnePlus 6 handoff record (1080×2280
a8r8g8b8, stride 4320 B, `memory-region` → 36 MiB carve-out at 0x9d400000) and no display device.

## Commits (base-k4, branch `op6/simplefb`; kernel-k4 unchanged at `8186c379`)
- `4def7e93` simplefb: discover device tree simple-framebuffer nodes
- `4f3f70f9` simplefb: find /chosen when the DTB contains FDT_NOP tags
- `3b3ed66b` vesad: use a device tree simple-framebuffer as the boot framebuffer

Precedence: DT record wins over the bootloader GOP `FRAMEBUFFER_*` env (Linux semantics: chosen
simplefb disables sysfb); a mismatch is logged; an invalid node logs the reason and falls back to env.

## Host tests — `k4-simplefb-host-tests.log`
`cargo test -p simplefb` (17 tests, in-process FDT builder, no binary fixtures). Optional
`SIMPLEFB_ENCHILADA_DTB=<dtb>` test passes on `sdm845-oneplus-enchilada-linux-5b0764a.dtb` (compiled here
from Linux `5b0764a` DTS; GPL-derived, kept out of the MIT tree), `k4-dt-op6.dtb`, and the in-guest
UEFI-rewritten DTB `k4-guest-op6.dtb` (dumped from RAM).

## QEMU runs — `k4-qemu-summary.txt`, `<name>.serial.log`, `<name>.png`
Image `k4-live.iso` = W0B live minimal + kernel-k4 + base-k4 initfs/bins; `k4-desktop.iso` = upstream
`desktop-minimal` (prebuilt aarch64 pkgs, installer run in Podman) + same swap. Hashes: `k4-images.sha256`.
DTBs: `k4-make-dtbs.sh` (QEMU dumpdtb + `/chosen/framebuffer` + `/reserved-memory`).

| Run | Display | Result |
|---|---|---|
| `k4-regress-ramfb` | ramfb, no DT node | env path; screen identical to prebuilt-vesad baseline |
| `k4-dt-ramfb` | ramfb + DT = GOP buffer | DT path; screen identical to env-only |
| `k4-dt-ramfb-half` | ramfb + DT = lower half only | "differs; using device tree"; fbcond login drawn in lower half |
| `k4-dt-ramfb-bad` | ramfb + DT `r5g6b5` | error logged, falls back to env |
| `k4-dt-op6` | none; DT 1080×2280 @0x9d400000 | DT path; `pmemsave` → `k4-dt-op6.fb.png` shows unskewed console |
| `k4-orbital-dt-ramfb[-half]` | as above, desktop image | Orbital login on the DT region |
| `k4-orbital-dt-op6` | none; phone record | Orbital portrait 1080×2280 in `k4-orbital-dt-op6.fb.png` |

Reproduce: `k4-build-base.sh`, `kernel-k4: make ARCH=aarch64 OBJCOPY=aarch64-linux-gnu-objcopy`,
`k4-make-dtbs.sh`, `k4-pack-image.sh k4-live.iso`, then e.g.
`K4_BOOTKEYS='\x1b[A|\r' ./k4-qemu.sh k4-dt-ramfb-half k4-live.iso 150 -device ramfb -dtb k4-dt-ramfb-half.dtb`.

## PROVENANCE entry (for `docs/PROVENANCE.md`)
| Path (redox base) | Upstream | SPDX | Kind |
|---|---|---|---|
| `drivers/graphics/simplefb/*` | Linux `Documentation/devicetree/bindings/display/simple-framebuffer.yaml` @5b0764a (binding semantics), OpenBSD `sys/dev/fdt/simplefb.c` @20cb344 (format names) | MIT (ours) | spec-only |
| `drivers/graphics/vesad/src/main.rs` (edit) | redox base @d2ffc4ca | MIT | adapt (in-tree) |

## Gaps
1. Spec §7.1 fixture is wrong: edk2-sdm845 `FdtBlob/sdm845-oneplus-enchilada.dtb` has **no**
   `/chosen/framebuffer` or splash carve-out; the upstream Linux DTB does. With edk2-sdm845 the DT path
   only works if a newer DTB is embedded; GOP env still works.
2. Kernel `graphical_debug` stays env-only: with DT but no GOP (stock-ABL path) the early kernel log is
   not on the panel until vesad/fbbootlogd start. Needs a kernel-side DT map + WC remap.
3. Only 32 bpp BGRA-layout formats (`a8r8g8b8`, `x8r8g8b8`) are accepted; others fall back.
4. ramfb under edk2 is limited to 640×480/800×600/1024×768, so 1080×2280 is proven by RAM dump, not scanout.
5. The UEFI bootloader's resolution menu has no timeout when a GOP exists (the harness answers it).
6. `fdt` 0.1.5 path lookup is NOP-unsafe; `bcm2835-sdhcid`'s `find_node("/soc")` can hit the same bug
   on firmware-edited DTBs (the kernel's own DT code was not audited for it).
