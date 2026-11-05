# Kernel series unification — `kernel/unify-stack`

Verified 2026-10-25, 5:30 PM PT. This branch has one active 21-patch kernel series, `patches/kernel-series/`, based on `a0a8ff96b065bd8f33ca3e0742fb09f246c1e32a`. `scripts/integrate.sh` applies it in filename order. The complete EMU-TOUCH self-parent rule and its host tests are patch 0013; the smaller EMU-BUTTONS inline guard was folded into that behavior. Touch Group 1 initialization was rebased after the private-IRQ/affinity changes so it runs after GIC distributor initialization. The resolved IRQ-width patches and opt-in sleep diagnostics are included. All old kernel patch directories were removed from the active tree and remain in Git history (`3c9cb4f`). Base and bootloader series still target separate repositories.

## Fresh `git am` and build

Only this worktree's `out/` was written. Source trees were read from `~/Developer/forks/redox/` and cloned/copied into `out/`; neither the phone nor the GPL QEMU checkout was edited.

```sh
git clone --shared --no-checkout ~/Developer/forks/redox/kernel-e1 out/kernel-series-fresh
git -C out/kernel-series-fresh checkout --detach a0a8ff96b065bd8f33ca3e0742fb09f246c1e32a
for p in patches/kernel-series/*.patch; do git -C out/kernel-series-fresh am "$PWD/$p"; done
make -C out/kernel-series-fresh ARCH=aarch64 OBJCOPY=aarch64-linux-gnu-objcopy
```

All 21 `git am` steps applied with a clean worktree. The resulting log, oldest first:

```text
1c37b5a0 Fix inherited devicetree interrupt domains on aarch64
cc3269a8 Add tested SDM845 PDC specifier decoder
bd626e7e Wire SDM845 PDC and TLMM interrupt domains
caea04f4 context: pass a Kstack in scheduler test helpers
6855762e devices: add polled Qualcomm GENI debug UART driver
eb21df40 dtb: select the GENI debug UART for qcom,geni-debug-uart consoles
485b1207 irq: carry IRQ numbers wider than u8 through the irq scheme
cb125c32 gicv3: keep the full INTID when acknowledging an interrupt
0694f8b1 devices: deliver GENI debug UART RX through the SE interrupt
1fafb358 dtb: discover a simple-framebuffer for the boot console
9fed4185 graphical_debug: scrolling, scaled, coloured boot console
b851bb21 aarch64: boot console on a device tree simple-framebuffer
1eed0327 dtb: treat a self-parented interrupt controller as a root
f5e972b5 gicv3 redistributor private irqs
aabb3998 reject malformed dt stride
08c6374e per controller cpu affinity registry
0ab21083 gicv3: put shared interrupts in Group 1
3f556644 irqchip: keep summary interrupts of driverless controllers masked
1df15cb9 aarch64: log first architected timer PPI for emulator gate
6b860773 context: retain due timers when a context lock is busy
d46bfa7e kernel: add opt-in bounded sleep IRQ diagnostics
```

`rustc --edition=2024 --test` on `irq_topology.rs`, `gic_groups.rs`, and `gicv3/model.rs` passed 2/2, 2/2, and 22/22. The fresh kernel `make` passed. `wy_sleep_diag` remains off in the default build.

## One combined image and runtime gates

The isolated `out/kernel-unify-forks/` contains kernel, base, bootloader and Redox cookbook copies. The build command was:

```sh
EMU_TOUCH=1 REDOX_FORKS="$PWD/out/kernel-unify-forks" scripts/integrate.sh --qemu
```

It applied unified kernel 0013–0021 after the 12 kernel commits already present in the pinned `op6/e1-demo` checkout. The resulting isolated kernel head was `b1a112c6`. It also applied base `integration/base/0011`–`0013` (SPMID) and `touch-e2e/base/0001`–`0003`; the resulting base head was `b8e34ed8`. The exact EFI, used unchanged by both OnePlus emulator runs, is `out/phone-boot/redox-ramboot-emu-touch.efi`, SHA-256 `0dfbec13e802b12d2902910dcaa953247c6d3b6d3954c722fd87f01487e745c3`. Both emulator receipts name this hash.

The `integrate.sh --qemu` generic `virt` boot passed its `ram:~$`, timeline 28 events, no-SPMI, vesad simplefb, and kernel fbcon assertions. Its serial SHA-256 was `f0fc0d4243aa240a5b28223d6486f90eb66b557594ee4b4ae970c5d293a26413`.

The buttons run used the same EFI and the documented emulator-only timer PPI DTB correction:

```sh
scripts/emu-op6/run.sh out/phone-boot/redox-ramboot-emu-touch.efi \
  --timer-ppi-fix --qmp-script docs/workbench/emu-buttons-qtest.json \
  --marker 'gicv3: affinity' \
  --marker 'architected timer virq = 30' \
  --marker 'architected timer PPI fired hwirq=30' \
  --marker 'spmid: PMIC type 0x51' --marker 'ram:~$' \
  --marker 'spmid: inputd key vol-down pressed=true' \
  --marker 'spmid: inputd key vol-down pressed=false' \
  --marker 'spmid: inputd key vol-up pressed=true' \
  --marker 'spmid: inputd key vol-up pressed=false' \
  --marker 'spmid: inputd key power pressed=true' \
  --marker 'spmid: inputd key power pressed=false' \
  --timeout 100 --run-id kernel-unify-buttons-retry
```

It passed, including PPI 30 on redistributor `0xffff800017a60000` and the six ordered key writes to `inputd`. The serial SHA-256 was `6c9846b991f3dab258b83ce7a1eaeb2e596a9d57733f14bad2b95ec63944a980`; QTest control log SHA-256 was `83f8d9d37030a45619e45a3b0f4ca233bd28fea83b36d66671d834a326f1f7f8`. The first bounded boot, `kernel-unify-buttons`, failed before the EFI loaded: U-Boot repeatedly logged `OCS error from controller = f` and `Bad device specification scsi 0`. Its serial SHA-256 was `abe64d35d544b066777907c61e9ba58cffb0ebeae9894c803c40333205be03f3`. The retry used the same EFI and emulator hash and passed; this was an emulator UFS startup failure, not a kernel execution result.

The touch run used that same EFI:

```sh
EMU_OP6_RUN_ID=kernel-unify-touch \
  scripts/emu-op6/touch-e2e.sh out/phone-boot/redox-ramboot-emu-touch.efi
```

It passed all 12 ordered markers: PDC, TLMM, PDT, wake route `TLMM125 -> PDC100 -> GIC SPI615` (`virq 647`), initial drain, press at (540,1140), `inputd` pointer and button-down, IRQ count 2, and button-up. Serial SHA-256: `88eff0ac7fa6112bf68d4dcfea7a408d5c94ff087dfbb3c6c577c062b883187f`. QMP transcript SHA-256: `e59a8c6606d71884b1ea00913039a602e839c4d4c8d186328c1d66313eec2db2`.

Compressed raw serial and control receipts are in `docs/workbench/kernel-unify-evidence/`. SHA-256 values above are for the uncompressed files; `gzip -dc` recovers those exact bytes. An initial touch wrapper invocation used a conflicting extra `--run-id` and was stopped before its injector could act; the corrected invocation above is the counted touch gate.

`scripts/qemu-demo/tests.sh` passed 73/73 after the worktree-local Wasm fixture was built with `cargo build --release --offline --target-dir "$PWD/out/qemu-demo/wasm"` from `crates/sandbox/guests/hello-rs`. Its initial run failed only the real-tree digest fixture because that generated Wasm file was absent. The Wyoming guest harness was skipped: `docs/workbench/E1-WY-GUEST-HARNESS.md` requires a separately reviewed VM phase and explicit Codex budget for each guest run. Its host tests or a previous run cannot substitute for that admission.

No physical phone, SMP, or UI consumer acceptance is claimed. Rollback is reverting the branch commits. No push or merge was performed.
