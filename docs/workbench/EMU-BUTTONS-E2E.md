# EMU-BUTTONS end-to-end result

Verified 2026-10-25, 4:45 PM PT on `emu/buttons-e2e`. This is a OnePlus 6 emulator result. The phone and GPL QEMU checkout were not changed.

## Result

The final run booted the EFI payload through the phone U-Boot, reached `ram:~$`, and passed ordered serial markers for all six key edges. The key log is emitted **after** `ProducerHandle::write_event` succeeds, so it proves spmid published each event to the `inputd` producer endpoint. It does not prove UI consumption on a physical phone.

| Serial line in [final.serial.log.gz](emu-buttons-evidence/final.serial.log.gz) | Receipt |
|---|---|
| 861, 870, 881 | GICv3 chose redistributor `0xffff800017a60000`; nonsecure timer virq 30 was registered; `architected timer PPI fired hwirq=30`. |
| 896, 898–899 | PMIC type `0x51`, initial released sample, RAM shell. |
| 905–908 | Volume down press and release samples and `inputd` writes. |
| 909–912 | Volume up press and release samples and `inputd` writes. |
| 913–916 | Power press and release samples and `inputd` writes. |

Payload SHA-256: `51636a0a36343722b3f8565150d2aa32669e8729e458598addd4a1d65ca51792`. Final serial SHA-256: `4aee4e35887fb0d3f9f60c05644bb9cb9a6366e49ef03630bcfc20ba1f38264c`. The [receipt](emu-buttons-evidence/final.receipt.tsv) binds the emulator, payload, disposable ESP, U-Boot source/copy, and serial digests. [Control log](emu-buttons-evidence/final.control.log.gz) records six `set_irq_in` replies of `OK` in the same order. The `.log.gz` files preserve the raw serial bytes; `gzip -dc <file>` reads them.

## Build and run

The source and toolchain were copied to ignored `out/emu-buttons-build/forks/`. Patch `0011`, kernel GICv3 `0001` and `0002`, and `kernel-sleep-wake/0001` were applied there, along with the branch's `0012`/`0013` spmid and `0003`/`0004` kernel follow-ups. `REDOX_PATCHES_APPLIED=1 REDOX_FORKS=$PWD/out/emu-buttons-build/forks scripts/integrate.sh` built the final EFI. The branch's `0013` changes PM8998 PON RT_STS power sampling from bit 7 to bit 0; its host replay checks bit 0 as pressed and bit 7 alone as released. `spmid-core` host tests: 8 passed. The full `spmid` host crate test hit the existing stable-toolchain `redox-scheme` feature error, so the successful target build and serial gate are the acceptance evidence.

```sh
scripts/emu-op6/run.sh out/phone-boot/redox-ramboot.efi \
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
  --timeout 100 --run-id buttons-e2e-final
```

The harness fixture suite passed 17/17. Both patch series reapply cleanly to isolated copies of the original sources. The GPL emulator and shared Redox source checkouts remain clean.

## Emulator gaps

1. `--qmp-script docs/workbench/emu-buttons-qmp.json` contains the requested serial-triggered `qom-set` commands. A probe sent its first command after the EFI diagnostic banner and failed: `Property 'qcom-spmi-pmic-arb.vol-down[0]' is not writable`. See [QMP response](emu-buttons-evidence/qmp-probe.qmp.log.gz) and its [serial run](emu-buttons-evidence/qmp-probe.serial.log.gz). The GPL device defines named GPIO-in lines, but QMP cannot drive these read-only QOM child properties. The MIT harness therefore also supports QTest `set_irq_in` commands in the same script format; the successful run used that interface while TCG continued running. No GPL source was edited.
2. The unmodified appended phone DTB describes timer PPIs 1/2/3/0; this emulator wires the architectural timers to INTIDs 29/30/27/26 (PPIs 13/14/11/10). In the [unmodified timer serial run](emu-buttons-evidence/unmodified-timer.serial.log.gz), the kernel registered timer virq 18 and spmid logged only its initial sample; no timer-PPI receipt or key event followed the QTest level changes. `--timer-ppi-fix` rewrites **only the per-run U-Boot DTB copy** to the emulator's PPI wiring. The final serial then records virq 30, a fired PPI, and all key edges. This correction is an emulator accommodation, not proof that the phone DTB needs changing.
3. U-Boot still logs its GPIO1 probe failure before Redox boots. Redox's direct PMIC GPIO5/6 sampling succeeds in the final serial. The emulator's GPIO1/pinctrl fidelity and a native QMP button injection API remain follow-up work in the GPL repository.

No phone test, physical interrupt validation, or downstream UI key handling is claimed.
