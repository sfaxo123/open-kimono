#!/usr/bin/env bash
# Digest gate for the E1 QEMU sandbox gate.
#
# apps/sandbox-demo/src/main.rs embeds the guest with
#   include_bytes!("../assets/hello_rs.wasm")
# and pins its identity with
#   const HELLO_RS_SHA256: &str = "<sha256>";
# build.sh separately rebuilds the guest from crates/sandbox/guests/hello-rs. If
# those two disagree, the cross-built binary silently executes STALE guest bytes
# while the evidence document reports the digest of the fresh build. This gate runs
# before the expensive podman cross-build so a mismatch cannot produce a binary at
# all, and it is what makes "never silently execute a stale asset" enforceable.
#
# Identity here means the FILE, not the filename: the include_bytes! path is
# resolved relative to the source file exactly as rustc resolves it, and must name
# the very asset passed in. `../other/hello_rs.wasm` is not `assets/hello_rs.wasm`
# no matter what the two digests say.
#
# usage: check-digest.sh <fresh-wasm> <embedded-asset> <main-rs>
set -euo pipefail

fresh=${1:-}; asset=${2:-}; mainsrc=${3:-}
[[ -n "$fresh" && -n "$asset" && -n "$mainsrc" ]] || {
    printf 'usage: check-digest.sh <fresh-wasm> <embedded-asset> <main-rs>\n' >&2; exit 1; }
for f in "$fresh" "$asset" "$mainsrc"; do
    test -f "$f" || { printf 'check-digest: missing %s\n' "$f" >&2; exit 1; }
done

sha_of() { sha256sum "$1" | cut -d ' ' -f1; }
fresh_sha=$(sha_of "$fresh"); asset_sha=$(sha_of "$asset")

pin=$(sed -n 's/.*HELLO_RS_SHA256: &str = "\([0-9a-fA-F]\{64\}\)".*/\1/p' "$mainsrc" | head -1)
[[ -n "$pin" ]] || { printf 'check-digest: no HELLO_RS_SHA256 pin found in %s\n' "$mainsrc" >&2; exit 1; }

embedded=$(sed -n 's/.*include_bytes!("\([^"]*\)").*/\1/p' "$mainsrc" | head -1)
[[ -n "$embedded" ]] || {
    printf 'check-digest: FAIL %s has no include_bytes!("…") — there is nothing to attest to\n' "$mainsrc" >&2
    exit 1; }

# rustc resolves include_bytes! RELATIVE TO THE .rs FILE that contains it, so the
# gate must resolve it the same way. Comparing basenames — as this used to — lets
# `include_bytes!("../other/hello_rs.wasm")` pass while the gate hashes
# `assets/hello_rs.wasm`: fresh, asset and pin all agree, and every digest printed
# below describes a file the binary never embeds.
src_dir=$(cd "$(dirname "$mainsrc")" && pwd -P)
if [[ "$embedded" == /* ]]; then inc=$embedded; else inc="$src_dir/$embedded"; fi
inc_abs=$(realpath -m -- "$inc")
asset_abs=$(realpath -- "$asset")
if [[ ! -f "$inc_abs" ]]; then
    printf 'check-digest: FAIL %s embeds %s, which resolves to %s — no such file\n' \
        "$mainsrc" "$embedded" "$inc_abs" >&2
    exit 1
fi
if [[ "$inc_abs" != "$asset_abs" ]]; then
    printf 'check-digest: FAIL %s embeds %s (%s), not the asset being checked (%s).\n' \
        "$mainsrc" "$embedded" "$inc_abs" "$asset_abs" >&2
    printf '  Same basename, different file. The digests below would describe bytes the\n' >&2
    printf '  binary never contains. Check the file the source actually embeds, or fix the\n' >&2
    printf '  include path — a basename match is not an identity.\n' >&2
    exit 1
fi

printf '  fresh guest   %s  %s\n' "$fresh_sha" "$fresh"
printf '  embedded asset %s  %s\n' "$asset_sha" "$asset"
printf '  include path  %s  (how rustc resolves it, relative to %s)\n' "$inc_abs" "$mainsrc"
printf '  pin in source  %s  %s\n' "$pin" "$mainsrc"

fail=0
if [[ "$asset_sha" != "$pin" ]]; then
    printf 'check-digest: FAIL the committed asset does not match the pin in %s.\n' "$mainsrc" >&2
    printf '  The binary would execute bytes that differ from the pinned identity. Update the\n' >&2
    printf '  asset or the pin deliberately — never let them drift silently.\n' >&2
    fail=1
fi
if [[ "$fresh_sha" != "$asset_sha" ]]; then
    printf 'check-digest: FAIL the freshly built guest differs from the embedded asset.\n' >&2
    printf '  fresh %s\n  asset %s\n' "$fresh_sha" "$asset_sha" >&2
    printf '  The app embeds the asset, not the fresh build: cross-building now would run a\n' >&2
    printf '  STALE guest while the evidence reports the fresh digest. Copy the fresh wasm over\n' >&2
    printf '  the asset and update the pin, or the gate refuses to build.\n' >&2
    fail=1
fi
exit "$fail"
