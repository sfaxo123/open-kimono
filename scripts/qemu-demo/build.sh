#!/usr/bin/env bash
# E1 QEMU gate, step 1: build the guest and the AArch64 Redox host binary.
#
# Everything lands under this worktree's out/qemu-demo/ — never the canonical
# out/ and never the fork build trees. The fork checkout is mounted read-only.
set -euo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
out="$repo/out/qemu-demo"
redox=${REDOX_COOKBOOK:-$HOME/Developer/forks/redox/redox}
cargo_bin=${CARGO_HOME:-$HOME/.cargo}/bin

export PATH="$cargo_bin:$PATH"
mkdir -p "$out"

# 1. The guest: crates/sandbox/guests/hello-rs built for wasm32-unknown-unknown.
#    Run from the guest directory so its .cargo/config.toml (16 KiB stack) applies.
(
    cd "$repo/crates/sandbox/guests/hello-rs"
    cargo build --release --offline --target-dir "$out/wasm"
)
wasm="$out/wasm/wasm32-unknown-unknown/release/hello_rs.wasm"
test -f "$wasm"
sha=$(sha256sum "$wasm" | cut -d ' ' -f1)
printf 'hello_rs.wasm sha256 %s\n' "$sha"

# Digest gate, BEFORE anything expensive runs. The app embeds
# apps/sandbox-demo/assets/hello_rs.wasm (include_bytes!) and pins its digest in
# src/main.rs; the build above produces a SEPARATE copy. If they disagree, the
# cross-build below would hand the guest a binary that silently executes stale
# bytes while the evidence reports the fresh digest — so the gate stops here.
"$repo/scripts/qemu-demo/check-digest.sh" \
    "$wasm" "$repo/apps/sandbox-demo/assets/hello_rs.wasm" "$repo/apps/sandbox-demo/src/main.rs"
printf 'digest gate OK: fresh build, embedded asset and source pin all agree\n'

# 2. Hermetic dependency set for the cross-build (no network, no shared registry).
(
    cd "$repo/apps/sandbox-demo"
    cargo vendor --offline "$out/vendor" >"$out/vendor.log"
)

# 3. The host binary: sandbox + wasmi compiled for aarch64-unknown-redox inside
#    the cookbook redox-base container. $rustup is the container's toolchain,
#    mounted read-only; CARGO_HOME is a worktree-local copy of the shims.
if [[ ! -d "$out/cargo-home/bin" ]]; then
    mkdir -p "$out/cargo-home"
    cp -a "$redox/build/podman/.cargo/bin" "$out/cargo-home/bin"
fi
timeout 1800 podman run --rm --workdir /mnt/project/apps/sandbox-demo --pids-limit=1024 \
    --volume "$redox":/mnt/redox:ro \
    --volume "$redox/build/podman/.rustup":/root/.rustup:ro \
    --volume "$out/cargo-home":/root/.cargo \
    --volume "$repo":/mnt/project \
    --env PATH=/root/.cargo/bin:/mnt/redox/prefix/aarch64-unknown-redox/sysroot/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
    --env TARGET=aarch64-unknown-redox \
    --env COOKBOOK_TOOLCHAIN=/mnt/redox/prefix/aarch64-unknown-redox/sysroot \
    --env COOKBOOK_SYSROOT=/mnt/redox/prefix/aarch64-unknown-redox/sysroot \
    redox-base /mnt/redox/target/release/cookbook_redoxer build --release \
    --target-dir /mnt/project/out/qemu-demo/redox --bins >"$out/redox-build.log" 2>&1

binary="$out/redox/aarch64-unknown-redox/release/sandbox-demo"
test -f "$binary"
file "$binary"
printf 'sandbox-demo sha256 %s\n' "$(sha256sum "$binary" | cut -d ' ' -f1)"
