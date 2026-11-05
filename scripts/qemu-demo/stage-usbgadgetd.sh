#!/usr/bin/env bash
# V2 QEMU negative-path staging, step 1 (see docs/workbench/USBGADGETD-WIRING.md
# section 8, V2): stage the aarch64-unknown-redox usbgadgetd binary into a
# worktree-local copy of the E1 RAM-boot initfs and pack it. Follows the
# sandbox-demo precedent in stage.sh, but uses its own out dir
# (out/qemu-gadget) so the E1 qemu-demo artifacts are never clobbered.
#
# Scope, honestly stated: this script does ONLY the initfs stage + pack.
# The bootloader/ESP build is stage-usbgadgetd-boot.sh and the QEMU boot
# itself is boot-usbgadgetd.sh (stage.sh steps 3-4 adapted to
# out/qemu-gadget); neither is done here.
#
# Expected guest behaviour once booted: usbgadgetd runs as a oneshot;
# payload v2 wired its Redox main, so in QEMU it maps the SMMU window,
# runs the log-only probe (verified NotMatched -> DMA refused), exits 3
# cleanly, and the debug ramsh (30_ramsh.service, already in the
# target) still reaches ram:~$. That failure path is what V2 proves -
# not enumeration. (Pre-payload-v2 binaries took the model-windows
# path: UnsupportedCore(0), exit 2; the validator accepts both.)
set -euo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
out="$repo/out/qemu-gadget"
forks=${REDOX_FORKS:-$HOME/Developer/forks/redox}
base="$forks/base-e1"

binary="$repo/crates/usbgadgetd/target/aarch64-unknown-redox/debug/usbgadgetd"
test -f "$binary" || { printf 'missing %s - build the redox target first (G-W1 recipe)\n' "$binary" >&2; exit 1; }
# Refuse to stage a wrong-architecture binary: the guest is aarch64.
file_out=$(file "$binary")
case "$file_out" in
  *"ARM aarch64"*) ;;
  *) printf 'binary is not aarch64: %s\n' "$file_out" >&2; exit 1 ;;
esac
src_sha=$(sha256sum "$binary" | cut -d ' ' -f1)
printf 'staging binary sha256 %s\n' "$src_sha"

mkdir -p "$out"
root="$out/ramboot-initfs"
rm -rf "$root"
cp -a "$base/build/aarch64/ramboot-initfs" "$root"
cp "$binary" "$root/bin/usbgadgetd"
chmod 755 "$root/bin/usbgadgetd"
cat >"$root/lib/init.d/46_usbgadgetd.service" <<'UNIT'
[unit]
description = "V2 negative-path probe (payload v2): usbgadgetd runs its log-only SMMU probe, verifies NotMatched, refuses DMA, and fails cleanly (exit 3) with no SMMU/DWC3 in QEMU"
requires_weak = ["00_traced.service"]

[service]
cmd = "usbgadgetd"
type = "oneshot"
UNIT
sed -i 's/"40_timeline_demo.service"]/"40_timeline_demo.service", "46_usbgadgetd.service"]/' \
    "$root/lib/init.d/90_ramboot.target"
# Verify what the final printf claims, in-script (Wonder diff review of
# 1914fdc, CORRECT BEFORE BUILDING ON IT): the staged copy must hash
# identically to the source binary, the service file must read back with
# its exact cmd/type/requires_weak lines, and the target must name it.
staged_sha=$(sha256sum "$root/bin/usbgadgetd" | cut -d ' ' -f1)
test "$staged_sha" = "$src_sha" || { printf 'staged binary sha256 %s != source %s\n' "$staged_sha" "$src_sha" >&2; exit 1; }
grep -Fqx 'cmd = "usbgadgetd"' "$root/lib/init.d/46_usbgadgetd.service"
grep -Fqx 'type = "oneshot"' "$root/lib/init.d/46_usbgadgetd.service"
grep -Fqx 'requires_weak = ["00_traced.service"]' "$root/lib/init.d/46_usbgadgetd.service"
grep -q '46_usbgadgetd.service' "$root/lib/init.d/90_ramboot.target"
printf 'staged binary sha256 %s (identical to source)\n' "$staged_sha"

"$base/target/debug/redox-initfs-ar" "$root" \
    "$base/build/aarch64/aarch64-unknown-redox/release/bootstrap" \
    -o "$out/ramboot-initfs.img" >"$out/initfs-pack.log" 2>&1
printf 'ramboot-initfs.img sha256 %s\n' "$(sha256sum "$out/ramboot-initfs.img" | cut -d ' ' -f1)"
printf 'staged tree: %s (binary + 46_usbgadgetd.service + target entry verified)\n' "$root"
