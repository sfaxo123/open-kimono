#!/usr/bin/env bash
# E1 QEMU gate, step 2: stage the sandbox-demo binary into a worktree-local copy
# of the E1 RAM-boot initfs, pack it, and build the RAM-boot bootloader that
# embeds it. Fork trees are read inputs only; every output is under out/.
set -euo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
out="$repo/out/qemu-demo"
forks=${REDOX_FORKS:-$HOME/Developer/forks/redox}
base="$forks/base-e1"
kernel="$forks/kernel-e1"
boot_src="$forks/bootloader-e1"

export PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"

binary="$out/redox/aarch64-unknown-redox/release/sandbox-demo"
test -f "$binary" || { printf 'missing %s — run scripts/qemu-demo/build.sh first\n' "$binary" >&2; exit 1; }

# 1. Worktree-local copy of the E1 RAM-boot initfs, plus the demo unit.
root="$out/ramboot-initfs"
rm -rf "$root"
cp -a "$base/build/aarch64/ramboot-initfs" "$root"
cp "$binary" "$root/bin/sandbox-demo"
chmod 755 "$root/bin/sandbox-demo"
cat >"$root/lib/init.d/45_sandbox_demo.service" <<'UNIT'
[unit]
description = "Run the E1 sandbox hello + TraceEmit-refusal gate"
requires_weak = ["00_traced.service"]

[service]
cmd = "sandbox-demo"
type = "oneshot"
UNIT
sed -i 's/"40_timeline_demo.service"\]/"40_timeline_demo.service", "45_sandbox_demo.service"]/' \
    "$root/lib/init.d/90_ramboot.target"
grep -q '45_sandbox_demo.service' "$root/lib/init.d/90_ramboot.target"

# 2. Pack it with the E1 bootstrap (init + initfs in one image).
"$base/target/debug/redox-initfs-ar" "$root" \
    "$base/build/aarch64/aarch64-unknown-redox/release/bootstrap" \
    -o "$out/ramboot-initfs.img" >"$out/initfs-pack.log" 2>&1
printf 'ramboot-initfs.img sha256 %s\n' "$(sha256sum "$out/ramboot-initfs.img" | cut -d ' ' -f1)"

# 3. Worktree-local copy of the E1 bootloader, building the RAM-boot EFI that
#    embeds the fork kernel and our initfs image.
rsync -a --exclude=build --exclude=target "$boot_src/" "$out/bootloader-e1/"
mkdir -p "$out/bootloader-e1/build"
rm -f "$out/bootloader-e1/build/bootloader-ramboot.efi"
timeout 1800 env RAMBOOT_KERNEL="$kernel/build/aarch64/kernel" \
    RAMBOOT_INITFS="$out/ramboot-initfs.img" \
    make -C "$out/bootloader-e1/build" -f ../Makefile TARGET=aarch64-unknown-uefi \
    "$out/bootloader-e1/build/bootloader-ramboot.efi" >"$out/bootloader-build.log" 2>&1
efi="$out/bootloader-e1/build/bootloader-ramboot.efi"
printf 'bootloader-ramboot.efi sha256 %s\n' "$(sha256sum "$efi" | cut -d ' ' -f1)"

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
sudo -n cp "$out/bootloader-e1/build/bootloader-ramboot.efi" "$out/esp-mount/EFI/BOOT/BOOTAA64.EFI"
sync
cleanup
trap - EXIT
printf 'esp.img sha256 %s\n' "$(sha256sum "$esp" | cut -d ' ' -f1)"
