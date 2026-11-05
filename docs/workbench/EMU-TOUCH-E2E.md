# EMU-TOUCH: end-to-end touch on the virtual OnePlus 6

Verified 2026-10-26 00:15 UTC (Oct 25, 5:15 PM PT) on branch `emu/touch-e2e`. This is an emulator and host result. It is not a physical phone test; the phone was not touched.

## Result

A press at (540, 1140) injected into `-M oneplus6` reaches `inputd` with those coordinates, and the release follows. The serial log of the final run (`evidence/emu-touch/touch-e2e-final-serial.txt`) shows every step in order:

```text
858  interrupt-controller@17a00000: interrupt parent is itself; treating it as a root
865  PDC probe: phys=0xb220000 version=0x20700 ranges=3
868  TLMM probe: phys=0x3400000 GPIOs=150 virqs=672..822
907  rmi4d: PDT at I2C 0xa90000 address 0x20: F01 query=0x10 command=0x28 control=0x29 data=0x06 irqs=1; F12 query=0x50 command=0x00 control=0x70 data=0x80 irqs=2
909  IRQ wake route: controller 0x4a L2(125, 2) -> controller 0x8d pin 100 -> GIC L3(0, 615, 1) virq 647
910  rmi4d: attention IRQ /scheme/irq/phandle-74/125,2 armed; Data1 offset 0 slots 10
911  rmi4d: initial drain contacts=[]
912  rmi4d: attention IRQ count=1 contacts=[Contact { slot: 0, x: 540, y: 1140, pressure: 50 }]
913  inputd: producer pointer x=540 y=1140
916  inputd: producer button left=true
919  rmi4d: attention IRQ count=2 contacts=[]
920  inputd: producer button left=false
```

Each line proves one link in the chain:

- **PDT scan.** `rmi4d` finds F01 and F12 over the GENI I2C12 FIFO at `0xa90000`, slave `0x20`.
- **IRQ route.** The kernel claims the DT attention specifier `<&tlmm 125 IRQ_TYPE_EDGE_FALLING>` (phandle `0x4a`). Its route goes from TLMM GPIO125 to PDC (`0x8d`) pin 100, then to GIC SPI 615 (INTID 647).
- **IRQ delivery.** `rmi4d` only reads the touch controller at startup (the drain) and when that IRQ's count advances. Before the press, the count is 0. A no-injection control boot of the same payload ran for 75 s and logged zero `attention IRQ count=` lines.
- **Emulator trace.** A QEMU trace of the same path (`touch-debug3-fixed-trace-excerpt.txt`) shows `rmi4_s3706_touch ... x 540 y 1140`, then distributor INTID 647 going high, then `ICC_IAR1 ... value 0x287` (647).

The final run's receipt (`touch-e2e-final-receipt.tsv`):

| artifact | SHA-256 |
|---|---|
| emulator `qemu-oneplus6` `462dbe3` build | `588ac3f028c74a73322c3f1cbac31228650bd7a6d85f76bbe79f38e80519188c` |
| payload `redox-ramboot-emu-touch.efi` | `9e6d227c3be297ced3bbf7ac6a7b894afed56d942aca350f654fda2f92bf4252` |
| serial log | `f046c9a4a2511e6bb221af4915e50181b460529199d8a24d7b787fd0e9c68b1c` |

All 18 `-M oneplus6` boots in this task used that emulator hash. Payload hashes differ per build, as noted in GOTCHAS.

## How to reproduce

The build and the run use the shared forks only through an isolated copy:

```sh
W=$PWD/out/emu-touch-build/forks   # copies of kernel-e1/base-e1/bootloader-e1 with standalone .git; redox -> shared
EMU_TOUCH=1 REDOX_FORKS=$W scripts/integrate.sh          # out/phone-boot/redox-ramboot-emu-touch.efi
scripts/emu-op6/touch-e2e.sh                             # boots it and injects through QMP
```

Each copy is a `cp -a` of the shared worktree. Its `.git` file is replaced by a `git clone --no-checkout` of the same branch, then `git reset`. The shared checkouts were not modified.

`touch-e2e.sh` passes `-qmp unix:<tmp>,server=on,wait=off` to QEMU through the new `run.sh --qemu-arg`. Once `rmi4d: initial drain` appears, it sends `input-send-event` with `btn left down`, `abs x 540` and `abs y 1140`. After `inputd` logs the press, it sends the release (`btn left up` with the same X/Y). The events carry no `device`, so the `rmi4-s3706` handler receives `src == NULL` and keeps raw values up to `max_x`/`max_y`; there is no tablet-style scaling. This is the model's QEMU input handler from REPORT-M6, not `usb-tablet`. The script requires 12 ordered markers. The QMP transcript is in `touch-e2e-final-qmp.txt`.

## Bugs found and fixed

The first three bugs each blocked the path on `-M oneplus6` with the real phone DTB. All fixes are MIT and live in `patches/touch-e2e/`.

| # | Symptom (red serial/trace) | Cause | Fix | Host test |
|---|---|---|---|---|
| 1 | Baseline boot: no `TLMM probe`/`PDC probe` at all, `failed to translate architected timer interrupt`, `serial port interrupt configuration failed` (`touch-baseline-serial.txt` 858–864) | The phone DTB's root `interrupt-parent` is the GIC, and the GIC names its own maintenance PPI. The GIC therefore recorded a parent link to itself. The only roots were the driverless PMIC GPIO controllers, so the GIC, TLMM and PDC were never initialized. | Kernel 0001: a self-parented controller is a root, as in Linux `of_irq_init`. The patch also logs the wake route. | `src/dtb/irq_topology.rs`: 2 tests, red with the old rule, green after. |
| 2 | Route armed, press injected, no IRQ in `rmi4d` (`touch-e2e-try1-serial.txt`). Trace: INTID 647 goes high and `GICD_ISENABLER20=0x80` is written, but there are 0 `ICC_IAR1` reads in the whole boot (`touch-debug1/2-trace-excerpt.txt`). | The GICv3 path reused the GICv2 distributor init, so every SPI stays Group 0. With the system-register interface Group 0 is FIQ, and only Group 1 is enabled. | Kernel 0002: all SPIs go to Group 1 (`GICD_IGROUPR`), as Linux `gic_dist_init` does. These registers are RAZ/WI under secure firmware. | `gic_groups.rs`: 2 tests, red with no group writes, green after. |
| 3 | `rmi4d` panics at start: `drivers common: memory root fd not initialized`, then `UNHANDLED EXCEPTION ... rmi4d` (`touch-red-rmi4d-long-serial.txt` 903–973) | The same class of bug as the SPMID crash: `PhysBorrowed::map` ran before `common::init`. `geni-i2cd` had it too. | Base 0001: both daemons call `common::init` first. | Not host-testable (Redox-only path); fixed in serial, as above. |
| 4 | After fix 1, the kernel enabled summary SPIs for driverless controllers (LMH 32/33, MDSS 83, SPMI 481, codec on TLMM 54; `touch-debug2-trace-excerpt.txt` ISENABLER writes). | The `Null` placeholder never EOIs, so one such interrupt would hold the shared priority and block every device IRQ, including touch. It was latent until fix 1 let the tree initialize. | Kernel 0003: reserve the line but keep it masked (`... stays masked`, final serial 862–869). Only TLMM summary, UART, timer and touch enables remain (`touch-debug4-masked-trace-excerpt.txt`). | Emulator trace only. |

`rmi4d` was also reworked from a 16 ms poll with a hard-coded base. It now:

- reads `/scheme/kernel.dtb` and requires an enabled `syna,rmi4-i2c` at `0x20` under an enabled `qcom,geni-i2c` node;
- takes the base from the DT and parses `interrupts-extended`;
- claims `/scheme/irq/phandle-<tlmm>/125,2` and waits on it with an event queue, acking after each read;
- does one drain read after arming, because an edge latched before arming would never repeat.

Pointer events come from a host-tested `Tracker`. The `--trace` flag prints contacts and sent events. `inputd` gets an opt-in `INPUTD_TRACE_POINTER` print.

Host and harness receipts are in [`EMU-TOUCH-TESTS.txt`](EMU-TOUCH-TESTS.txt):

- the kernel helpers red, then green;
- the new `rmi4-touch-core` tests: 5 new tests (compile-red before the API existed), 10 total pass in the project crate and in the base copy;
- `geni-i2c-core`: 4 tests;
- the host stub check of `geni-i2cd`, `rmi4d` and `inputd`;
- `scripts/emu-op6/tests.sh`: 19/19, including the new `--qemu-arg` checks.

## What changed

- `patches/touch-e2e/kernel/0001–0003`: the self-parented root, SPI Group 1, and masked driverless summaries.
- `patches/touch-e2e/base/0001` (`rmi4d`/`geni-i2cd` plus core), `0002` (the `inputd` trace), `0003` (`RAMBOOT_EMU_TOUCH` and `init.emu-touch.d`). Both series apply with `git am` to fresh clones of the shared `op6/e1-demo` branches (base after `integration/base/0011`).
- `crates/rmi4-touch`: `Attention` and `Tracker`, with tests (the project copy of `rmi4-touch-core`).
- `scripts/integrate.sh`: applies both series. `EMU_TOUCH=1` adds the emulator services and writes `redox-ramboot-emu-touch.efi` with `SHA256SUMS.emu-touch`. The default build still asserts there is no `30_rmi4d.service`.
- `scripts/emu-op6/run.sh`: `--qemu-arg` (repeatable).
- `scripts/emu-op6/touch-e2e.sh`: the QMP injection driver.

`rmi4d` still needs an explicit `DATA1_OFFSET SLOTS`. The emulator service passes `0 10`, the model's layout from REPORT-M6 (F12 data `0x80`, 10 × 8-byte slots), not oracle data. The phone ramboot does not start `rmi4d`.

## Regression gates on the final sources

- Default build `scripts/integrate.sh --qemu` on generic QEMU `virt`: exit 0. All existing greps pass (`ram:~$`, 28 trace events, `spmid: no SPMI arbiter`, `vesad: simplefb 800x600`, the fbcon line). Serial: `touch-final-default-virt-qemu-serial.txt`.
- Default payload on `-M oneplus6`: `TLMM probe`, `spmid: PMIC type 0x51` and `ram:~$`, with zero `rmi4d` lines (`touch-final-default-op6-serial.txt`).
- Builds with kernel 0001–0002 but before 0003 passed three further touch runs (`try2`, `run2`, `run3`; payload `31e50d21…`). Those logs are not committed.

## Gaps and limits (not proven here)

- **No consumer after `inputd`.** Every event logs `queued pointer event for 0 consumer(s) on VT None`. The RAM boot has no Orbital or fbcond consumer, so events stop inside `inputd`. Orbital touch is a separate gate.
- **Timer PPI still not taken on this board.** The final traces have exactly two `ICC_IAR1` reads, both INTID 647. `architected timer virq = 18` now translates, but the PPI enable goes to the distributor; that is the job of the separate `kernel-gicv3-private-irq` series, which is not applied here. UART RX (virq 386) was not exercised.
- **Real-hardware unknowns.** These remain unknown:
  - the s3706b Data1 offset and slot count;
  - whether the controller reports F01 "unconfigured" (status bit 7) until the host sets F01 `ctrl0` bit 7. `rmi4d` never sets it, and the model always reports configured.
  - clocks, pinctrl and rails for I2C12;
  - whether firmware leaves INTID 647 Group 1.
- **Status bits not checked on hardware.** `geni-i2c-core` treats IRQ status bits 6–7 as errors, but Linux `geni-se.h` names them `M_TIMESTAMP_EN` and `M_RX_IRQ_EN`. The emulator never sets them.
- **Single pointer.** `Tracker` reports the first finger only. Multi-touch MTT events were not injected.

## Emulator gaps for qemu-oneplus6 (read-only here; GPL tree not edited)

- `rmi4-s3706` only clears a slot on an ABS-carrying sync. A button-up with no X/Y leaves the finger down, so `touch-e2e.sh` repeats the coordinates on release.
- `rmi4-s3706` F01 device status is always 0 (configured). It never models the post-reset unconfigured bit, and the PDT exposes only F01 and F12.
- The GICv3 is created without security extensions, so the kernel must assign groups itself; on the phone, firmware does that. The board also gives TLMM, PDC and GENI I2C no trace events; only `rmi4_s3706_*` and `gicv3_*` exist.
- TLMM GPIO125 drives the PDC from the raw pad level whatever TLMM `INTR_CFG` says. That matches SDM845 wake routing, but it means the TLMM summary path for GPIO125 was not exercised.

## Note for the sibling buttons worker

`redox-oneplus6-emu-buttons-e2e/out/emu-buttons-build/forks/base-e1/.git` still points at the shared `base/.git/worktrees/base-e1` (seen read-only). A `git am` there would move the shared worktree's HEAD; see the SPMID gotcha.
