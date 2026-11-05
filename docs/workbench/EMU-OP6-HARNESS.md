# EMU-OP6-HARNESS — MIT test harness for `-M oneplus6` (`qemu-oneplus6`)

## Overview and license boundary

`scripts/emu-op6/run.sh` and `scripts/emu-op6/tests.sh` are MIT-licensed harness scripts in `redox-oneplus6` that invoke the external GPL-2.0 `qemu-oneplus6` emulator binary as a subprocess without copying any QEMU code.

- **Harness script:** `scripts/emu-op6/run.sh`
- **Fake-QEMU test suite:** `scripts/emu-op6/tests.sh`
- **Default emulator binary (`QEMU_OP6`):** `~/Developer/forks/qemu-oneplus6/build/qemu-system-aarch64`
- **Default phone boot artifacts:** `$HOME/artifacts/handoff/oneplus-redox-20261024/phone-boot/`

## Usage and contract

```sh
scripts/emu-op6/run.sh <payload> [--marker STR ...] [--qmp-script FILE] [--timer-ppi-fix] [--timeout S] [--run-id ID]
```

1. **Emulator discovery:** Resolves `QEMU_OP6` (default `~/Developer/forks/qemu-oneplus6/build/qemu-system-aarch64`) and verifies it is executable.
2. **Phone U-Boot preparation:** Reads `u-boot-nodtb.bin.gz-dtb` (verifying SHA-256 `c50fec59e1c6bedaa5077b324615bf3eb001315144501da850d007b0a7619662` unless overridden via `EMU_OP6_UBOOT_SHA256`), decompresses the raw U-Boot binary, extracts the appended enchilada DTB, patches `/memory@80000000/reg` to the 4 GiB bank (`0 0x80000000 1 0`) and removes GCC's `power-domains` property if present in this disposable emulator copy, then writes `out/emu-op6/<run-id>/oneplus6-u-boot-with-ram.bin`. This matches the M10 runner's U-Boot workaround.
3. **ESP image copy & payload staging:** Copies `op6-boot-esp.img` to `out/emu-op6/<run-id>/esp.img` (leaving the base image untouched) and places `<payload>` at `/EFI/BOOT/BOOTAA64.EFI` in the FAT32 ESP partition (offset `1048576`) via `mcopy -m -o`.
4. **Serial prompt driving:** Connects a named pipe (`serial.in`) to QEMU's `-serial stdio` input and captures output to `out/emu-op6/<run-id>/serial.log`:
   - On `Press any key to continue...`, sends `\r`.
   - On `*** U-Boot Boot Menu ***`, sends `5` (Drop to shell).
   - On `=> `, sends `scsi scan; load scsi 0:1 0xa0000000 /EFI/BOOT/BOOTAA64.EFI; bootefi 0xa0000000 ${fdtcontroladdr}\r` in paced 16-byte chunks (avoiding overflow of the 64-byte SDM845 GENI UART RX FIFO). The disposable ESP is attached as `-drive if=none,id=ufs0`.
5. **Ordered marker verification:** Verifies all `--marker STR` arguments appear in `serial.log` in strict sequential order. Out-of-order or missing markers fail with a non-zero exit code.
6. **Timeout and process reaping:** Wraps QEMU in `timeout --foreground --signal=TERM --kill-after=<K> <timeout>` with `-pidfile`, and installs `EXIT`/`INT`/`TERM` traps that terminate and (`SIGKILL` if needed) both the `timeout` wrapper and the `-pidfile` QEMU process on every exit path.
7. **Receipt generation:** Writes `out/emu-op6/<run-id>/receipt.tsv` with the SHA-256 digests and paths of `emulator`, `payload`, `image` (the per-run ESP copy), `uboot_image`, `uboot_source`, and `serial_log`.
8. **Scripted inputs:** `--qmp-script` reads JSON with a serial `marker` and ordered `commands`. Standard entries are QMP `execute`/`arguments` requests. The harness records responses in `qmp.log` and fails on QMP errors. This emulator's GPIO-in QOM children reject `qom-set` as read-only; for button tests, entries with `qtest_set_irq_in: {"line":"vol-down|vol-up|pwrkey","level":0|1}` use the QTest control socket while TCG continues running. The supplied scripts and exact result are in [EMU-BUTTONS-E2E.md](EMU-BUTTONS-E2E.md).
9. **Emulator timer correction:** `--timer-ppi-fix` changes only the per-run appended DTB's `/timer/interrupts` from PPI 1/2/3/0 to 13/14/11/10, matching the QEMU board's timer wiring. The original phone U-Boot source and base ESP stay unchanged. This opt-in correction is required for polling button input on the current emulator binary.

## Fake-QEMU fixture suite (`scripts/emu-op6/tests.sh`)

`scripts/emu-op6/tests.sh` runs without booting a real VM and verifies marker ordering, U-Boot prompt serial handshaking, timeout termination against a `SIGTERM`-ignoring fake QEMU, orphan prevention on normal exit and external `SIGTERM`, and `receipt.tsv` / ESP copy integrity:

```text
$ scripts/emu-op6/tests.sh
1. marker order & serial prompt driving
  PASS  in-order markers with U-Boot prompt handshake pass
  PASS  out-of-order markers are rejected (exit 1)
  PASS  out-of-order failure names the ordering violation
  PASS  missing marker is rejected (exit 1)

2. timeout enforcement with kill-after
  PASS  timeout exits non-zero (exit 1)
  PASS  timeout killed stubborn fake QEMU (PID 1383001 reaped)

3. no orphan qemu on exit paths (trap)
  PASS  marker-matched run with lingering fake QEMU exits 0
  PASS  no orphan QEMU after normal exit (PID 1383304 reaped by EXIT trap)
  PASS  no orphan QEMU after SIGTERM to run.sh (PID 1383688 reaped by trap)

4. receipt.tsv and ESP copy integrity
  PASS  receipt.tsv written for passing run
  PASS  receipt.tsv also written on timed-out run
  PASS  receipt.tsv records exact emulator SHA-256
  PASS  receipt.tsv records exact payload SHA-256
  PASS  receipt.tsv records exact ESP copy SHA-256
  PASS  receipt.tsv records exact serial.log SHA-256
  PASS  ESP copy contains payload at /EFI/BOOT/BOOTAA64.EFI
  PASS  base ESP image remains unmodified

Summary: 17 passed, 0 failed
```

## Real acceptance run with `efi-diag.efi` reaching U-Boot prompt (`=> `)

Command:

```sh
scripts/emu-op6/run.sh $HOME/artifacts/handoff/oneplus-redox-20261024/phone-boot/efi-diag.efi \
  --run-id m7-efi-diag-uboot-prompt \
  --marker '=> ' \
  --timeout 30
```

Output:

```text
run_id=m7-efi-diag-uboot-prompt
serial=$HOME/Developer/experiments/redox-oneplus6-emuharness/out/emu-op6/m7-efi-diag-uboot-prompt/serial.log
receipt=$HOME/Developer/experiments/redox-oneplus6-emuharness/out/emu-op6/m7-efi-diag-uboot-prompt/receipt.tsv
```

### `out/emu-op6/m7-efi-diag-uboot-prompt/receipt.tsv`

```tsv
artifact	sha256	path
emulator	71a2bbc148615adb1760489fd45cbac1255b4f3fa66f3a87e3ca0b026c62103e	$HOME/Developer/forks/qemu-oneplus6/build/qemu-system-aarch64
payload	1c7f666200771ed480c9278e32525baaa11dee2ff1742eea56b2992d0ebf4679	$HOME/artifacts/handoff/oneplus-redox-20261024/phone-boot/efi-diag.efi
image	0e23d863815f427f7dcfd66d8e87631bdc3a953ad91656ab22ca69aafad2ab33	$HOME/Developer/experiments/redox-oneplus6-emuharness/out/emu-op6/m7-efi-diag-uboot-prompt/esp.img
uboot_image	867f1332593662439d74f5821065ac4843b55d810a2ca9b3fea4a2a6b38a2056	$HOME/Developer/experiments/redox-oneplus6-emuharness/out/emu-op6/m7-efi-diag-uboot-prompt/oneplus6-u-boot-with-ram.bin
uboot_source	c50fec59e1c6bedaa5077b324615bf3eb001315144501da850d007b0a7619662	$HOME/artifacts/handoff/oneplus-redox-20261024/phone-boot/u-boot-nodtb.bin.gz-dtb
serial_log	3acc03ac5fc1fd3e1cbd2424be3cbe06a94d267f168c61d3ab3d885e3194d58f	$HOME/Developer/experiments/redox-oneplus6-emuharness/out/emu-op6/m7-efi-diag-uboot-prompt/serial.log
```

### `out/emu-op6/m7-efi-diag-uboot-prompt/serial.log` (captured transcript)

```text
U-Boot 2026.10-g8649a54d301a (Oct 24 2026 - 23:18:35 +0000)

Model: OnePlus 6
DRAM:  4 GiB
Error: qcom_smmu_connect: failed to allocate context bank for geniqup@ac0000
Core:  218 devices, 29 uclasses, devicetree: separate
WDT:   Not starting watchdog@17980000
MMC:   
Loading Environment from nowhere... OK
button_gpio gpio-keys: pinctrl_select_state_full: uclass_get_device_by_phandle_id: err=-22
button_gpio gpio-keys: pinctrl_select_state_full: uclass_get_device_by_phandle_id: err=-22
Button key-vol-down failed to probe
Button key-vol-up failed to probe
In:    serial,button-kbd
Out:   serial,vidconsole
Err:   serial,vidconsole
Model: OnePlus 6
smem: SMEM is not initialized by SBL
WARNING at drivers/soc/qcom/smem.c:649/qcom_smem_get()!
smem: Couldn't find socinfo: -22
WARNING: GDSC 3 is stuck during power on/off
Net:   No ethernet found.
Error: qcom_smmu_connect: failed to allocate context bank for geniqup@8c0000
GENI-SE: GENI SE wrapper not found

scanning bus for devices...
WARNING: GDSC 3 is stuck during power on/off
No button labelled 'Volume Down'
Hit any key to stop autoboot: 0
Cannot persist EFI variables without system partition
fw_name is not defined. Not generating capsule GUIDs
Press any key to continue...
*** U-Boot Boot Menu ***
...
=> fatload scsi 0:1 $loadaddr /EFI/BOOT/BOOTAA64.EFI; bootefi $loadaddr $fdt_addr_r
** Bad device specification scsi 0 **
Couldn't find partition scsi 0:1
Can't set block device
No UEFI binary known at 162c00000
=> 
```

> **Note on current `-M oneplus6` milestone state:** On the current `qemu-oneplus6` binary (`c582943`, M7 board + standalone M8 `qcom-ufshc` + standalone M9 `mmio-replay`), `qcom-ufshc` is not yet wired into `hw/arm/oneplus6.c`, so `fatload scsi 0:1` reports `** Bad device specification scsi 0 **` and returns cleanly to `=> `. Once `-M oneplus6` wires `qcom-ufshc` to the `-drive` passed by `run.sh`, the same command loads `/EFI/BOOT/BOOTAA64.EFI` from the staged ESP copy and executes `bootefi`.

## Extra QEMU arguments and touch injection (EMU-TOUCH)

`run.sh --qemu-arg ARG` (repeatable) appends arguments to the QEMU command line. The fake suite checks their order and that spaces survive (`tests.sh` section 5, 19/19). `scripts/emu-op6/touch-e2e.sh [payload [run.sh options]]` uses it for `-qmp unix:<tmp>,server=on,wait=off`. It injects a press and a release at `EMU_TOUCH_X`/`EMU_TOUCH_Y` (default 540,1140) through `input-send-event` and requires the serial PDT → TLMM/PDC/GIC route → `rmi4d` IRQ → `inputd` markers in order. Report: [EMU-TOUCH-E2E.md](EMU-TOUCH-E2E.md).

