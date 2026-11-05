#!/usr/bin/env bash
# E1 Wyoming probe, host negative checks. Each case proves the client cannot
# hang and that failures propagate, with no orphan python processes left.
#
# N1 no-reply peer: a fixture that accepts and never answers -> the client
#    must fail on its socket timeout well inside the outer bound, rc 1.
# N2 failing peer (close-before-reply): the fixture accepts and closes ->
#    the client must report the closed connection, rc 1.
# N3 failure propagation through interop.sh: a client that always fails must
#    make interop.sh exit non-zero promptly AND reap its python server.
#
# Every invocation here carries a hard outer `timeout --kill-after`, so a
# regression in the client or the harness cannot hang this script either.
set -uo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
out="$repo/out/wyoming-probe"
bin=${WYOMING_PROBE_BIN:-"$out/host/release/wyoming-probe"}
mkdir -p "$out"
fail=0

# A python fixture that binds, prints PORT, accepts, then does the named
# thing: 'silent' (never replies) or 'eof' (closes the accepted socket).
# `exec` so $! is the python pid itself, not a wrapper subshell — otherwise
# killing $! would orphan the fixture.
fixture() {
    exec python3 - "$1" <<'PY'
import socket, sys
mode = sys.argv[1]
s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
s.settimeout(60)
s.bind(("127.0.0.1", 0))
s.listen(1)
print(f"PORT {s.getsockname()[1]}", flush=True)
conn, _ = s.accept()
if mode == "eof":
    conn.close()
    s.close()
    sys.exit(0)
import time
time.sleep(60)          # 'silent': accept, then never answer
PY
}

wait_port() {  # $1 = log file
    for _ in $(seq 1 200); do
        grep -q '^PORT ' "$1" && return 0
        sleep 0.05
    done
    return 1
}

kill_fixture() {  # $1 = pid: kill, bound the death, reap
    local pid=$1
    if kill -0 "$pid" 2>/dev/null; then
        kill "$pid" 2>/dev/null || true
        for _ in $(seq 1 60); do
            kill -0 "$pid" 2>/dev/null || break
            sleep 0.05
        done
        if kill -0 "$pid" 2>/dev/null; then kill -9 "$pid" 2>/dev/null || true; fi
    fi
    wait "$pid" 2>/dev/null || true
}

owned=()
orphans() {  # only fixture PIDs this test owns; never unrelated workers
    local pid
    for pid in "${owned[@]}"; do
        kill -0 "$pid" 2>/dev/null && echo "$pid"
    done
    return 0
}

echo "== N1 no-reply peer (silent server) =="
: >"$out/n1-fixture.log"
fixture silent >"$out/n1-fixture.log" 2>&1 &
fx=$!
owned+=("$fx")
if wait_port "$out/n1-fixture.log"; then
    p=$(sed -n 's/^PORT //p' "$out/n1-fixture.log" | head -1)
    start=$(date +%s)
    timeout --kill-after=5s 30 "$bin" "127.0.0.1:$p" "n1-$$" 3   # 3 s client deadline
    rc=$?
    el=$(( $(date +%s) - start ))
    echo "N1: rc=$rc elapsed=${el}s (client deadline 3s, outer timeout 30s)"
    [[ $rc -eq 1 && $el -le 8 ]] || { echo "N1 FAIL: expected rc 1 inside the bound"; fail=1; }
else
    echo "N1 FAIL: fixture never printed PORT"; fail=1
fi
kill_fixture "$fx"

echo "== N2 peer closes before reply =="
: >"$out/n2-fixture.log"
fixture eof >"$out/n2-fixture.log" 2>&1 &
fx=$!
owned+=("$fx")
if wait_port "$out/n2-fixture.log"; then
    p=$(sed -n 's/^PORT //p' "$out/n2-fixture.log" | head -1)
    timeout --kill-after=5s 30 "$bin" "127.0.0.1:$p" "n2-$$" 5
    rc=$?
    echo "N2: rc=$rc"
    [[ $rc -eq 1 ]] || { echo "N2 FAIL: expected rc 1"; fail=1; }
else
    echo "N2 FAIL: fixture never printed PORT"; fail=1
fi
kill_fixture "$fx"

echo "== N3 interop.sh propagates a failing client and reaps the server =="
start=$(date +%s)
set +e
timeout --kill-after=5s 45 env WYOMING_PROBE_BIN=/bin/false WYOMING_PROBE_TIMEOUT=10 \
    bash "$repo/apps/wyoming-probe/interop.sh" >"$out/n3-interop.log" 2>&1
rc=$?
set -e
server_pid=$(sed -n 's/^interop: server_pid=//p' "$out/n3-interop.log")
[[ "$server_pid" =~ ^[0-9]+$ ]] && owned+=("$server_pid")
el=$(( $(date +%s) - start ))
echo "N3: interop.sh rc=$rc elapsed=${el}s (outer bound 45s)"
tail -2 "$out/n3-interop.log"
[[ $rc -ne 0 ]] || { echo "N3 FAIL: interop.sh returned 0 for a failing client"; fail=1; }
[[ $el -le 20 ]] || { echo "N3 FAIL: interop.sh took ${el}s — the server reap is not bounded"; fail=1; }

sleep 1
left=$(orphans)
if [[ -n "$left" ]]; then
    echo "ORPHANS: $left"; fail=1
else
    echo "no orphan python processes"
fi

[[ $fail -eq 0 ]] && echo "NEGATIVES PASS" || echo "NEGATIVES FAIL"
exit "$fail"
