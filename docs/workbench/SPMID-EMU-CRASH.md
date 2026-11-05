# SPMID OnePlus 6 emulator crash

Verified 2026-10-25, 1:14 PM PT on `fix/spmid-emu-crash`. This is an emulator and host result, not a physical phone test.

## Reproduction and cause

The M10 UFS boot requires the disposable U-Boot DTB's GCC `power-domains` property removed, a `ufs0` drive, `scsi scan`, and `bootefi` with `${fdtcontroladdr}`. `scripts/emu-op6/run.sh` now uses that path. The original payload SHA-256 was `3b025a82185e16daf11f6948e35e8c0e5d957b4f7007771908251e0111490f28`; baseline serial SHA-256 was `4b6b1b3a1e88c4b3ca56f704150abdf5829ed0fc2c60d1f4b5c95437d6c603ac` at `out/emu-op6/spmid-baseline-m10-ufs/serial.log`.

```text
thread 'main' (1) panicked at drivers/common/src/lib.rs:56:10:
drivers common: memory root fd not initialized. Please call `common::init` in your main function.
[ERROR spmid@src/header/stdlib/mod.rs:121] Abort
  00007fffffff3700: GUARD PAGE
kernel::context::signal:INFO -- UNHANDLED EXCEPTION, CPU #0, PID 9, NAME /scheme/initfs/bin/spmid
```

The daemon mapped MMIO with `PhysBorrowed::map` before calling `common::init`. The BRK exception was a Rust panic on that missing memory-root descriptor. The nearby guard page does not establish a stack overflow. The daemon also allocated a 16 KiB APID lookup table on its stack (`4096 * size_of::<Option<u16>>()` on this host); that separate risk is now a heap vector.

Linux `drivers/spmi/spmi-pmic-arb.c` v5 skips zero APID map entries and reads ownership at `cnfg + 0x700 + 4 * APID`. The daemon used `cnfg + 4 * APID` and accepted empty entries. QEMU's `qcom-spmi-pmic-arb.c` reports 512 slots, with only APID 1 (REVID) and 34 (PON) populated; its config reads return zero at every offset, which masks the daemon's wrong owner offset. The model has no GPIO5/6 peripherals, so volume sampling returns `BadChannel`. That is an emulator coverage gap for volume-button tests, not the cause of the startup exception. No emulator file was edited.

## Fix and tests

The downstream `0011` patch calls `common::init` before any mapping, skips empty APIDs, uses the v5 owner offset, puts the lookup table on the heap, removes MMIO adapter assertions, logs hardware errors, and retries. It connects to `inputd` only when an edge needs publishing; otherwise `ENODEV` at the producer endpoint prevented button sampling. A failed sample resets the edge baseline, avoiding a false key event after recovery.

The new host replay with a 512-slot sparse map, APIDs 1/34, and absent GPIO failed against the original code (`pmic_type`: `Err(BadChannel)` versus `Ok(0x51)`). After the fix:

```text
cargo test --manifest-path crates/spmid/Cargo.toml --offline
8 passed; 0 failed
cargo test -p spmid-core -p spmid --offline  # isolated Redox base copy
8 passed; 0 failed
cargo check -p spmid --features host-compile --offline  # isolated Redox base copy
Finished dev profile; exit 0
scripts/emu-op6/tests.sh
Summary: 17 passed, 0 failed
```

`REDOX_FORKS=<isolated worktree copy> scripts/integrate.sh` rebuilt the AArch64 ramboot image with the documented initfs packer and bootloader Makefile. The final payload is `out/phone-boot/redox-ramboot.efi`, SHA-256 `e734abf28fb93347f351b0622b93f26ba939caeaf2b4207b42f50650db1f070e`.

## Emulator result

Command: `scripts/emu-op6/run.sh out/phone-boot/redox-ramboot.efi --marker 'spmid: PMIC type 0x51' --marker 'spmid: SPMI button read: BadChannel; retrying' --marker 'ram:~$' --timeout 100 --run-id spmid-fixed-m10-v2`. It exited zero. `out/emu-op6/spmid-fixed-m10-v2/serial.log` has SHA-256 `d8519661970151a276df999087a4171edf575d2ca994fd9d9d4380a418b95962` and no `panicked` or `UNHANDLED EXCEPTION` line. Its excerpt:

```text
timeline-demo-publish: sent 28 events to trace:publish
spmid: PMIC type 0x51; power key enabled per OpenBSD qcpon status bit; verify on hardware before autostart
spmid: SPMI button read: BadChannel; retrying
Redox RAM shell. Type help for commands.
ram:~$
```

The PMIC revision read succeeds and the daemon handles the absent GPIO channel without an exception. This run does not prove volume input, `inputd` event delivery, or physical phone behavior. The shared Redox checkouts were verified clean after the isolated build; the build copy's Git metadata initially pointed to the shared `base-e1` worktree, and that accidental branch/index change was restored to its pre-build commit `82a70629`. Future isolated copies must replace `.git` pointers before `git am`.
