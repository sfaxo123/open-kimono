#!/bin/sh
set -eu
umask 077
out=${1:?Usage: sudo sh capture-recovery.sh NEW_EMPTY_OUTPUT_DIRECTORY}
case "$out" in /dev/*|/proc/*|/sys/*) echo "Invalid output directory" >&2; exit 2;; esac
[ ! -e "$out" ] || { echo "Output already exists" >&2; exit 2; }
mkdir -p "$out"
for label in boot_a boot_b dtbo_a dtbo_b vbmeta_a vbmeta_b persist modemst1 modemst2 fsg fsc devinfo config frp param oem_dycnvbk oem_stanvbk cdt ddr sec xbl_a xbl_b xbl_config_a xbl_config_b abl_a abl_b; do
  test ! -e "$out/$label.img"
  dd if="/dev/disk/by-partlabel/$label" of="$out/$label.img" bs=1048576 2>"$out/$label.dd.log"
done
for disk in sda sdb sdc sdd sde sdf; do
  sectors=$(cat "/sys/block/$disk/size")
  dd if="/dev/$disk" of="$out/$disk-first-256K.bin" bs=512 count=512 2>/dev/null
  dd if="/dev/$disk" of="$out/$disk-last-256K.bin" bs=512 skip=$((sectors-512)) count=512 2>/dev/null
done
lsblk -o NAME,SIZE,TYPE,FSTYPE,MOUNTPOINTS > "$out/lsblk.txt"
ls -l /dev/disk/by-partlabel > "$out/partition-labels.txt"
cat /proc/sys/kernel/random/boot_id > "$out/boot-id.txt"
tar -C /boot -cf "$out/current-boot-files.tar" .
cd "$out"
sha256sum *.img *.bin *.tar > SHA256SUMS
chmod 600 ./*
printf 'RECOVERY-CAPTURE-COMPLETE\n'
