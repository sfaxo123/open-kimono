#!/usr/bin/env bash
# E1 Wyoming probe, host interop smoke: run the wyoming-probe client against
# the EXISTING pinned Python reference **echo fixture** (crates/wyoming/tests/
# pyinterop_server.py, wyoming 1.11.0 @ 08a0c25). Nothing here re-implements
# the reference: the server is started exactly as tests/pyinterop.rs starts it
# (PYTHONPATH = reference checkout : tests/pyref_meta).
#
# Lifetime bounds (learned the hard way — see docs/workbench/E1-WYOMING-PROBE.md):
# * the client runs under `timeout --kill-after` so a TERM-resistant client dies;
# * on client failure the server is KILLED BEFORE `wait`, because a server left
#   blocked in accept() makes an unbounded `wait` hang (observed: 114 s);
# * the reap is bounded on every path (poll, SIGKILL, then wait);
# * both exit statuses are reported, and a failing server fails the run.
set -euo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
pyref=${WYOMING_PYREF:-$HOME/Developer/forks/op6-refs/wyoming}
out="$repo/out/wyoming-probe"
bin=${WYOMING_PROBE_BIN:-"$out/host/release/wyoming-probe"}
client_timeout=${WYOMING_PROBE_TIMEOUT:-30}
nonce="wy-probe-$(date +%s)-$$"

mkdir -p "$out"
[[ -x "$bin" ]] || { echo "missing executable: $bin (build it first)" >&2; exit 2; }
[[ -d "$pyref/wyoming" ]] || { echo "missing python reference at $pyref" >&2; exit 2; }

server=""
reap() {
    # Kill, bound the death, then reap. Never a bare `wait` on a live server.
    [[ -z "$server" ]] && return 0
    if kill -0 "$server" 2>/dev/null; then
        kill "$server" 2>/dev/null || true
        for _ in $(seq 1 100); do
            kill -0 "$server" 2>/dev/null || break
            sleep 0.05
        done
        if kill -0 "$server" 2>/dev/null; then kill -9 "$server" 2>/dev/null || true; fi
    fi
    wait "$server" 2>/dev/null || true
}
cleanup() { reap; }
trap cleanup EXIT

# Same PYTHONPATH the Rust harness builds (pyinterop.rs::pyref_pythonpath):
# the read-only reference checkout plus tests/pyref_meta, which supplies the
# dist-info metadata wyoming/version.py reads via importlib.metadata.
PYTHONPATH="$pyref:$repo/crates/wyoming/tests/pyref_meta" python3 \
    "$repo/crates/wyoming/tests/pyinterop_server.py" \
    >"$out/server.log" 2>&1 &
server=$!
echo "interop: server_pid=$server"

for _ in $(seq 1 200); do
    grep -q '^PORT ' "$out/server.log" && break
    kill -0 "$server" 2>/dev/null || { echo "python server died:"; cat "$out/server.log"; exit 1; }
    sleep 0.05
done
grep '^REF ' "$out/server.log"
port=$(sed -n 's/^PORT //p' "$out/server.log" | head -1)
[[ -n "$port" ]] || { echo "python server never printed PORT" >&2; exit 1; }

set +e
timeout --kill-after=5s "$client_timeout" "$bin" "127.0.0.1:$port" "$nonce"
rc=$?
set -e

# Client failed or was killed: the fixture is probably still in accept().
# Kill it now so the reap below cannot block.
if [[ $rc -ne 0 ]] && kill -0 "$server" 2>/dev/null; then
    kill "$server" 2>/dev/null || true
fi
# Success path: the fixture exits on its own once the client closed; bound it.
for _ in $(seq 1 100); do
    kill -0 "$server" 2>/dev/null || break
    sleep 0.05
done
if kill -0 "$server" 2>/dev/null; then kill -9 "$server" 2>/dev/null || true; fi
set +e
wait "$server"; src=$?
set -e
trap - EXIT

cat "$out/server.log"
echo "interop: client rc=$rc (timeout ${client_timeout}s + kill-after 5s) python rc=$src nonce=$nonce port=$port"
if [[ $rc -eq 124 || $rc -eq 137 ]]; then
    echo "interop: FAIL — client exceeded ${client_timeout}s and was killed" >&2
    exit "$rc"
fi
# A passing client against a fixture that died is still a failure.
if [[ $rc -eq 0 && $src -ne 0 ]]; then
    echo "interop: FAIL — client rc 0 but python fixture exited $src" >&2
    exit "$src"
fi
exit "$rc"
