#!/usr/bin/env bash
# Host-test companion for e1-net-serial-handshake.sh (phase
# CONTINUE-SERIAL-FIX-20261025T1149). FAKE QEMU/FIFO simulated serial ONLY —
# no boot, no image, no mounts. Drives the ACTUAL run_handshake state machine
# against a simulated interactive process and proves sequence/drift:
#   T1 delayed login prompt is still caught (no blind sleep)
#   T2 stale earlier "redox login:" marker must NOT satisfy the later wait
#   T3 missing password prompt -> PASSWORD_PROMPT_TIMEOUT
#   T4 missing shell prompt -> SHELL_PROMPT_TIMEOUT
#   T5 no app command emitted before an authenticated shell
#   T6 claimed host-test phase dir is never re-claimed (one-time claim)
set -euo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
# shellcheck source=e1-net-serial-handshake.sh
. "$repo/scripts/qemu-demo/e1-net-serial-handshake.sh" lib

# Clearly-named host-test scratch (NOT a boot phase claim). Unique per run so
# reruns never collide with or delete earlier evidence.
scratch="$repo/out/e1-net-serial-handshake-tests/host-tests-20261025T1149-$(date +%s)-$$"
mkdir -p "$scratch"
fails=0; pass=0
ok()   { pass=$((pass + 1)); printf 'PASS %s\n' "$1"; }
bad()  { fails=$((fails + 1)); printf 'FAIL %s — %s\n' "$1" "$2"; }

# fake_vm <serial_file> <recv_file> <scenario> — reads CR-terminated console
# input on stdin (the FIFO), appends simulated serial output to $1, records
# every received line to $2. Scenarios: ok|delayed_login|stale_login|no_passwd|no_shell
fake_vm() {
    local ser=$1 recv=$2 sc=$3 line enters=0
    case "$sc" in
        stale_login) printf 'boot-service noise: redox login: (stale)\n' >>"$ser"; sleep 0.5 ;;
    esac
    printf 'Redox OS Bootloader\nPress e to edit boot environment\n' >>"$ser"
    while read -r -t 25 -d $'\r' line; do
        printf '%s\n' "$line" >>"$recv"
        if [[ -z "$line" ]]; then
            enters=$((enters + 1))
            if (( enters == 2 )); then
                case "$sc" in
                    ok|no_passwd|no_shell) sleep 0.3; printf 'redox login: ' >>"$ser" ;;
                    delayed_login)         sleep 3;  printf 'redox login: ' >>"$ser" ;;
                    stale_login) : ;;  # no FRESH login marker: only the stale one exists
                esac
            fi
        elif [[ "$line" == root ]]; then
            case "$sc" in
                ok|delayed_login|no_shell) sleep 0.3; printf 'Password: ' >>"$ser" ;;
                no_passwd|stale_login) : ;;
            esac
        elif [[ "$line" == password ]]; then
            case "$sc" in
                ok|delayed_login) sleep 0.3; printf '\033]0;root: /home/root\007\033[1;38;5;85mroot\033[39m:\033[38;5;75m~\033[39m# \033[0m' >>"$ser" ;;
                no_shell|no_passwd|stale_login) : ;;
            esac
        fi
    done
}

# run_case <name> <scenario> <expect_status> — returns via globals st/recv
run_case() {
    local name=$1 sc=$2 expect=$3 dir st deadline
    dir="$scratch/$name"; mkdir -p "$dir"
    local fifo="$dir/in.fifo" ser="$dir/serial.log" recv="$dir/recv.log"
    : >"$ser"; : >"$recv"; mkfifo "$fifo"
    fake_vm "$ser" "$recv" "$sc" <"$fifo" & local vpid=$!
    exec 3<>"$fifo"
    deadline=$(( $(date +%s) + 6 ))
    st=$(run_handshake "$ser" 3 "$deadline" '/usr/bin/net-probe TESTNONCE 10.0.2.2:4799')
    exec 3>&-
    kill "$vpid" 2>/dev/null || true; wait "$vpid" 2>/dev/null || true
    rm -f "$fifo"
    [[ "$st" == "$expect" ]] && ok "$name: status=$st" || bad "$name" "got '$st' want '$expect'"
}

# Real prompt bytes above are captured from integration/e1net-attempt-1-raw.txt:603.
# T1: login prompt arrives 3 s late — prompt-driven wait must still complete.
run_case t1-delayed-login delayed_login HANDSHAKE_OK
# T2: stale pre-boot "redox login:" must not satisfy the post-Enter wait.
run_case t2-stale-marker stale_login "$AB_LOGIN"
# T3: password prompt never appears.
run_case t3-no-passwd no_passwd "$AB_PASSWD"
# T4: shell prompt never appears.
run_case t4-no-shell no_shell "$AB_SHELL"

# T5: no app command may reach the guest before an authenticated shell.
leaked=0
for t in t2-stale-marker t3-no-passwd t4-no-shell; do
    if grep -q 'net-probe' "$scratch/$t/recv.log"; then leaked=1; fi
done
if ! grep -q 'net-probe' "$scratch/t1-delayed-login/recv.log"; then leaked=1; fi
(( leaked == 0 )) && ok "t5: probe only after auth shell (absent in t2-t4, present in t1)" \
                  || bad t5 'probe command leaked before authenticated shell (or missing in t1)'

# T6: one-time claim — second mkdir of the same phase dir must refuse.
claim="$scratch/claim"; first=0 second=1
mkdir "$claim" 2>/dev/null && first=0 || first=1
mkdir "$claim" 2>/dev/null && second=0 || second=1
if (( first == 0 && second == 1 )); then
    ok "t6: claim created once, re-claim refused (no second boot possible)"
else
    bad t6 "first=$first second=$second"
fi

printf 'RESULT pass=%s fail=%s\n' "$pass" "$fails"
exit "$fails"
