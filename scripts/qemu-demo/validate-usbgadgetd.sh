#!/usr/bin/env bash
# Acceptance validator for the V2 QEMU negative-path boot (usbgadgetd).
#
# What V2 proves (USBGADGETD-WIRING.md section 8): QEMU has no peripheral
# DWC3, so usbgadgetd must fail cleanly and the debug ramsh must still
# reach ram:~$. A capture is therefore accepted only if, after ANSI
# stripping, it contains:
#   - the debug shell prompt `ram:~$` (the daemon's failure did not cost
#     the operator the shell), and
#   - daemon failure evidence: ONE line naming usbgadgetd together with
#     its clean negative init (`UnsupportedCore(0)` / `init failed
#     cleanly` / `no DWC3 hardware`) - the same line, not disjoint lines
#     (payload v2, 2026-10-30: the wired daemon's log-only SMMU probe
#     line is the same kind of evidence ONLY in its strict QEMU form -
#     seq 1 and seq 2 NotMatched, verified Err(SidNotMatched), DMA
#     refused, and init reporting exit status 3 after it; the bare
#     substring also matches the daemon's failure line and verified
#     Bypass/Translated lines, which must NOT be accepted here)
#     spliced from anywhere in the capture (Wonder diff review of
#     c959a28), and
#   - a `ram:~$` prompt on a line AFTER that daemon-evidence line (a
#     prompt printed before the daemon ran proves nothing), and
#   - NO panic anywhere: the panic scan is case-insensitive, so bare
#     `panic` / `PANIC` count, not only `panicked`.
# Daemon evidence is reported either way so a ram:~$-only capture is
# recorded as PARTIAL, never silently accepted. Synthetic fixtures for
# every rule live in test-validate-usbgadgetd.sh; the accepted V2
# capture is pinned in fixtures/v2-serial-accepted.raw.
#
# usage:
#   validate-usbgadgetd.sh <serial-file>          exit 0 accepted, 1 rejected/partial
#   validate-usbgadgetd.sh --poll <serial-file>   exit 0 accepted, 2 panic seen,
#                                                 1 still incomplete (keep waiting)
set -euo pipefail

poll=0
if [[ ${1:-} == --poll ]]; then poll=1; shift; fi
serial=${1:-}
[[ -n "$serial" ]] || { printf 'usage: validate-usbgadgetd.sh [--poll] <serial-file>\n' >&2; exit 1; }

if [[ ! -s "$serial" ]]; then
    printf 'validate-usbgadgetd: no capture yet at %s\n' "$serial"
    exit 1
fi

tmp=$(mktemp)
trap 'rm -f "$tmp"' EXIT
sed 's/\x1b\[[0-9;]*[a-zA-Z]//g; s/\x1b\[=[0-9;]*[a-zA-Z]//g; s/\x1b[()][A-Z0-9]//g; s/\r//g' \
    "$serial" > "$tmp"

line_of() { grep -nF -- "$1" "$tmp" | head -1 | cut -d: -f1; }

ramsh_any=$(line_of 'ram:~$' || true)
panic=$(grep -niE 'panic|UNHANDLED EXCEPTION' "$tmp" | head -1 | cut -d: -f1 || true)
# Combined evidence: a single line that names usbgadgetd AND carries a
# clean-negative-init marker. Disjoint lines never satisfy this.
daemon=$(grep -nE 'usbgadgetd' "$tmp" \
    | grep -E 'UnsupportedCore\(0\)|init failed cleanly|no DWC3 hardware' \
    | head -1 | cut -d: -f1 || true)
# Payload-v2 evidence is STRICTER (Wonder diff review of 5a4e2cf): the
# bare substring 'SMMU log-only probe' would also accept the daemon's
# failure line ('...probe failed cleanly...', no scan ever verified)
# and a verified-Bypass/Translated line. The QEMU negative demo is
# pinned exactly: one line with seq 1 AND seq 2 NotMatched, the
# verified Err(SidNotMatched) answer, and the DMA refusal - PLUS the
# init line reporting exit status 3 AFTER it.
if [[ -z "$daemon" ]]; then
    v2=$(grep -nE 'usbgadgetd: SMMU log-only probe: seq 1 -> NotMatched; seq 2 -> NotMatched;.*verified DMA mode: Err\(SidNotMatched\); DMA refused' "$tmp" \
        | head -1 | cut -d: -f1 || true)
    if [[ -n "$v2" ]]; then
        exit3=$(awk -v d="$v2" 'NR > d && index($0, "usbgadgetd") && index($0, "failed with exit status: 3") { print NR; exit }' "$tmp" || true)
        [[ -n "$exit3" ]] && daemon=$v2
    fi
fi
# The shell prompt must survive the daemon failure: first ram:~$ AFTER
# the daemon-evidence line.
ramsh=''
if [[ -n "$daemon" ]]; then
    ramsh=$(awk -v d="$daemon" 'NR > d && index($0, "ram:~$") { print NR; exit }' "$tmp" || true)
fi

if (( poll )) && [[ -n "$panic" ]]; then
    printf 'validate-usbgadgetd: panic on the serial, stopping the boot early\n'
    exit 2
fi

reasons=()
[[ -n "$panic" ]] && reasons+=("panic / unhandled exception on line $panic")
[[ -n "$daemon" ]] || reasons+=("no single line proves the daemon's clean negative init (a line naming usbgadgetd together with UnsupportedCore(0) / init failed cleanly / no DWC3 hardware, or the strict payload-v2 log-only line: seq 1+2 NotMatched, Err(SidNotMatched), DMA refused, followed by exit status 3) - disjoint lines do not count (PARTIAL at best)")
[[ -n "$ramsh" ]] || reasons+=("no ram:~\$ prompt AFTER the daemon-evidence line - the daemon failure cost the shell, or the only prompt predates the daemon")

if (( ${#reasons[@]} )); then
    printf 'validate-usbgadgetd: REJECT %s\n' "$serial"
    for r in "${reasons[@]}"; do printf '  - %s\n' "$r"; done
    printf '  daemon-evidence line: %s; ram:~$ after it: %s; first ram:~$ anywhere: %s\n' "${daemon:-none}" "${ramsh:-none}" "${ramsh_any:-none}"
    exit 1
fi

printf 'validate-usbgadgetd: ACCEPT %s\n' "$serial"
printf '  daemon clean-negative-init line %s; ram:~$ after it on line %s; no panic\n' "$daemon" "$ramsh"
exit 0
