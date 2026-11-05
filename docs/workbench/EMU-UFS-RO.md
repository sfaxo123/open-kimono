# EMU-UFS-RO — read-only UFS probe on `-M oneplus6`

Verified October 25, 2026, 10:30 PM PT on branch `emu/ufs-ro`. The opt-in Redox `ufsd` completed the requested UFSHCI and LUN0 GPT path in the emulator. This is an emulator result; the physical phone and the GPL `qemu-oneplus6` checkout were not changed. This branch was neither pushed nor merged.

## Result

The unified-kernel final payload passed all ordered markers through `scripts/emu-op6/run.sh` (run ID `ufs-ro-unified-final`). The raw serial is preserved as [final.serial.log.gz](emu-ufs-evidence/final.serial.log.gz), and [final.receipt.tsv](emu-ufs-evidence/final.receipt.tsv) binds the payload, emulator, per-run ESP, U-Boot source/copy, and serial hashes. The serial also logged `architected timer PPI fired hwirq=30` before `ufsd`. `gzip -dc` restores the raw serial bytes.

```text
904 ufsd: MMIO CAP=0x0107021f VER=0x00000210
910 ufsd: HCE enabled: CAP=0x0107021f VER=0x210
911 ufsd: link startup OK (HCS.DP set)
914 ufsd: NOP OUT/IN OK
916 ufsd: fDeviceInit done
919 ufsd: READ CAPACITY LUN0: last_lba=131071 block_size=512
920 ufsd: READ LBA 0..33 OK (34 x 512 B)
921 ufsd: GPT header at LBA 1: "EFI PART" entries_lba=2 entries=128 entry_size=128
922 ufsd: ESP entry 0: name="OP6BOOT" lba 2048..129023
923 ufsd: done: read-only session, no WRITE opcode was sent
```

SHA-256: unified kernel `e79a4c9fa76b679886afe9136aef50b7907b90958bf16eb1cfb31a07232659ee`; payload `aaeb770408937091298f67620d600b7625f87fb5263aeb9562c47553d4fe6210`; raw serial `75a10396067925b71ddf227e187eeb0ca4e0d8492f136e83c468f34ec5454b65`; emulator `588ac3f028c74a73322c3f1cbac31228650bd7a6d85f76bbe79f38e80519188c`; per-run ESP `c40f2e2c34c3722f038e51d5f42c3afd5294b0a32974ae8e1e3da73867a640ab`. The compressed serial itself hashes to `7aa195d223922bb0450c81882543176866994851675962cb579ffe5917718eaf`.

## Code and safety gate

Patch `0014` adds the opt-in daemon and host core; [patch `0015`](../../patches/integration/base/0015-ufsd-probe-mapped-controller-and-emulator-gpt.patch) corrects its controller base to the DT `0x01d84000`, prints CAP/VER before DMA allocation, uses the emulator's generic UFSHCI path, reads LUN0 without optional descriptor enumeration, and corrects the standard ESP type GUID. A fresh worktree at the `0014` base commit applied `0015` with `git am` successfully. The scratch kernel's temporary `UFSDBG` fault prints were saved under ignored `out/` and removed from the final kernel; none is in the committed patch series.

`cargo test --offline --manifest-path crates/ufs/Cargo.toml` passed 47 host tests. The `Cmd` enum emits only `00`, `03`, `12`, `25`, and `28`; the negative test refuses WRITE-class raw CDBs before MMIO or DMA. The emulator serial proves the accepted read path, while the no-WRITE claim also relies on that code and host gate. Controller register writes and the volatile UFS `fDeviceInit` SET_FLAG are required for startup; no media WRITE command is emitted.

## Reproduce

The payload was built in this worktree's isolated `out/emu-ufs-build/forks/` source copies. Kernel commits `0013`–`0021` from the unified series in [KERNEL-SERIES-UNIFY](KERNEL-SERIES-UNIFY.md) were applied to its first 12 kernel commits. Base patch `0014` was followed by `0015`. The final kernel contained no `UFSDBG` string. `REDOX_PATCHES_APPLIED=1 REDOX_FORKS="$PWD/out/emu-ufs-build/forks" scripts/integrate.sh` is the integrated build entry point after applying the patches. The final incremental build used `redox-base make ramboot-initfs` inside the same cookbook Podman toolchain, then `make -C out/emu-ufs-build/forks/bootloader-e1/build -f ../Makefile TARGET=aarch64-unknown-uefi` with `RAMBOOT_KERNEL` and `RAMBOOT_INITFS` pointing to those isolated build outputs. Its EFI was copied to `out/phone-boot/redox-ramboot.efi`.

```sh
cargo test --offline --manifest-path crates/ufs/Cargo.toml
scripts/emu-op6/run.sh out/phone-boot/redox-ramboot.efi --timer-ppi-fix \
  --marker 'ufsd: MMIO CAP=' \
  --marker 'ufsd: HCE enabled:' \
  --marker 'ufsd: link startup OK' \
  --marker 'ufsd: NOP OUT/IN OK' \
  --marker 'ufsd: READ CAPACITY LUN0:' \
  --marker 'ufsd: READ LBA 0..33 OK' \
  --marker 'ufsd: GPT header at LBA 1: "EFI PART"' \
  --marker 'ufsd: ESP entry' \
  --marker 'ufsd: done:' \
  --timeout 110 --run-id ufs-ro-unified-final
sha256sum out/phone-boot/redox-ramboot.efi out/emu-op6/ufs-ro-unified-final/serial.log
```

## Emulator gaps and handoff

The emulator does not model the Qualcomm vendor PHY DME attributes needed by the original vendor path. A bounded run of that path reached `ufsd` but did not produce HCE completion. The final daemon selects generic UFSHCI setup for this emulator probe. Descriptor enumeration returned `Ocs(1)` after successful NOP and `fDeviceInit`; LUN0 SCSI READ CAPACITY and READ(10) succeeded when the probe proceeded directly to them. The emulator LUN0 reports 512-byte blocks and one 64 MiB ESP copy. U-Boot on the physical OnePlus 6 reported Samsung KLUDG4U1EA, six LUNs, and 4096-byte blocks, so these emulator geometry values are not phone claims. Emulator U-Boot UFS startup was intermittent in earlier attempts; only the passing final run is used for acceptance.

There is no remaining item in the EMU-UFS acceptance scope. If this patch series is later integrated, reapply `0014` then `0015` to an isolated base checkout and rerun the host and ordered serial gates above. Stop on a failing marker, preserve that run's serial and receipt, and do not infer a phone result. Rollback is to omit `0015` (or leave this unmerged branch unused); no emulator or phone state needs restoring.
