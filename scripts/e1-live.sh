#!/usr/bin/env bash
set -euo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
fork=${REDOX_FORKS:-$HOME/Developer/forks/redox}
redox="$fork/redox"
base="$fork/base-e1"
kernel="$fork/kernel-e1"
boot="$fork/bootloader-e1"
build="$redox/build/aarch64/desktop-minimal"
source_image="$build/redox-live.iso"
image="$build/e1-live.iso"
mount="$build/e1-mount"
stage="$build/e1-staging"
timeline="$redox/build/e1-timeline/aarch64-unknown-redox/release"
output="$repo/out/integration"
mkdir -p "$output"

for tree in "$base" "$kernel" "$boot"; do
    test "$(git -C "$tree" branch --show-current)" = op6/e1-demo
done
test -f "$source_image"
test -f "$timeline/timeline"
test -f "$timeline/timeline-demo-publish"
test -f "$redox/recipes/core/redoxfs/target/aarch64-unknown-redox/stage/usr/bin/redoxfs"

podman run --rm --workdir /mnt/base-e1 --pids-limit=1024 \
    --volume "$redox":/mnt/redox \
    --volume "$redox/build/podman":/root \
    --volume "$base":/mnt/base-e1 \
    --env PATH=/root/.cargo/bin:/mnt/redox/prefix/aarch64-unknown-redox/sysroot/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
    --env TARGET=aarch64-unknown-redox \
    --env COOKBOOK_TOOLCHAIN=/mnt/redox/prefix/aarch64-unknown-redox/sysroot \
    --env COOKBOOK_SYSROOT=/mnt/redox/prefix/aarch64-unknown-redox/sysroot \
    redox-base make install-base TARGET=aarch64-unknown-redox \
    BUILD_DIR=/mnt/base-e1/build/aarch64 LINKER=aarch64-unknown-redox-ld \
    CARGO=/mnt/redox/target/release/cookbook_redoxer \
    SYSROOT=/mnt/redox/recipes/core/redoxfs/target/aarch64-unknown-redox/stage/usr \
    DESTDIR=/mnt/redox/build/aarch64/desktop-minimal/e1-staging \
    >"$output/e1-base-live-build.log" 2>&1

make -C "$boot/build" -f ../Makefile TARGET=aarch64-unknown-uefi \
    "$boot/build/bootloader-live.efi" >"$output/e1-live-bootloader-build.log" 2>&1
mkdir -p "$boot/build/tftp-live" "$mount"
cp "$boot/build/bootloader-live.efi" "$boot/build/tftp-live/bootaa64.efi"
cp --reflink=auto "$source_image" "$image"

fuse_pid=
unmount_image() {
    if mountpoint -q "$mount"; then
        fusermount3 -u "$mount"
    fi
    if [[ -n "$fuse_pid" ]]; then
        wait "$fuse_pid"
        fuse_pid=
    fi
}
trap unmount_image EXIT
mount_image() {
    "$redox/build/fstools/bin/redoxfs" --no-daemon "$image" "$mount" \
        >"$output/e1-redoxfs.log" 2>&1 &
    fuse_pid=$!
    for _ in {1..50}; do
        mountpoint -q "$mount" && return
        sleep 0.1
    done
    echo "RedoxFS did not mount $image" >&2
    exit 1
}

mount_image
cp "$stage/usr/lib/boot/initfs" "$mount/usr/lib/boot/initfs"
cp "$kernel/build/aarch64/kernel" "$mount/usr/lib/boot/kernel"
cp "$kernel/build/aarch64/kernel.all" "$mount/usr/lib/boot/kernel.all"
cp "$timeline/timeline" "$timeline/timeline-demo-publish" "$mount/usr/bin/"
cat >"$mount/usr/lib/init.d/20_orbital" <<'EOF'
requires_weak 10_net.target 20_audiod.service
timeline-demo-publish
nowait VT=3 orbital timeline --stream trace:subscribe/{} --width 760 --height 540
EOF
sync
cmp "$kernel/build/aarch64/kernel" "$mount/usr/lib/boot/kernel"
cmp "$timeline/timeline" "$mount/usr/bin/timeline"
unmount_image

# A FUSE write is not evidence until it survives closing and reopening RedoxFS.
mount_image
cmp "$kernel/build/aarch64/kernel" "$mount/usr/lib/boot/kernel"
cmp "$stage/usr/lib/boot/initfs" "$mount/usr/lib/boot/initfs"
cmp "$timeline/timeline" "$mount/usr/bin/timeline"
cmp "$timeline/timeline-demo-publish" "$mount/usr/bin/timeline-demo-publish"
grep -q '^nowait VT=3 orbital timeline ' "$mount/usr/lib/init.d/20_orbital"
unmount_image
sha256sum "$image" "$boot/build/bootloader-live.efi" >"$output/e1-live-sha256.txt"
