#!/usr/bin/env bash
# V2 QEMU negative-path staging, step 2 (see docs/workbench/USBGADGETD-WIRING.md
# section 8, V2): build the RAM-boot bootloader that embeds the qemu-gadget
# initfs packed by stage-usbgadgetd.sh (step 1), and build the FAT ESP that
# carries it. Steps 3-4 of scripts/qemu-demo/stage.sh, adapted to
# out/qemu-gadget so the E1 qemu-demo artifacts are never clobbered.
#
# Scope, honestly stated: this script does ONLY the bootloader build + ESP.
# The QEMU boot itself is boot-usbgadgetd.sh (step 3).
set -euo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
out="$repo/out/qemu-gadget"
forks=${REDOX_FORKS:-$HOME/Developer/forks/redox}
kernel="$forks/kernel-e1"
boot_src="$forks/bootloader-e1"

export PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"

test -f "$out/ramboot-initfs.img" || { printf 'missing %s - run scripts/qemu-demo/stage-usbgadgetd.sh first\n' "$out/ramboot-initfs.img" >&2; exit 1; }
test -f "$kernel/build/aarch64/kernel" || { printf 'missing E1 kernel %s\n' "$kernel/build/aarch64/kernel" >&2; exit 1; }

# 3. Worktree-local copy of the E1 bootloader, building the RAM-boot EFI that
#    embeds the fork kernel and the qemu-gadget initfs image.
rsync -a --exclude=build --exclude=target "$boot_src/" "$out/bootloader-e1/"
mkdir -p "$out/bootloader-e1/build"
rm -f "$out/bootloader-e1/build/bootloader-ramboot.efi"
timeout 1800 env RAMBOOT_KERNEL="$kernel/build/aarch64/kernel" \
    RAMBOOT_INITFS="$out/ramboot-initfs.img" \
    make -C "$out/bootloader-e1/build" -f ../Makefile TARGET=aarch64-unknown-uefi \
    "$out/bootloader-e1/build/bootloader-ramboot.efi" >"$out/bootloader-build.log" 2>&1
efi="$out/bootloader-e1/build/bootloader-ramboot.efi"
test -f "$efi" || { printf 'bootloader build produced no EFI (see %s)\n' "$out/bootloader-build.log" >&2; exit 1; }
efi_sha=$(sha256sum "$efi" | cut -d ' ' -f1)
printf 'bootloader-ramboot.efi sha256 %s\n' "$efi_sha"
# Verify what the final printf claims, in-script (Wonder diff review of
# c959a28, CORRECT BEFORE BUILDING ON IT): the EFI must be an Aarch64
# PE32+ application, and it must embed THIS run's qemu-gadget initfs
# verbatim (the RAMBOOT_INITFS build embeds it uncompressed - proven on
# the accepted V2 artifacts, where the initfs is a byte substring of
# the EFI). A build that silently embedded a stale initfs fails here.
file "$efi" | grep -q 'PE32+ executable (EFI application) Aarch64' \
    || { printf 'EFI is not an Aarch64 PE32+ application: %s\n' "$(file "$efi")" >&2; exit 1; }
initfs_sha=$(sha256sum "$out/ramboot-initfs.img" | cut -d ' ' -f1)
python3 - "$efi" "$out/ramboot-initfs.img" <<'PYEOF' \
    || { printf 'EFI does not embed %s verbatim\n' "$out/ramboot-initfs.img" >&2; exit 1; }
import sys
efi = open(sys.argv[1], "rb").read()
initfs = open(sys.argv[2], "rb").read()
sys.exit(0 if initfs in efi else 1)
PYEOF
printf 'EFI verified: Aarch64 PE32+; embeds ramboot-initfs.img %s verbatim\n' "$initfs_sha"

# 4. FAT ESP on virtio-blk: the firmware path that needs no network device.
#    mtools is not installed here, so the ESP is populated through a loop mount.
esp="$out/esp.img"
rm -f "$esp"
truncate -s 64M "$esp"
parted -s "$esp" mklabel gpt
parted -s "$esp" mkpart ESP fat32 1MiB 63MiB
parted -s "$esp" set 1 esp on
sudo -n true || { printf 'passwordless sudo is required to populate the ESP\n' >&2; exit 1; }
mkdir -p "$out/esp-mount"
loop=$(sudo -n losetup -P -f --show "$esp")
cleanup() { sudo -n umount "$out/esp-mount" 2>/dev/null || true; sudo -n losetup -d "$loop" 2>/dev/null || true; }
trap cleanup EXIT
sudo -n mkfs.vfat -F 32 -n REDOXESP "${loop}p1" >"$out/esp-mkfs.log"
sudo -n mount "${loop}p1" "$out/esp-mount"
sudo -n mkdir -p "$out/esp-mount/EFI/BOOT"
sudo -n cp "$efi" "$out/esp-mount/EFI/BOOT/BOOTAA64.EFI"
sync
# Read the ESP-resident copy back BEFORE unmounting and require byte
# equality with the built EFI (Wonder diff review of c959a28: the old
# final printf claimed "verified" after only test -f on the standalone
# EFI and self-hashes - a corrupted ESP copy still printed verified).
esp_efi_sha=$(sudo -n sha256sum "$out/esp-mount/EFI/BOOT/BOOTAA64.EFI" | cut -d ' ' -f1)
test "$esp_efi_sha" = "$efi_sha" \
    || { printf 'ESP BOOTAA64.EFI sha256 %s != built EFI %s\n' "$esp_efi_sha" "$efi_sha" >&2; exit 1; }
printf 'ESP BOOTAA64.EFI sha256 %s (identical to built EFI)\n' "$esp_efi_sha"
cleanup
trap - EXIT
esp_sha=$(sha256sum "$esp" | cut -d ' ' -f1)
printf 'esp.img sha256 %s\n' "$esp_sha"
# Chain record for boot-usbgadgetd.sh's preflight: binds this exact
# esp.img to the exact EFI read back from it and the exact initfs the
# EFI was proven to embed, so a stale ESP cannot pass a later boot.
printf 'efi_sha256 %s\ninitfs_sha256 %s\nesp_sha256 %s\n' \
    "$efi_sha" "$initfs_sha" "$esp_sha" > "$out/esp-chain.txt"
printf 'staged boot artifacts verified: %s (ESP-resident EFI == built EFI %s; EFI embeds initfs %s; chain record in esp-chain.txt)\n' "$out" "$efi_sha" "$initfs_sha"
