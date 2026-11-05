#!/usr/bin/env bash
# E1 QEMU gate, step 4: turn the run artifacts into the evidence document.
#
# Two rules this script enforces, because both were violated once already:
#
# 1. Nothing is written unless the strict validator accepts the capture. A
#    non-empty serial is not evidence of success — a capture that ends in
#    `sandbox-demo: FAIL` is also non-empty.
# 2. Every run is recorded as an immutable receipt. Builds here are NOT bit-
#    reproducible (the same source re-staged produces different initfs/ESP/DTB
#    hashes), so hashes are per-run receipts, never identity proofs. A rerun must
#    not overwrite the artifacts backing an earlier document: each run gets its
#    own run-id under docs/workbench/evidence/, and the run ledger is append-only.
# 3. An artifact digest is only ever recorded for the run that produced it. The
#    worktree's current out/qemu-demo/ belongs to the CURRENT capture. Documenting
#    an archived --serial capture therefore requires a paired manifest that
#    re-verifies, or the artifacts are recorded as UNKNOWN — never as today's build.
# 4. A manifest row is bound to a role by its exact resolved PATH, never by
#    basename. `other/esp.img` is not `esp.img`, and a row that happens to share a
#    filename with the capture is not the capture.
set -euo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
cd "$repo"
out="$repo/out/qemu-demo"
doc="$repo/docs/workbench/QEMU-SANDBOX-DEMO.txt"
evidence_dir="$repo/docs/workbench/evidence"
ledger="$repo/docs/workbench/QEMU-SANDBOX-DEMO-runs.tsv"
validate="$repo/scripts/qemu-demo/validate.sh"

# 0. Arguments are parsed BEFORE any capture path is looked at. `--serial` points
#    at an ARCHIVED capture, and an archive must be documentable in a worktree that
#    has no current run at all — checking the default path first made that illegal.
serial=''
manifest=${MANIFEST:-}
manifest_dir=''
# Which archived file answers for which artifact. These are the canonical names of
# the archived layout: paths relative to the manifest's own directory. An archive
# that stores an artifact elsewhere says so explicitly with --map; nothing is ever
# matched by filename alone.
declare -A art_path=(
    [wasm]=hello_rs.wasm
    [binary]=sandbox-demo
    [initfs]=ramboot-initfs.img
    [efi]=bootloader-ramboot.efi
    [esp]=esp.img
    [dtb]=simplefb-virt.dtb
)
while [[ $# -gt 0 ]]; do
    case "$1" in
        --serial)   serial=${2:-}; shift 2 ;;
        --manifest) manifest=${2:-}; shift 2 ;;
        --map)
            mapping=${2:-}; shift 2
            role=${mapping%%=*}; mapped=${mapping#*=}
            if [[ -z "$role" || -z "$mapped" || "$role" == "$mapping" || ! ${art_path[$role]+isset} ]]; then
                printf 'evidence.sh: --map wants <role>=<path relative to the manifest>; roles: %s\n' \
                    "${!art_path[*]}" >&2
                exit 1
            fi
            art_path[$role]=$mapped ;;
        -h|--help)
            printf 'usage: evidence.sh [--serial <capture>] [--manifest <SHA256SUMS>] [--map <role>=<path>]\n'
            exit 0 ;;
        *) printf 'evidence.sh: unknown argument %s\n' "$1" >&2; exit 1 ;;
    esac
done
[[ -n "$serial" ]] || serial="$out/serial.raw"
test -s "$serial" || { printf 'no serial capture at %s — run boot.sh first\n' "$serial" >&2; exit 1; }

# 1. Validate before anything is replaced. A non-empty capture is not acceptance.
"$validate" "$serial" || { printf 'refusing to write evidence: the capture fails validation\n' >&2; exit 1; }

strip=$(mktemp)
trap 'rm -f "$strip"' EXIT
sed 's/\x1b\[[0-9;]*[a-zA-Z]//g; s/\x1b\[=[0-9;]*[a-zA-Z]//g; s/\x1b[()][A-Z0-9]//g; s/\r//g' \
    "$serial" > "$strip"

# 2. Identity of THIS run: the capture's own modification time plus its digest.
#    A rerun produces a different serial, hence a different run-id and its own
#    receipts; the receipts backing an earlier document are never touched.
serial_sha=$(sha256sum "$serial" | cut -d ' ' -f1)
captured_utc=$(date -u -r "$serial" '+%Y-%m-%dT%H:%M:%SZ')
captured_pt=$(TZ=America/Los_Angeles date -r "$serial" '+%Y-%m-%d %H:%M')
run_id="$(date -u -r "$serial" '+%Y%m%dT%H%M%SZ')-${serial_sha:0:12}"
recorded_utc=$(date -u '+%Y-%m-%dT%H:%M:%SZ')
recorded_pt=$(TZ=America/Los_Angeles date '+%Y-%m-%d %H:%M')

hash_or_na() { if [[ -f "$1" ]]; then sha256sum "$1" | cut -d ' ' -f1; else printf 'NOT-PRESENT\n'; fi; }

# 2b. Which build produced THIS capture?
#
# The worktree's current out/qemu-demo/ artifacts belong to the current run. When
# the capture IS this run's serial.raw, that attribution is honest. When the capture
# is an archive, the worktree may hold a completely different build — hashing it and
# printing the digests under the archived run's id would fabricate evidence. So an
# archived capture may only cite artifacts a manifest vouches for, and the manifest
# itself must re-verify line by line and name this very capture. Anything left over
# is recorded as UNKNOWN, with the reason.
row_abs() { # <manifest dir> <path as listed in a manifest row> -> canonical absolute path
    if [[ "$2" == /* ]]; then realpath -m -- "$2"; else realpath -m -- "$1/$2"; fi
}

verify_manifest() { # <manifest> <capture> <capture-sha256>
    local m=$1 s=$2 ssha=$3 dir cap_abs line row_sha row_path f vouched=0
    dir=$(cd "$(dirname "$m")" && pwd -P)
    cap_abs=$(realpath -- "$s")
    while IFS= read -r line || [[ -n "$line" ]]; do
        if [[ -z "$line" ]]; then continue; fi
        row_sha=${line%% *}
        row_path=${line#* }
        row_path=${row_path# }
        row_path=${row_path#\*}
        if [[ ${#row_sha} != 64 ]]; then
            printf 'manifest %s: malformed line: %s\n' "$m" "$line" >&2; return 1
        fi
        f=$(row_abs "$dir" "$row_path")
        if [[ ! -f "$f" ]]; then
            printf 'manifest %s: lists %s (%s), which is not present\n' "$m" "$row_path" "$f" >&2; return 1
        fi
        if [[ "$(sha256sum "$f" | cut -d ' ' -f1)" != "$row_sha" ]]; then
            printf 'manifest %s: %s does not match its recorded digest\n' "$m" "$row_path" >&2
            return 1
        fi
        # Pairing binds the manifest to the capture by PATH, not by filename. A row
        # must resolve to this very capture file; a same-named copy elsewhere in the
        # archive is a different file and vouches for nothing.
        if [[ "$f" == "$cap_abs" ]]; then
            if [[ "$row_sha" == "$ssha" ]]; then vouched=1
            else
                printf 'manifest %s lists %s with a digest that is not this capture\n' "$m" "$row_path" >&2
                return 1
            fi
        fi
    done < "$m"
    if [[ "$vouched" != 1 ]]; then
        printf 'manifest %s has no row resolving to this capture (%s) — it is not paired with it\n' \
            "$m" "$cap_abs" >&2
        return 1
    fi
    return 0
}

manifest_sha_at() { # <exact path relative to the manifest dir> -> its digest, else empty
    local want line row_sha row_path
    want=$(row_abs "$manifest_dir" "$1")
    while IFS= read -r line || [[ -n "$line" ]]; do
        [[ -n "$line" ]] || continue
        row_sha=${line%% *}; row_path=${line#* }; row_path=${row_path# }; row_path=${row_path#\*}
        if [[ "$(row_abs "$manifest_dir" "$row_path")" == "$want" ]]; then printf '%s\n' "$row_sha"; return 0; fi
    done < "$manifest"
    return 0
}

current_run=0
if [[ "$(realpath -m -- "$serial")" == "$(realpath -m -- "$out/serial.raw")" ]]; then current_run=1; fi
if [[ "$current_run" == 1 ]]; then
    provenance="current worktree artifacts — this capture is this run's out/qemu-demo/serial.raw"
else
    if [[ -z "$manifest" ]]; then
        cdir=$(cd "$(dirname "$(realpath -- "$serial")")" && pwd -P)
        if [[ -f "$cdir/SHA256SUMS" ]]; then manifest="$cdir/SHA256SUMS"; fi
    fi
    if [[ -n "$manifest" ]]; then
        manifest_dir=$(cd "$(dirname "$manifest")" && pwd -P)
        verify_manifest "$manifest" "$serial" "$serial_sha" || {
            printf 'refusing to write evidence: the manifest paired with this capture does not verify\n' >&2
            exit 1; }
        provenance="manifest-verified: every row of $manifest was re-hashed and matched, and a row resolves to this capture at $manifest_dir"
    else
        provenance='UNKNOWN — archived capture with no paired manifest; the current out/qemu-demo/ build is a different run and is NOT attributed to it'
    fi
fi

artifact() { # <role> <path in the current worktree>
    local h
    if [[ "$current_run" == 1 ]]; then
        hash_or_na "$2"
    elif [[ -n "$manifest" ]]; then
        h=$(manifest_sha_at "${art_path[$1]}")
        if [[ -n "$h" ]]; then printf '%s\n' "$h"
        else printf 'UNKNOWN (%s is not listed under that exact path in %s)\n' \
            "${art_path[$1]}" "$(basename "$manifest")"; fi
    else
        printf 'UNKNOWN (no manifest paired with this capture)\n'
    fi
}

wasm_sha=$(artifact wasm "$out/wasm/wasm32-unknown-unknown/release/hello_rs.wasm")
# The asset and the pin live in the repo, not in out/, so they describe the tree
# this document was generated from — stated as such rather than claimed for the run.
asset_sha=$(sha256sum "$repo/apps/sandbox-demo/assets/hello_rs.wasm" | cut -d ' ' -f1)
pin_sha=$(sed -n 's/.*HELLO_RS_SHA256: &str = "\([0-9a-fA-F]\{64\}\)".*/\1/p' "$repo/apps/sandbox-demo/src/main.rs" | head -1)
binary_sha=$(artifact binary "$out/redox/aarch64-unknown-redox/release/sandbox-demo")
initfs_sha=$(artifact initfs "$out/ramboot-initfs.img")
efi_sha=$(artifact efi "$out/bootloader-e1/build/bootloader-ramboot.efi")
esp_sha=$(artifact esp "$out/esp.img")
dtb_sha=$(artifact dtb "$out/simplefb-virt.dtb")
branch=$(git rev-parse --abbrev-ref HEAD 2>/dev/null || printf 'no-commit')

# 3. Immutable receipts. The same capture never overwrites an existing run; a
#    different capture is a different run-id by construction.
mkdir -p "$evidence_dir"
receipt="$evidence_dir/$run_id-receipt.tsv"
run_serial="$evidence_dir/$run_id-serial.txt"
if [[ -e "$receipt" || -e "$run_serial" ]]; then
    printf 'refusing to overwrite the existing receipts for run %s\n' "$run_id" >&2
    exit 1
fi
cp "$strip" "$run_serial"
{
    printf 'run_id\t%s\n' "$run_id"
    printf 'captured_utc\t%s\n' "$captured_utc"
    printf 'captured_pt\t%s\n' "$captured_pt"
    printf 'receipt_written_utc\t%s\n' "$recorded_utc"
    printf 'capture_path\t%s\n' "${serial#"$repo"/}"
    printf 'branch\t%s\n' "$branch"
    printf 'validator\tACCEPT (%s)\n' "$validate"
    printf 'serial.raw\t%s\n' "$serial_sha"
    printf 'fresh_wasm\t%s\n' "$wasm_sha"
    printf 'embedded_asset\t%s\n' "$asset_sha"
    printf 'source_pin\t%s\n' "$pin_sha"
    printf 'sandbox_demo_binary\t%s\n' "$binary_sha"
    printf 'ramboot_initfs\t%s\n' "$initfs_sha"
    printf 'bootloader_efi\t%s\n' "$efi_sha"
    printf 'esp_img\t%s\n' "$esp_sha"
    printf 'simplefb_virt_dtb\t%s\n' "$dtb_sha"
    printf 'host\t%s %s\n' "$(uname -sr)" "$(qemu-system-aarch64 --version | head -1)"
    printf 'artifact_provenance\t%s\n' "$provenance"
    printf 'asset_and_pin_source\tembedded_asset and source_pin are read from the current repo tree (branch %s); they describe the source, not this capture\n' "$branch"
    if [[ -n "${ARTIFACT_NOTE:-}" ]]; then
        printf 'operator_note\t%s\n' "$ARTIFACT_NOTE"
    fi
} > "$receipt"

# 4. Append-only run ledger: one row per run, never rewritten.
if [[ ! -f "$ledger" ]]; then
    printf 'run_id\tcaptured_utc\tserial_sha256\twasm\tasset\tbinary\tinitfs\tesp\tdtb\tnote\n' > "$ledger"
fi
if ! cut -f1 "$ledger" | grep -qx "$run_id"; then
    printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
        "$run_id" "$captured_utc" "$serial_sha" "$wasm_sha" "$asset_sha" "$binary_sha" \
        "$initfs_sha" "$esp_sha" "$dtb_sha" "validator ACCEPT; receipts: docs/workbench/evidence/$run_id-receipt.tsv" >> "$ledger"
fi

# 5. Regenerate the document for THIS run, naming it explicitly.
{
    printf 'E1 QEMU SANDBOX GATE — device-free execution proof\n'
    printf '=================================================\n\n'
    printf 'This document describes run %s, captured %s PT (%s UTC). The receipt for it\n' "$run_id" "$captured_pt" "$captured_utc"
    printf 'was written %s PT. Every run ever recorded for this gate is listed under RUNS\n' "$recorded_pt"
    printf 'below; the hashes in this document belong to %s and to no other run.\n\n' "$run_id"
    if [[ -n "${GATE_NOTE:-}" ]]; then
        printf 'GATE NOTE\n---------\n'
        printf '%s\n\n' "$GATE_NOTE"
    fi
    printf 'Worktree: redox-oneplus6-qemu   Branch: %s\n' "$branch"
    printf 'Host: %s   %s\n\n' "$(uname -sr)" "$(qemu-system-aarch64 --version | head -1)"

    printf 'RESULT\n------\n'
    printf 'The Wasm service sandbox executed on real AArch64 Redox inside QEMU. The run is\n'
    printf 'accepted only because the app declared success AFTER every assertion, which the\n'
    printf 'strict validator (scripts/qemu-demo/validate.sh) checked on the raw serial:\n\n'
    printf '  SANDBOX_HELLO_OK                       (granted run completed)\n'
    printf '  SANDBOX_REFUSED_TRACEEMIT              (refusal + stop reason, printed only\n'
    printf '                                          after the re-entry assertion holds)\n'
    printf '  stopped service is never re-entered  (the final refusal assertion ran)\n'
    printf '  sandbox-demo: done                   (terminal: printed after every assertion)\n\n'
    printf 'A capture containing `sandbox-demo: FAIL`, or missing the terminal done line, is\n'
    printf 'rejected by the validator and produces no evidence at all.\n\n'
    printf 'The guest ran with NO network device: the firmware was loaded from a FAT ESP on\n'
    printf 'virtio-blk (bootindex=1), so there is no virtio-net and no TFTP. The UEFI line\n'
    printf '"BdsDxe: starting Boot0001 ... from PciRoot(0x0)/Pci(0x2,0x0)" is that virtio-blk\n'
    printf 'device. Nothing in the RAM initfs starts a network daemon.\n\n'

    printf 'WHAT RAN\n--------\n'
    printf 'apps/sandbox-demo (aarch64-unknown-redox ELF) links crates/sandbox (wasmi 2.0.0)\n'
    printf 'and embeds assets/hello_rs.wasm, whose digest is pinned in src/main.rs and is\n'
    printf 'checked against the freshly built guest by scripts/qemu-demo/check-digest.sh\n'
    printf 'before the cross-build is allowed to start. It is started by init as\n'
    printf '45_sandbox_demo.service (oneshot, requires_weak 00_traced) from the\n'
    printf '90_ramboot.target, and its stderr goes through logd to the kernel debug console.\n'
    printf 'Run 1 grants every capability; run 2 withholds trace.emit.\n\n'
    printf 'Note on the Loaded event: its `requested` list is what the module IMPORTS, not\n'
    printf 'what was granted — identical in both runs by design. The grants differ, which is\n'
    printf 'why run 2 ends in Denied { TraceEmit, NotGranted }.\n\n'

    printf 'NOT PROOF\n---------\n'
    printf 'The host (x86_64) smoke run of the same app and the aarch64 rlib/ELF build are not\n'
    printf 'execution proof. Only the AArch64 Redox serial markers below count. And the\n'
    printf 'hashes below are receipts for run %s only: this build chain is not bit-\n' "$run_id"
    printf 'reproducible, so re-staging the same source yields different initfs/ESP/DTB\n'
    printf 'digests. Hashes identify a run; they do not prove two runs were identical.\n\n'

    printf 'RUNS (append-only ledger, docs/workbench/QEMU-SANDBOX-DEMO-runs.tsv)\n'
    printf '%s\n' '--------------------------------------------------------------------'
    column -t -s "$(printf '\t')" "$ledger" | sed 's/^/  /'
    printf '\n'

    printf 'EXACT QEMU COMMAND LINE\n-----------------------\n'
    printf '  timeout --signal=TERM --kill-after=5 300 qemu-system-aarch64 \\\n'
    printf '      -machine virt,acpi=off,gic-version=3 -cpu max -smp 1 -m 2048 \\\n'
    printf '      -bios /usr/share/qemu-efi-aarch64/QEMU_EFI.fd \\\n'
    printf '      -drive if=none,id=esp,file=out/qemu-demo/esp.img,format=raw,readonly=on \\\n'
    printf '      -device virtio-blk-pci,drive=esp,bootindex=1 \\\n'
    printf '      -device ramfb -dtb out/qemu-demo/simplefb-virt.dtb \\\n'
    printf '      -pidfile out/qemu-demo/qemu.pid \\\n'
    printf '      -display none -serial file:out/qemu-demo/serial.raw -monitor none -no-reboot\n\n'
    printf 'No -nic / -netdev / -device virtio-net-pci is present. Paths are relative to the\n'
    printf 'worktree root. The ESP image is mounted read-only. TERM at the deadline, KILL 5 s\n'
    printf 'later, and an EXIT trap that stops the child if this script dies first.\n'
    printf 'This is the gate command line as of the commit that wrote this document; a run\n'
    printf 'captured by an earlier version of the gate is flagged in its own receipt.\n\n'

    printf 'ARTIFACT SHA256 — receipts for run %s\n---------------------------------\n' "$run_id"
    printf '  Provenance: %s\n' "$provenance"
    printf '  A value of UNKNOWN means the artifact for THIS run is not attributable here.\n'
    printf '  It is never filled in with the current build of this worktree, which is another\n'
    printf '  run.\n\n'
    printf '  %s  apps/sandbox-demo/assets/hello_rs.wasm (embedded, pinned — current repo tree)\n' "$asset_sha"
    printf '  %s  source pin HELLO_RS_SHA256 (current repo tree)\n' "$pin_sha"
    printf '  %s  out/qemu-demo/wasm/.../hello_rs.wasm (fresh build)\n' "$wasm_sha"
    printf '  %s  out/qemu-demo/redox/aarch64-unknown-redox/release/sandbox-demo\n' "$binary_sha"
    printf '  %s  out/qemu-demo/ramboot-initfs.img\n' "$initfs_sha"
    printf '  %s  out/qemu-demo/bootloader-e1/build/bootloader-ramboot.efi\n' "$efi_sha"
    printf '  %s  out/qemu-demo/esp.img\n' "$esp_sha"
    printf '  %s  out/qemu-demo/simplefb-virt.dtb\n' "$dtb_sha"
    printf '  %s  %s (this run)\n\n' "$serial_sha" "${serial#"$repo"/}"
    if [[ -n "${ARTIFACT_NOTE:-}" ]]; then
        printf '  Operator note: %s\n\n' "$ARTIFACT_NOTE"
    fi

    printf 'MARKERS, VERBATIM FROM THE RAW SERIAL\n'
    printf '%s\n' '--------------------------------- '
    grep 'sandbox-demo' "$strip" | sed 's/^/  /'
    printf '\n'

    printf 'HOW TO REPRODUCE\n--------------\n'
    printf '  scripts/qemu-demo/build.sh       # wasm guest, digest gate, then cross-build\n'
    printf '  scripts/qemu-demo/stage.sh       # initfs copy + unit + pack + ramboot EFI + ESP\n'
    printf '  scripts/qemu-demo/boot.sh        # bounded boot, strict validator decides\n'
    printf '  scripts/qemu-demo/evidence.sh    # validate, then write receipts + this document\n'
    printf '  scripts/qemu-demo/tests.sh       # fault-injection tests for the gate itself\n\n'
    printf 'To archive a run so it can be re-documented later without the worktree:\n'
    printf '  sha256sum serial.raw esp.img ramboot-initfs.img simplefb-virt.dtb ... > SHA256SUMS\n'
    printf '  scripts/qemu-demo/evidence.sh --serial <archived serial> [--manifest <SHA256SUMS>]\n'
    printf 'The manifest must re-verify line by line and must carry a row that RESOLVES to\n'
    printf 'that capture. Artifact roles bind to these exact manifest paths:\n'
    printf '  hello_rs.wasm sandbox-demo ramboot-initfs.img bootloader-ramboot.efi esp.img\n'
    printf '  simplefb-virt.dtb        (relative to the manifest; never matched by filename)\n'
    printf 'An archive that stores an artifact elsewhere says so with --map <role>=<path>, e.g.\n'
    printf '  --map esp=run/esp.img.   Anything not bound this way reads UNKNOWN.\n\n'
    printf 'All build outputs are worktree-local under out/qemu-demo/ (gitignored). The fork\n'
    printf 'trees (forks/redox, base-e1, kernel-e1, bootloader-e1) are read-only inputs; the\n'
    printf 'cookbook container mounts /mnt/redox read-only and uses a worktree-local CARGO_HOME.\n\n'

    printf 'FULL SERIAL FOR RUN %s (terminal control sequences stripped; raw\n' "$run_id"
    printf 'capture %s, sha256 %s; archived copy:\n' "${serial#"$repo"/}" "$serial_sha"
    printf 'docs/workbench/evidence/%s-serial.txt)\n' "$run_id"
    printf '=============================================================\n\n'
    cat "$strip"
} > "$doc"

printf 'run %s: receipts in docs/workbench/evidence/, ledger row appended, wrote %s (%s bytes)\n' \
    "$run_id" "${doc#"$repo"/}" "$(wc -c < "$doc")"
