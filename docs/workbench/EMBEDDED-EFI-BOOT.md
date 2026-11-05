# Embedded EFI Android RAM boot image

`scripts/phone/build-embedded-efi.py` creates an Android v0 RAM-boot image with a U-Boot EFI app embedded directly in the linked U-Boot `.rodata`. The `op6efi` command passes `bootefi <start>:<size> <control-fdt>`; the size after the colon is required by this U-Boot revision. It checks the EFI bounds against the relocated image and monitor reservation before invoking `bootefi`.

The default EFI app is the 60-second diagnostic. The optional Redox app is pinned by SHA-256 `c3f6f3ac44c24a6330ad9f81578a838864fbc67cb6acd56adf04e8766adebf5a`. The Redox image was built and inspected offline only; it has not been executed in QEMU or on the phone.

## Inputs and build

The packager copies the checked-in `scripts/phone/embedded-efi/baseline.config` and uses the pinned phone DTB in that folder. The reconstructed DTS is checked through a compile/decompile canonical round trip each time the packager runs; the exact validated DTB bytes remain the packaged input. The DTS source was reconstructed from that binary, and its original upstream revision/build recipe is unknown. The round trip was verified directly on 2026-10-27 and is also a packager pre-build check. See `scripts/phone/embedded-efi/INPUTS.md` for hashes and provenance.

The build-only host tools and cross compiler are read from the existing phone-boot workspace at `$HOME/artifacts/handoff/oneplus-redox-20261024/phone-boot/`. The script creates its separate U-Boot worktree at `$HOME/artifacts/handoff/redox-perplexity/embedded-efi-boot/src` from commit `3e15c2819caa7e14e1e4b2ff758a41cbc58a7311` when absent. It refuses a changed commit or unexpected worktree edits. U-Boot private OnePlus 6 patches 001 and 002 are already present in that pinned commit.

Run either build with:

```sh
python3 scripts/phone/build-embedded-efi.py --payload diagnostic --out $HOME/artifacts/handoff/redox-perplexity/embedded-efi-boot/build-diagnostic-noabort
python3 scripts/phone/build-embedded-efi.py --payload redox --out $HOME/artifacts/handoff/redox-perplexity/embedded-efi-boot/build-redox-noabort
```

Always pass a fresh explicit output directory to preserve prior artifacts. The script stores outputs and a hash/inspection manifest there. Its manifest records the source commit, config and project input hashes, code/assembly/environment hashes, payload hash, linker and `.rodata` ranges, extracted Android payload comparison, and Android header inspection. It refuses to reuse a build directory whose prior manifest names a different payload.

The build changes the selected text environment and boot delay from the checked-in baseline `.config`, then runs `olddefconfig`. It uses `aarch64-linux-gnu-`, the existing host tools and BISON data, and packages gzip-compressed `u-boot-nodtb.bin` plus the actual OnePlus 6 DTB. Android options are page size 4096, base 0, kernel offset `0x8000`, ramdisk offset `0x01000000`, second offset `0x00f00000`, tags offset `0x100`, and header version 0. `SOURCE_DATE_EPOCH` is set from the pinned U-Boot commit. The packager rejects a phone or built U-Boot DTB `/config/bootdelay` override.

## Boot behavior

The generated environment sets `bootdelay=-2` and `bootcmd=op6efi`, and Kconfig sets `CONFIG_BOOTDELAY=-2`. U-Boot defines `-2` as autoboot without a delay and without checking for abort input. This avoids stray serial/button-keyboard input aborting autoboot during board initialization. The environment has no button action, menu, storage scan, USB/ACM command, reset, or write command. Manual recovery remains available through the platform recovery controls. In the V2 diagnostic run, the EFI app returned after its 60-second wait and U-Boot displayed the interactive `=>` prompt.

The EFI app is linked into U-Boot and does not need USB, UFS, or persistent storage to locate its bytes. Normal board initialization may still probe controllers: QEMU emitted nonfatal UFS/FMP probe errors before autoboot.

## No-abort rebuild receipts

Fresh diagnostic and Redox builds completed on 2026-10-27 in `build-diagnostic-noabort/` and `build-redox-noabort/`. Both manifests record `CONFIG_BOOTDELAY=-2`, environment `bootdelay=-2`, `bootcmd=op6efi`, OnePlus 6 DTB model, no phone or built-DTB `/config/bootdelay` override, and source epoch `1792986556`. Diagnostic Android image SHA-256 is `6d37e442b7280e2be10214e1bf5d26f073d2deec47ff6ccb21edf175e10de282`; Redox image SHA-256 is `51a98dc871b5fc68e6801b3ea35ad8e93db7465991c7fc0edabcc155528ffe7f`. The embedded payload and Android header/extraction checks passed for both. These are new outputs; older package artifacts were preserved.

The first no-abort package was run once in OnePlus 6 QEMU with a fresh disposable 4 GiB DTB copy, the two unsupported USB nodes disabled, and a newline piped to serial at process launch. The raw U-Boot binary plus DTB was loaded at `0x80008000` using QEMU's loader device. It reached the `op6efi` marker and EFI diagnostic entry at EL1; the app reported `DTB model: OnePlus 6`, `oneplus,enchilada`, `qcom,sdm845`, and GOP `1080x2280`. No `Hit any key to stop autoboot` prompt appeared. The 45-second host timeout stopped QEMU during its 60-second wait, before shell return. See `build-diagnostic-noabort/qemu-noabort-receipt.json` and its serial/stderr evidence files.

Two invocation mistakes were preserved separately: the initial duplicate-memory argument exited before QEMU startup (`qemu-noabort-duplicate-memory.stderr.log`); a `-kernel` invocation ran but produced no serial output (`qemu-noabort-queued-newline-run.stderr.log` and empty serial log). The successful loader-mode invocation above is the only positive diagnostic test. No physical device or protected VM image was used.

## V2 queued-input and shell-return check

The V2 diagnostic-only build is in `build-diagnostic-noabort-v2/`; it preserves all earlier outputs. Its Android image SHA-256 is `3ec4324de2ad4afe19f6845195787fa7d6c99d1009d88b5fed63f8a4264c4981`. Its embedded environment includes the read-only preboot marker `OP6_EFI_DIAG_NOABORT_V2` followed by `printenv bootcmd bootdelay stdin stdout stderr`. The preboot command contains no EFI invocation, and `bootcmd` remains the single `op6efi` call. Offline inspection found those exact environment strings in `u-boot-nodtb.bin`, `CONFIG_BOOTDELAY=-2`, and registered symbol `_u_boot_list_2_cmd_2_op6efi` in `u-boot`.

One 120-second-capped positive QEMU run used queued newline input and the direct loader method at `0x80008000`, with a fresh 4 GiB DTB copy and both unsupported USB nodes disabled. The marker printed all environment values, then `op6efi` and the EFI diagnostic ran at EL1 and identified the OnePlus 6 FDT and GOP. After the diagnostic's 60-second wait, U-Boot returned to the `=>` prompt. The host stopped that disposable QEMU after observing the prompt. The positive receipt and logs are `build-diagnostic-noabort-v2/qemu-v2-positive-receipt.json` and `qemu-v2-positive-queued-newline.*`.

One 15-second-capped negative control used the prior build with `CONFIG_BOOTDELAY=3` and `bootdelay=3`, a fresh 4 GiB/no-USB DTB copy, and queued newline. It showed `Hit any key to stop autoboot: 3` then `...: 0`, returned to `=>`, and did not enter EFI. This demonstrates the input abort on the old delay setting and the bypass in V2. Its receipt and logs are `qemu-old3s-control-receipt.json` and `qemu-old3s-control-queued-newline.*` in the V2 output directory. These are emulator results only; no phone or protected VM image was used.

## Diagnostic package and QEMU receipt

The immutable diagnostic package used for the QEMU run and staged by the root coordinator is `$HOME/artifacts/handoff/redox-perplexity/physical-embedded-20261027/op6-embedded-efi-diagnostic.img`, SHA-256 `a5fef3294894dcb50de07fd8d26b617f9a4e06a5dc0f2978154844a16da395ed`. Its build manifest is beside it. The original package was subsequently left untouched. A later diagnostic package refresh in this worktree has SHA-256 `4d1169099b715cbf1fffb5b88e759aab7eeefce7b3e2887914c49cf5aa1d5943`; its build inputs and linked source are the same, and its hash differs with the generated U-Boot timestamp.

The QEMU run booted the newly built raw U-Boot binary plus a fresh disposable DTB copy at `0x80008000`. The copy describes 4 GiB and disables the two unsupported USB nodes; the packaged phone DTB was not modified. It reached `op6efi`; the diagnostic reported EL1, `DTB model: OnePlus 6`, compatible strings `oneplus,enchilada` and `qcom,sdm845`, and GOP `1080x2280`. Runtime EFI range `0x17fe61000..0x17fe6c000` was within both the linked image range `0x17fda4000..0x17fef71c8` and monitor reservation `0x17fda4000..0x17feffb08`.

The one authorized run stopped at 44 seconds while the diagnostic displayed its 60-second wait. It did not reset the emulator; the host timeout ended QEMU. The shell return was not observed. The machine-readable test receipt is `qemu-diagnostic-receipt.json`; serial output, U-Boot+test DTB, QEMU diagnostics, and a screen capture are retained in `$HOME/artifacts/handoff/redox-perplexity/embedded-efi-boot/build-diagnostic/`. The serial log contains the readable diagnostic results. No protected VM image was used.

## Redox package receipt

The build-only Redox image is `$HOME/artifacts/handoff/redox-perplexity/embedded-efi-boot/build-redox/op6-embedded-efi-redox.img`, SHA-256 `aca872519f72874c7874e58813e593f6569eb52c3fde7124458d66fa5d52714d`. Its embedded EFI payload matches the pinned 8,767,488-byte source. The linked EFI range is `0xbd000..0x919800`, within `.rodata` and the linked image ending at `0x9a41c8`. Android header inspection and extracted gzip-U-Boot-plus-DTB equality passed. This does not prove the larger EFI app executes.

The builder worker issued no phone/Mac commands or flash. Root hardware admission/results are recorded in the canonical HANDOFF; this workbench receipt is build/emulator evidence.

## Pinned kernel23 output routing — root source review, Oct27 12:08 AM PT

Source-only review receipt: `$HOME/artifacts/handoff/redox-perplexity/OBSERVABILITY-HELPERS-ROOT.json`, including exact source hashes. Kernel cb1dfb1388a33674ccfa14bd20806f038ba42819 and loader 0ab58d911f930cf467d06ae41eeb972a944c13a5 remain unchanged. This resolves the earlier four-file review's routing uncertainty; it does not establish phone visibility.

- Kernel `src/macros.rs:12,34` routes println/info through `crate::log::Writer`; `src/log.rs:59` writes to an installed DEBUG_DISPLAY and then the architecture serial writer. Memory-log allocation later in startup does not gate these console writes.
- `src/devices/graphical_debug/mod.rs:110` parses five FRAMEBUFFER_* environment values. All must be nonzero before install_display. The installer creates the framebuffer DebugDisplay; its writer draws pixels and flushes dirty rows. `init_dtb` can replace the GOP path with a validated simple-framebuffer. Neither successful mapping nor visible pixels has yet been observed on this phone.
- Loader `src/main.rs:594,678` sets final video mode before page tables and exports mapped address, dimensions and stride when the first output has a selected mode. Missing GOP/output/mode can leave no environment framebuffer. U-Boot visibility alone proves none of these loader steps.
- Loader println resolves through the pinned uefi_std dependency 26a499eeaf55d42fb24206830345e26fb8f5f835, `src/macros.rs` and `src/io.rs`, to UEFI ConsoleOut.OutputString. This is the actual source path, not proof firmware successfully displays it. Loader `kernel_entry(...)` text is printed before entering the kernel and is not kernel-entry proof.
- Kernel `start.rs:71` attempts graphics before DT serial initialization at88. `Starting graphical debug` and the GOP framebuffer dimensions are printed BEFORE installing the new display, so they are unsuitable first-screen criteria. `Redox OS starting...` at92 is a better kernel marker conditional on successful display initialization and returning from the intervening DT/UART work. A supported UART setup can print earlier; absence is not definitive failure localization. A valid GOP path lets kernel logging reach framebuffer without working GENI serial.
- `src/dtb/mod.rs:614` selects chosen stdout; absent/unsupported serial can leave COM1 absent. Serial setup/MMIO, mapping, cache behavior and actual display remain physical uncertainties. Source ordering is not a bounded-time or hardware-liveness proof.

Next physical step remains capture the already requested NEW V2 diagnostic screen. No new reboot/RAM boot/installation is authorized by this source review. After V2 result/root review and fresh Linux recovery, rebuild the pinned Redox EFI with the corrected current loading path; independently review its admission. No stale staged image or accepted30 substitution. No new VM run or code change was needed for this review.
