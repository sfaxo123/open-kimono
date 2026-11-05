#!/usr/bin/env bash
# Fixture tests for validate-usbgadgetd.sh (Wonder diff review of
# c959a28: the validator must pin WHAT it accepts, not just accept the
# one happy capture). Each case builds a synthetic serial capture and
# asserts the validator's exit code:
#   combined daemon+clean line, then ram:~$        -> ACCEPT (0)
#   the pinned real V2 capture (fixtures/)         -> ACCEPT (0)
#   daemon name and clean marker on disjoint lines -> REJECT (1)
#   ram:~$ BEFORE the daemon-evidence line         -> REJECT (1)
#   bare `panic` / `PANIC` anywhere                -> REJECT (1)
#   ram:~$ only, no daemon evidence                -> REJECT (1)
# Exits 0 only if every case behaves as required.
set -uo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
validate="$here/validate-usbgadgetd.sh"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
fails=0

check() { # name expected_exit file
    local name=$1 want=$2 file=$3 rc=0
    "$validate" "$file" >"$tmp/out" 2>&1 || rc=$?
    if [[ $rc -eq $want ]]; then
        printf 'PASS %s (exit %s as required)\n' "$name" "$rc"
    else
        printf 'FAIL %s: validator exit %s, required %s\n' "$name" "$rc" "$want"
        sed 's/^/    /' "$tmp/out"
        fails=$((fails + 1))
    fi
}

combined='usbgadgetd: no DWC3 hardware behind model windows; init failed cleanly: UnsupportedCore(0)'

printf 'boot noise\n%s\ninit: usbgadgetd failed with exit status: 2\nram:~$\n' "$combined" >"$tmp/accept.txt"
check 'combined-line-then-prompt' 0 "$tmp/accept.txt"

logonly='usbgadgetd: SMMU log-only probe: seq 1 -> NotMatched; seq 2 -> NotMatched; no matched SMR; verified DMA mode: Err(SidNotMatched); DMA refused: log-only first wired run, no DMA attempted'
printf 'boot noise\n%s\ninit: usbgadgetd failed with exit status: 3\nram:~$\n' "$logonly" >"$tmp/accept-v2.txt"
check 'payload-v2-logonly-line-then-prompt' 0 "$tmp/accept-v2.txt"

fixture="$here/fixtures/v2-serial-accepted.raw"
if [[ -s "$fixture" ]]; then
    check 'pinned-real-v2-capture' 0 "$fixture"
else
    printf 'FAIL pinned-real-v2-capture: fixture missing at %s\n' "$fixture"
    fails=$((fails + 1))
fi

v2fail='usbgadgetd: SMMU log-only probe failed cleanly: s2cr thing; no DMA attempted'
printf '%s\ninit: usbgadgetd failed with exit status: 3\nram:~$\n' "$v2fail" >"$tmp/v2-failure-line.txt"
check 'payload-v2-failure-line-not-evidence' 1 "$tmp/v2-failure-line.txt"

v2bypass='usbgadgetd: SMMU log-only probe: seq 1 -> Bypass { smr: 9 }; seq 2 -> Bypass { smr: 9 }; SMR[9]=0x80010740 S2CR[9]=0x00010000; verified DMA mode: Ok(Bypass); DMA refused: log-only first wired run, no DMA attempted'
printf '%s\ninit: usbgadgetd failed with exit status: 3\nram:~$\n' "$v2bypass" >"$tmp/v2-bypass-line.txt"
check 'payload-v2-bypass-line-not-evidence' 1 "$tmp/v2-bypass-line.txt"

printf 'boot noise\n%s\nram:~$\n' "$logonly" >"$tmp/v2-no-exit3.txt"
check 'payload-v2-line-without-exit-3' 1 "$tmp/v2-no-exit3.txt"

printf 'usbgadgetd: starting\nunrelated driver: UnsupportedCore(0)\nram:~$\n' >"$tmp/disjoint.txt"
check 'disjoint-lines' 1 "$tmp/disjoint.txt"

printf 'ram:~$\n%s\n' "$combined" >"$tmp/prompt-first.txt"
check 'prompt-before-daemon' 1 "$tmp/prompt-first.txt"

printf '%s\nram:~$\nkernel panic - not syncing: fatal exception\n' "$combined" >"$tmp/panic-lower.txt"
check 'bare-panic-lowercase' 1 "$tmp/panic-lower.txt"

printf '%s\nram:~$\nPANIC: unhandled\n' "$combined" >"$tmp/panic-upper.txt"
check 'bare-PANIC-uppercase' 1 "$tmp/panic-upper.txt"

printf 'boot noise\nram:~$\n' >"$tmp/ramsh-only.txt"
check 'ramsh-only-partial' 1 "$tmp/ramsh-only.txt"

if (( fails )); then
    printf 'test-validate-usbgadgetd: %s case(s) FAILED\n' "$fails" >&2
    exit 1
fi
printf 'test-validate-usbgadgetd: all cases PASS\n'
exit 0
