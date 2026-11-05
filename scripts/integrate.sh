#!/usr/bin/env bash
set -euo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
fork=${REDOX_FORKS:-$HOME/Developer/forks/redox}
redox="$fork/redox"
kernel="$fork/kernel-e1"
base="$fork/base-e1"
boot="$fork/bootloader-e1"
payload="$repo/out/phone-boot"
# EMU_TOUCH=1 adds the emulator-only rmi4d/inputd-trace services
# (base init.emu-touch.d) and writes a separately named payload.
emu_touch=${EMU_TOUCH:-}
payload_name=redox-ramboot.efi
if [[ -n $emu_touch ]]; then
    payload_name=redox-ramboot-emu-touch.efi
fi
output="$repo/out/integration"
mkdir -p "$output" "$payload"

# Host-only reference model gate: keep authority/budget regressions out of the
# longer kernel, initfs, and QEMU integration build.
if ! cargo test --locked --manifest-path "$repo/tools/caps-check/Cargo.toml" \
    >"$output/caps-check-tests.log" 2>&1; then
    cat "$output/caps-check-tests.log" >&2
    exit 1
fi

# B5 target gate (Cursor audit B5-3): the sandbox crate must keep compiling for the
# real aarch64-unknown-redox target via the project's cookbook toolchain (see
# docs/GOTCHAS.md K7 note and docs/workbench/B5-REDOX-TARGET.md). This is a
# local-prefix workaround, not an upstream rustup fix.
redox_prefix="$redox/prefix/aarch64-unknown-redox"
if test -x "$redox_prefix/rust-install/bin/cargo"; then
    PATH="$redox_prefix/rust-install/bin:$redox_prefix/sysroot/bin:$PATH" \
    LD_LIBRARY_PATH="$redox_prefix/sysroot/lib" \
    cargo build --locked --offline --manifest-path "$repo/crates/sandbox/Cargo.toml" \
        --target aarch64-unknown-redox \
        >"$output/sandbox-redox-target.log" 2>&1 || {
        cat "$output/sandbox-redox-target.log" >&2
        exit 1
    }
else
    printf 'FAIL: cookbook rust-install prefix missing at %s (required for the B5 target gate)\n' "$redox_prefix" >&2
    exit 1
fi

apply_series() {
    local tree=$1 folder=$2 patch subject history
    test "$(git -C "$tree" branch --show-current)" = op6/e1-demo
    if ! compgen -G "$repo/patches/$folder/*.patch" >/dev/null; then
        printf 'Skipping absent patch series: %s\n' "$folder"
        return
    fi
    for patch in "$repo/patches/$folder"/*.patch; do
        subject=$(sed -n 's/^Subject: \[PATCH[^]]*\] //p' "$patch" | head -1)
        history=$(git -C "$tree" log --format=%s)
        if ! grep -Fq "$subject" <<<"$history"; then
            if ! git -C "$tree" am "$patch"; then
                git -C "$tree" am --abort
                printf 'FAIL: patch did not apply: %s (series %s on %s); am aborted, tree restored\n' "$patch" "$folder" "$tree" >&2
                exit 1
            fi
        fi
    done
}

if [[ ${REDOX_PATCHES_APPLIED:-0} != 1 ]]; then
apply_series "$kernel" kernel-series
apply_series "$base" base
apply_series "$base" base-spmid
apply_series "$base" integration/base
# Rebased base-simplefb + base-touch; the original touch patch duplicates
# K7's mmio-replay and conflicts with Cargo.lock when applied verbatim.
apply_series "$base" integration-2/base
apply_series "$base" base-traced
apply_series "$base" e1/base
apply_series "$base" touch-e2e/base
apply_series "$boot" bootloader
apply_series "$boot" integration-2/bootloader
apply_series "$boot" e1/bootloader
fi

# F-9 (P4-FEATURE-01): the kernel Makefile passes
# --features=$(KERNEL_CARGO_FEATURES) to cargo but does not treat the
# value as a dependency, so changing the feature set silently reused a
# stale kernel (the first p4pon60 image's kernel lacked the PON feature
# it was built for). Stamp the feature set beside the kernel tree; on
# any change, clean the kernel build before making. The stamp is
# written only after a successful kernel make.
kernel_features=${KERNEL_CARGO_FEATURES:-}
kernel_features_stamp="$kernel/.kernel-cargo-features.stamp"
if [[ -f $kernel/build/aarch64/kernel ]] && \
   [[ ! -f $kernel_features_stamp || $(<"$kernel_features_stamp") != "$kernel_features" ]]; then
    printf 'KERNEL_CARGO_FEATURES changed (%s -> %s); cleaning kernel build (F-9 stamp)\n' \
        "$(cat "$kernel_features_stamp" 2>/dev/null || echo '<none>')" "$kernel_features" >&2
    make -C "$kernel" clean >>"$output/kernel-build.log" 2>&1
fi
make -C "$kernel" ARCH=aarch64 OBJCOPY=aarch64-linux-gnu-objcopy \
    >"$output/kernel-build.log" 2>&1
printf '%s' "$kernel_features" > "$kernel_features_stamp"

podman run --rm --workdir /mnt/base-e1 --pids-limit=1024 \
    --volume "$redox":/mnt/redox \
    --volume "$redox/build/podman":/root \
    --volume "$base":/mnt/base-e1 \
    --env PATH=/root/.cargo/bin:/mnt/redox/prefix/aarch64-unknown-redox/sysroot/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
    --env TARGET=aarch64-unknown-redox \
    --env COOKBOOK_TOOLCHAIN=/mnt/redox/prefix/aarch64-unknown-redox/sysroot \
    --env COOKBOOK_SYSROOT=/mnt/redox/prefix/aarch64-unknown-redox/sysroot \
    redox-base make ramboot-initfs TARGET=aarch64-unknown-redox \
    BUILD_DIR=/mnt/base-e1/build/aarch64 LINKER=aarch64-unknown-redox-ld \
    CARGO=/mnt/redox/target/release/cookbook_redoxer \
    RAMBOOT_EMU_TOUCH="$emu_touch" \
    >"$output/base-build.log" 2>&1

for bin in geni-i2cd rmi4d inputd vesad traced; do
    test -f "$base/build/aarch64/ramboot-initfs/bin/$bin"
done
if [[ -n $emu_touch ]]; then
    test -e "$base/build/aarch64/ramboot-initfs/lib/init.d/30_rmi4d.service"
else
    test ! -e "$base/build/aarch64/ramboot-initfs/lib/init.d/30_rmi4d.service"
fi
test ! -e "$base/build/aarch64/ramboot-initfs/lib/init.d/30_geni-i2cd.service"

podman run --rm --workdir /mnt/project/apps/timeline --pids-limit=1024 \
    --volume "$redox":/mnt/redox \
    --volume "$redox/build/podman":/root \
    --volume "$repo":/mnt/project \
    --env PATH=/root/.cargo/bin:/mnt/redox/prefix/aarch64-unknown-redox/sysroot/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
    --env TARGET=aarch64-unknown-redox \
    --env COOKBOOK_TOOLCHAIN=/mnt/redox/prefix/aarch64-unknown-redox/sysroot \
    --env COOKBOOK_SYSROOT=/mnt/redox/prefix/aarch64-unknown-redox/sysroot \
    redox-base /mnt/redox/target/release/cookbook_redoxer build --release \
    --target-dir /mnt/redox/build/e1-timeline --bins >"$output/timeline-build.log" 2>&1

root="$base/build/aarch64/ramboot-initfs"
timeline_bins="$redox/build/e1-timeline/aarch64-unknown-redox/release"
for item in "$timeline_bins/timeline" "$timeline_bins/timeline-demo-publish"; do
    test -f "$item"
done
cp "$timeline_bins/timeline-demo-publish" "$root/bin/"
"$base/target/debug/redox-initfs-ar" "$root" \
    "$base/build/aarch64/aarch64-unknown-redox/release/bootstrap" \
    -o "$base/build/aarch64/ramboot-initfs.img" >"$output/initfs-pack.log" 2>&1

mkdir -p "$boot/build"
rm -f "$boot/build/bootloader-ramboot.efi"
RAMBOOT_KERNEL="$kernel/build/aarch64/kernel" \
RAMBOOT_INITFS="$base/build/aarch64/ramboot-initfs.img" \
    make -C "$boot/build" -f ../Makefile TARGET=aarch64-unknown-uefi \
    "$boot/build/bootloader-ramboot.efi" >"$output/bootloader-build.log" 2>&1

install -m 0644 "$boot/build/bootloader-ramboot.efi" "$payload/$payload_name"
digest=$(sha256sum "$payload/$payload_name" | cut -d ' ' -f1)
if [[ -n $emu_touch ]]; then
    printf '%s  %s\n' "$digest" "$payload_name" >"$payload/SHA256SUMS.emu-touch"
    (cd "$payload" && sha256sum -c SHA256SUMS.emu-touch) >"$output/sha256-check.log"
else
    printf '%s  %s\n' "$digest" "$payload_name" >"$payload/SHA256SUMS"
    (cd "$payload" && sha256sum -c SHA256SUMS) >"$output/sha256-check.log"
fi
mkdir -p "$boot/build/tftp"
cp "$payload/$payload_name" "$boot/build/tftp/bootaa64.efi"
printf 'Redox integration payload: %s\n' "$digest"

if [[ ${1:-} == --qemu ]]; then
    dtb="$output/simplefb-virt.dtb"
    qemu-system-aarch64 -machine virt,acpi=off,gic-version=3,dumpdtb="$output/virt.dtb" \
        -cpu max -smp 1 -m 2048 >/dev/null 2>&1
    dtc -@ -I dts -O dtb -o "$output/simplefb.dtbo" "$repo/docs/workbench/K8B-SIMPLEFB.dts"
    fdtoverlay -i "$output/virt.dtb" -o "$dtb" "$output/simplefb.dtbo"
    set +e
    timeout --signal=TERM 45 qemu-system-aarch64 \
        -machine virt,acpi=off,gic-version=3 -cpu max -smp 1 -m 2048 \
        -bios /usr/share/qemu-efi-aarch64/QEMU_EFI.fd -boot order=n \
        -device ramfb -dtb "$dtb" \
        -nic "user,model=virtio-net-pci,tftp=$boot/build/tftp,bootfile=bootaa64.efi" \
        -display none -serial stdio -monitor none >"$output/qemu.log" 2>&1
    result=$?
    set -e
    if [[ $result != 0 && $result != 124 ]]; then
        exit "$result"
    fi
    grep -q 'ram:~\$' "$output/qemu.log"
    grep -q 'timeline-demo-publish: sent 28 events to trace:publish' "$output/qemu.log"
    grep -q 'spmid: no SPMI arbiter in device tree; skipping' "$output/qemu.log"
    grep -q 'vesad: simplefb 800x600' "$output/qemu.log"
    # K14: the kernel boot console must render on the DT framebuffer.
    grep -q 'device tree simple-framebuffer 800x600 stride 800 at' "$output/qemu.log"
    if grep -q 'ignoring device tree simple-framebuffer' "$output/qemu.log"; then
        printf 'kernel fbcon rejected the DT framebuffer; see %s\n' "$output/qemu.log" >&2
        exit 1
    fi
    if grep -q 'vesad: failed to map framebuffer\|vesad.*exited without notifying' "$output/qemu.log"; then
        printf 'vesad did not start cleanly; see %s\n' "$output/qemu.log" >&2
        exit 1
    fi
fi
