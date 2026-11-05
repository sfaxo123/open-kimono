# K8 — integrated drive-free Redox build

**Result (2026-10-24, 4:38 PM PT):** The K8 kernel, base, and bootloader branches contain all available Wave 1 patch series. The real `aarch64-unknown-redox` cookbook toolchain inside Podman built `spmid` into the RAM initfs. QEMU booted the refreshed EFI payload through PXE with no drive, reached `ram:~$`, and printed `spmid: no SPMI arbiter in device tree; skipping`. The daemon therefore started and exited before mapping SDM845 MMIO on QEMU. No phone was touched, no remote was pushed, and phone boot remains unverified.

## Source and integration commits

| Worktree | Applied series | Final commit |
| --- | --- | --- |
| `kernel-k8`, `op6/integration` | DT inherited interrupts, PDC/TLMM, GENI UART | `f922469d` |
| `base-k8`, `op6/integration` | RAM rescue initfs, K7 `spmid`, K8 DT guard and service | `ef40656a` |
| `bootloader-k8`, `op6/integration` | Embedded kernel and initfs | `92ff2e7` |

All patches applied without source conflicts. The GENI patch commits were given the required Codex co-author trailer when imported. Exported series are in `patches/integration/{kernel,base,bootloader}/`. These are downstream forks.

## Rebuild and evidence

From this repository, run `scripts/integrate.sh --qemu`. It applies any missing source series, builds the kernel, builds base inside the existing Redox Podman image with the cookbook toolchain and sysroot, builds the EFI bootloader, refreshes the handoff payload and `SHA256SUMS`, verifies all recorded hashes, then boots QEMU. The build tree `$HOME/Developer/forks/redox/redox` is used only for toolchain and build outputs. The script's ignored logs are in `out/integration/` (`kernel-build.log`, `base-build.log`, `bootloader-build.log`, `sha256-check.log`, `qemu.log`).

The QEMU command uses `virt,acpi=off,gic-version=3`, `-cpu max`, `-smp 1`, EDK2 UEFI and a PXE/TFTP EFI payload. There is no `-drive`, block device, or CD-ROM. The captured serial log is [`K8-QEMU.txt`](./K8-QEMU.txt); it records:

```text
>>Start PXE over IPv4.
Redox OS Bootloader 1.0.0 on aarch64/UEFI
REDOX_RAMBOOT=1
spmid: no SPMI arbiter in device tree; skipping
init: RAM boot shell on /scheme/debug; no rootfs mounted
ram:~$
```

The 65-second QEMU timeout shut down the waiting rescue shell. The prior K3 direct UEFI test executed a command at that prompt; this K8 run confirms the integrated prompt and daemon behavior. QEMU cannot prove the OnePlus 6 SDM845 MMIO path, physical buttons, ABL, display, or USB handoff. K7's static SPMI window addresses and PMIC status polarity still require phone oracle validation before a hardware claim.

## Refreshed handoff payload

- File: `$HOME/artifacts/handoff/oneplus-redox-20261024/phone-boot/redox-ramboot.efi`
- Size: 9,287,168 bytes; embedded initfs: 7,557,408 bytes.
- SHA-256: `79e26539061b259031e27ec1cf27f00b001588ed104859b5f81d8b35622856e5`.
- `phone-boot/SHA256SUMS` has the refreshed entry; `sha256sum -c SHA256SUMS` passed for the full artifact set.

Repeated builds produced different EFI hashes even with the same source commits; the exact cause was not isolated. The script refreshes the hash list on every run. Use the hash of the specific file being transferred, and retain its matching QEMU log.

If rebuilding fails, keep the last verified payload and inspect the named log before retrying. To undo the integration in a worktree, reset only that isolated branch to its recorded pre-series base (`kernel a0a8ff96`, `base d2ffc4ca`, `bootloader cc922ec`); do not alter shared main or the phone.
