#!/usr/bin/env bash
# E1-NET-SERIAL-HANDSHAKE (host-only phase CONTINUE-SERIAL-FIX-20261025T1149,
# Codex-approved 4:49 AM PT). PROMPT-DRIVEN serial state machine — replaces the
# blind fixed-sleep injection that fed the probe into the bootloader ENV EDITOR
# in phase net-from-image-20261025T1137 (reviewed BOOTED_LOGIN_ONLY /
# HARNESS_HANDSHAKE_MISSING). Sequence, every wait BOUNDED with an EXPLICIT
# abort classification, all inside the 90 s VM deadline (deadline epoch passed
# in; the outer VM timeout remains the hard cap):
#   wait bootloader menu -> Enter x2 -> wait "redox login:" -> send root ->
#   wait password prompt -> send password -> wait shell prompt -> ONLY THEN
#   send the net-probe command.
# Readiness markers are the ones already proven in e1-net-exchange.sh (menu
# "Press e to edit boot environment", "redox login:", creds root|password,
# shell "root:~# ") — not invented. Stale earlier markers can NEVER satisfy a
# later wait: each wait scans only serial bytes appended AFTER it started.
# This phase is HOST-ONLY: no boot wiring here; sourcing with `lib` defines the
# helpers; tests drive run_handshake against a fake interactive process.
# Root owns e1-net-native-probe.sh classify + shared tests on main — untouched.
set -euo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)

# Menu/login markers follow the exchange harness. Root shell formatting was
# verified against integration/e1net-attempt-1-raw.txt:603 (archived real guest).
M_MENU='Press e to edit boot environment'
M_LOGIN='redox login:'
M_PASSWD='assword:'          # matches "Password:"/"password:" without case games
M_SHELL='root:~# '
CRED_USER='root'
CRED_PASS='password'

# Abort classifications (explicit, one per wait; also the drift guards).
AB_BOOTLOADER='BOOTLOADER_WAIT_TIMEOUT'
AB_LOGIN='LOGIN_PROMPT_TIMEOUT'
AB_PASSWD='PASSWORD_PROMPT_TIMEOUT'
AB_SHELL='SHELL_PROMPT_TIMEOUT'

poll_step=${POLL_STEP:-0.25}

# wait_marker <serial_file> <offset_var> <marker> <deadline_epoch> <abort_class>
# Scans ONLY bytes after the current offset (stale markers cannot satisfy a
# later wait), polls until the deadline, then returns the abort class on stdout.
wait_marker() {
    local f=$1 offvar=$2 marker=$3 deadline=$4 abort=$5 cur size
    cur=${!offvar}
    while :; do
        size=$(wc -c <"$f" 2>/dev/null || echo 0)
        # Parse the bounded snapshot without a tail|grep -q SIGPIPE race, and
        # strip the CSI/OSC formatting present in the real guest root prompt.
        if (( size > cur )) && python3 - "$f" "$cur" "$size" "$marker" <<'PYMARK'
from pathlib import Path
import re, sys
f, start, end, marker = sys.argv[1:]
with Path(f).open('rb') as stream:
    stream.seek(int(start)); data = stream.read(int(end) - int(start))
data = re.sub(rb'\x1b\][^\x07]*(?:\x07|\x1b\\)', b'', data)
data = re.sub(rb'\x1b\[[0-?]*[ -/]*[@-~]', b'', data)
sys.exit(0 if marker.encode() in data else 1)
PYMARK
        then
            printf -v "$offvar" '%s' "$size"
            return 0
        fi
        if (( $(date +%s) >= deadline )); then printf '%s' "$abort"; return 1; fi
        sleep "$poll_step"
    done
}

# send_line <fd> <text>  — CR-terminated console input, same UART as exchange.
send_line() { printf '%s\r' "$2" >&"$1"; }

# run_handshake <serial_file> <input_fd> <deadline_epoch> <probe_cmd>
# Runs the state machine; prints the final status word on stdout:
#   HANDSHAKE_OK | one of the AB_* abort classes.
# The probe command is emitted ONLY after the authenticated shell prompt.
run_handshake() {
    local f=$1 fd=$2 deadline=$3 probe=$4 off=0
    wait_marker "$f" off "$M_MENU" "$deadline" "$AB_BOOTLOADER" && {
        send_line "$fd" ''            # Enter #1 (documented bootloader nudge)
        sleep 0.5
        send_line "$fd" ''            # Enter #2
        printf 'handshake: bootloader Enter x2 sent\n' >&2
    } || return 0                     # abort word already on stdout
    wait_marker "$f" off "$M_LOGIN" "$deadline" "$AB_LOGIN" && {
        send_line "$fd" "$CRED_USER"
        printf 'handshake: login user sent\n' >&2
    } || return 0
    wait_marker "$f" off "$M_PASSWD" "$deadline" "$AB_PASSWD" && {
        send_line "$fd" "$CRED_PASS"
        printf 'handshake: password sent\n' >&2
    } || return 0
    wait_marker "$f" off "$M_SHELL" "$deadline" "$AB_SHELL" && {
        send_line "$fd" "$probe"      # ONLY NOW the app command
        printf 'handshake: probe sent after authenticated shell\n' >&2
        printf 'HANDSHAKE_OK'
    } || return 0
}

if [[ "${1:-}" == lib ]]; then return 0 2>/dev/null || exit 0; fi

# No implicit boot: this host-only phase ships helpers only. Boot wiring
# requires a NEW Codex-approved phase id and budget.
printf 'usage: source "%s" lib (host-only helpers; tests: e1-net-serial-handshake-tests.sh)\n' "$0" >&2
exit 2
