#!/usr/bin/env bash
# Strict acceptance validator for the E1 QEMU sandbox gate.
#
# Why this exists: the two markers are NOT acceptance. apps/sandbox-demo/src/main.rs
# prints SANDBOX_REFUSED_TRACEEMIT and then still has one assertion left (a stopped
# service must not be re-entered); if that assertion fails the app prints
# `sandbox-demo: FAIL ...` and exits non-zero — but both markers are already on the
# serial. Grepping for the two markers therefore accepts a run that failed. The
# only capture that counts is one where the app itself declared success, AFTER
# every assertion: `sandbox-demo: done`, appearing last, with no FAIL anywhere.
#
# usage:
#   validate.sh <serial-file>          exit 0 accepted, 1 rejected
#   validate.sh --poll <serial-file>   exit 0 accepted, 2 FAIL seen (stop now),
#                                      1 still incomplete (keep waiting)
set -euo pipefail

poll=0
if [[ ${1:-} == --poll ]]; then poll=1; shift; fi
serial=${1:-}
[[ -n "$serial" ]] || { printf 'usage: validate.sh [--poll] <serial-file>\n' >&2; exit 1; }

# A missing or empty capture is never a pass. In poll mode it is just "not yet".
if [[ ! -s "$serial" ]]; then
    printf 'validate: no capture yet at %s\n' "$serial"
    exit 1
fi

tmp=$(mktemp)
trap 'rm -f "$tmp"' EXIT
sed 's/\x1b\[[0-9;]*[a-zA-Z]//g; s/\x1b\[=[0-9;]*[a-zA-Z]//g; s/\x1b[()][A-Z0-9]//g; s/\r//g' \
    "$serial" > "$tmp"

line_of() { grep -nF -- "$1" "$tmp" | head -1 | cut -d: -f1; }

hello=$(line_of 'sandbox-demo: SANDBOX_HELLO_OK' || true)
refused=$(line_of 'sandbox-demo: SANDBOX_REFUSED_TRACEEMIT' || true)
reentry=$(line_of 'sandbox-demo: stopped service is never re-entered' || true)
final=$(line_of 'sandbox-demo: done' || true)
fail=$(line_of 'sandbox-demo: FAIL' || true)

reasons=()
[[ -n "$fail" ]] && reasons+=("the app printed FAIL on line $fail: $(grep -m1 -F 'sandbox-demo: FAIL' "$tmp")")
[[ -n "$hello" ]] || reasons+=("SANDBOX_HELLO_OK missing — the granted run did not complete")
[[ -n "$refused" ]] || reasons+=("SANDBOX_REFUSED_TRACEEMIT missing — the refusal run did not reach its stop")
[[ -n "$reentry" ]] || reasons+=("the final refusal assertion never ran: no 'stopped service is never re-entered' line")
[[ -n "$final" ]] || reasons+=("the app never declared success: no terminal 'sandbox-demo: done' line")

# Ordering: the granted run precedes the refusal run, and the terminal marker must
# come after every assertion result. A `done` printed before the refusal marker
# means the demo never reached run 2.
if [[ -n "$hello" && -n "$refused" && "$hello" -gt "$refused" ]]; then
    reasons+=("run order wrong: SANDBOX_HELLO_OK (line $hello) is not before SANDBOX_REFUSED_TRACEEMIT (line $refused)")
fi
if [[ -n "$final" ]]; then
    for pair in "SANDBOX_HELLO_OK:$hello" "SANDBOX_REFUSED_TRACEEMIT:$refused" "re-entry assertion:$reentry"; do
        n=${pair##*:}; name=${pair%%:*}
        [[ -z "$n" || "$final" -gt "$n" ]] || reasons+=("terminal 'done' (line $final) precedes $name (line $n) — an assertion after it never reported")
    done
fi

if (( poll )) && [[ -n "$fail" ]]; then
    printf 'validate: FAIL on the serial, stopping the boot early\n'
    printf '  %s\n' "$(grep -m1 -F 'sandbox-demo: FAIL' "$tmp" 2>/dev/null || true)"
    exit 2
fi

if (( ${#reasons[@]} )); then
    printf 'validate: REJECT %s\n' "$serial"
    for r in "${reasons[@]}"; do printf '  - %s\n' "$r"; done
    exit 1
fi

if (( poll )); then
    printf 'validate: ACCEPT %s (all assertions reported, terminal done present)\n' "$serial"
    exit 0
fi

printf 'validate: ACCEPT %s\n' "$serial"
printf '  SANDBOX_HELLO_OK            line %s\n' "$hello"
printf '  SANDBOX_REFUSED_TRACEEMIT   line %s\n' "$refused"
printf '  re-entry assertion          line %s\n' "$reentry"
printf '  terminal sandbox-demo: done line %s\n' "$final"
printf '  no sandbox-demo: FAIL line\n'
exit 0
