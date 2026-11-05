#!/usr/bin/env bash
# Host-only orchestration regression for e1-net-native-probe.sh.
# Calls the SAME evaluate_run the boot loop uses (extract -> classify -> accept
# ordering cannot drift from the tests). No QEMU, no image writes: the one boot
# interruption test runs the REAL script with a stubbed QEMU and SKIP_STAGE=1.
set -uo pipefail
repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
main="$repo/scripts/qemu-demo/e1-net-native-probe.sh"
# shellcheck source=/dev/null
. "$main" lib
set +e   # main sets -e; the test harness must keep going to print a full summary
t=$(mktemp -d "$out/tests.XXXXXX")
nonce='e1np-selftest-9001'; smoke_port=4799
pass=0; fail=0
want() {
    if [[ "$2" == "$3" ]]; then printf '  PASS  %s -> %s\n' "$1" "$2"; pass=$((pass+1));
    else printf '  FAIL  %s: want %s got %s\n' "$1" "$2" "$3"; fail=$((fail+1)); fi
}

# ---------------------------------------------------------------- pcap cases --
# Builder: one guest->host TCP packet (payload) + optional decoy/padding cases.
mkpcap() {  # $1=file $2=case
python3 - "$1" "$2" "$nonce" <<'PY'
import sys, struct
pcap, case, nonce = sys.argv[1:4]
def pkt(src, dst, proto, dport, payload, extra=b'', tot_override=None, cap_cut=None, frag=False):
    eth = b'\x52\x54\x00\x12\x34\x56\x52\x54\x00\x12\x34\x55' + b'\x08\x00'
    ihl, thl = 20, 20
    tot = tot_override if tot_override else ihl + thl + len(payload)
    fl = 0x2000 if frag else 0
    ip = bytes([0x40 | (ihl >> 2)]) + b'\x00' + struct.pack('>H', tot) + b'\x00\x00' + struct.pack('>H', fl) + bytes([64, proto, 0, 0])
    ip += bytes(map(int, src.split('.'))) + bytes(map(int, dst.split('.')))
    tcp = struct.pack('>HHIIBBHHH', 40000, dport, 0, 0, (thl >> 2) << 4, 0x18, 64224, 0, 0)
    frame = eth + ip + tcp + payload + extra
    if cap_cut: frame = frame[:cap_cut]
    return frame
def rec(data): return struct.pack('<IIII', 0, 0, len(data), len(data)) + data
out = open(pcap, 'wb')
out.write(struct.pack('<IHHiIII', 0xa1b2c3d4, 2, 4, 0, 0, 65535, 1))
g, h = '10.0.2.15', '10.0.2.2'
if case == 'good':
    out.write(rec(pkt(g, h, 6, 4766, (nonce + '\n').encode())))
    out.write(rec(pkt(h, g, 6, 40000, b'DECOY-' + nonce.encode() + b'\n')))  # reverse: must drop
elif case == 'padded':   # nonce ONLY in Ethernet padding beyond total_length
    out.write(rec(pkt(g, h, 6, 4766, b'short\n', extra=nonce.encode() + b'\x00' * 8)))
elif case == 'nontcp':   # nonce in a UDP payload: not TCP
    out.write(rec(pkt(g, h, 17, 4766, (nonce + '\n').encode())))
elif case == 'truncated':  # capture cut before total_length
    out.write(rec(pkt(g, h, 6, 4766, (nonce + '\n').encode(), cap_cut=44)))
elif case == 'fragment':
    out.write(rec(pkt(g, h, 6, 4766, (nonce + '\n').encode(), frag=True)))
elif case == 'affix':   # real protocol framing but affixed: prefix<n>suffix must refuse
    out.write(rec(pkt(g, h, 6, 4766, ('prefix-' + nonce + '-suffix\n').encode())))
elif case == 'split':      # nonce split across two payloads: conservative refuse
    half = len(nonce) // 2
    out.write(rec(pkt(g, h, 6, 4766, nonce[:half].encode())))
    out.write(rec(pkt(g, h, 6, 4766, (nonce[half:] + '\n').encode())))
out.close()
PY
}
run_case() {  # $1=case -> verdict via the SAME evaluator
    local rd="$t/run-$1"; mkdir -p "$rd"
    mkpcap "$rd/pcap.pcap" "$1"
    printf 'net-probe: CONNECT_OK\nnet-probe: WRITE_OK 22 bytes\n' >"$rd/serial.log"
    printf '%s\n' "$nonce" >"$rd/host-recv.txt"
    evaluate_run "$rd" "$nonce" 4766
}
want "good capture: guest->host exact nonce -> FULL"        FULL "$(run_case good)"
want "padding beyond total_length cannot counterfeit"      REJECT "$(run_case padded)"
want "non-TCP payload is refused"                          REJECT "$(run_case nontcp)"
want "truncated capture is refused"                        REJECT "$(run_case truncated)"
want "fragmented capture is refused"                       REJECT "$(run_case fragment)"
want "app-protocol payload nonce+LF is FULL"            FULL "$(run_case good)"
want "affixed payload prefix<n>suffix refused"              REJECT "$(run_case affix)"
want "nonce split across packets conservatively refused"   REJECT "$(run_case split)"

# The production evaluator requires WRITE_OK even when READ_ACK is present.
rd="$t/run-readack-only"; mkdir -p "$rd"
mkpcap "$rd/pcap.pcap" good
printf '%s\n' "$nonce" >"$rd/host-recv.txt"
printf 'net-probe: start pid_nonce=test\nnet-probe: READ_ACK match\n' >"$rd/serial.log"
want "READ_ACK cannot replace required WRITE_OK" REJECT_MISSING_WRITE_OK "$(evaluate_run "$rd" "$nonce" 4766)"

# ------------------------------------------------------- exact-match negatives --
mkdir -p "$t/ex"
printf '%s\n' "$nonce" >"$t/pay-exact"
printf 'prefix-%s-suffix\n' "$nonce" >"$t/pay-affix"
printf 'prefix-%s-suffix\n' "$nonce" >"$t/recv-affix"
printf '%s extra\n' "$nonce" >"$t/recv-extra"
want "payload affix prefix<n>suffix refused"  REJECT "$(accept "$nonce" "$t/pay-exact" "$t/pay-affix" write_ok)"
want "peer affix line refused"                REJECT "$(accept "$nonce" "$t/recv-affix" "$t/pay-exact" write_ok)"
want "peer line with trailing junk refused"   REJECT "$(accept "$nonce" "$t/recv-extra" "$t/pay-exact" write_ok)"
want "exact peer + exact payload -> FULL"     FULL "$(accept "$nonce" "$t/pay-exact" "$t/pay-exact" write_ok)"

# --------------------------------------------------------- acceptance fixture --
printf 'net-probe: CONNECT_START\nnet-probe: CONNECT_OK\nnet-probe: WRITE_OK 22 bytes\n' >"$t/serial-write"
printf 'net-probe: CONNECT_START\nnet-probe: CONNECT_OK\n' >"$t/serial-handshake"
printf 'ERROR - READ_BLOCK: INCORRECT HASH\nSETUP PANIC: Failed to find usr: I/O error\n' >"$t/serial-image"
printf 'net-probe: page fault at EL0 PC 0x4012ab\n' >"$t/serial-crash"
: >"$t/recv-empty"
printf '%s\n' 'e1np-from-a-previous-run' >"$t/recv-stale"
want "console-only WRITE_OK is not payload proof"  REJECT "$(accept "$nonce" "$t/recv-empty" "$t/pay-exact" "$(classify "$t/serial-write")")"
want "stale nonce at the peer is refused"          REJECT "$(accept "$nonce" "$t/recv-stale" "$t/pay-exact" write_ok)"
want "empty/default nonce is refused"              REJECT_EMPTY_NONCE "$(accept 'e1np-missing' "$t/pay-exact" "$t/pay-exact" write_ok)"
want "handshake alone is PARTIAL, never success"   PARTIAL "$(accept "$nonce" "$t/recv-empty" "$t/recv-empty" "$(classify "$t/serial-handshake")")"
want "a broken image is not a transport result"    REJECT_IMAGE_INTEGRITY "$(accept "$nonce" "$t/recv-empty" "$t/recv-empty" "$(classify "$t/serial-image")")"
want "an EL0 fault is an app fault, not transport" REJECT_APP_CRASH "$(accept "$nonce" "$t/recv-empty" "$t/recv-empty" "$(classify "$t/serial-crash")")"

# Boot-service faults cannot be promoted to a native application crash.
printf 'kernel::exception:ERROR -- FATAL: unrelated startup fault\nSP_EL0: 123\nredox login:\n' >"$t/serial-boot-login"
want "boot faults before probe start remain login-only" login_only "$(classify "$t/serial-boot-login")"
printf 'kernel::exception:ERROR -- FATAL: old boot fault\nnet-probe: start pid_nonce=test\nnet-probe: WRITE_OK 22 bytes\n' >"$t/serial-boot-then-probe"
want "old boot fault does not poison later native write" write_ok "$(classify "$t/serial-boot-then-probe")"

# ------------------------------------------------- real loopback peer + ACK ----
run_timeout=40; validate_env
start_peer "$t" "$nonce" "$smoke_port"
sleep 1
ack=$(printf '%s\n' "$nonce" | timeout 10 python3 -c '
import socket, sys
s = socket.create_connection(("127.0.0.1", int(sys.argv[1])), timeout=5)
s.sendall(sys.argv[2].encode() + b"\n")
print(s.recv(128).decode().strip())' "$smoke_port" "$nonce")
want "peer smoke: client receives exact ACK"   "ACK-$nonce" "$ack"
want "peer smoke: recv file exact nonce line"  "$nonce" "$(head -n1 "$t/host-recv.txt" 2>/dev/null)"
reap_group "$PEER_PGID"; PEER_PGID=""

# ------------------------------------------------------ validation + verdict --
bad=0
for v in abc 200 10 0; do ( run_timeout=$v validate_env ) 2>/dev/null || bad=$((bad+1)); done
want "invalid RUN_TIMEOUTs rejected (4/4)" "4" "$bad"
rc=0; ( [[ "$(accept "$nonce" "$t/none" "$t/none" write_ok)" == FULL ]] ) || rc=$?
want "no-FULL verdict exits nonzero" "nonzero" "$([[ $rc -ne 0 ]] && echo nonzero || echo zero)"

# Writer may be running even though its FUSE mount never became ready.
setsid sleep 301 & rfs_pid=$!
writer=$rfs_pid
sleep 0.2
cleanup
want "unmounted image writer is reaped" "dead" "$(kill -0 "$writer" 2>/dev/null && echo alive || echo dead)"

# ------------------------------------- TERM-resistant child is still reaped ---
setsid bash -c 'trap "" TERM; while :; do sleep 1; done' >/dev/null 2>&1 &
resist=$!
sleep 1
reap_group "$resist"
want "TERM-resistant group reaped via bounded KILL" "dead" "$(kill -0 "$resist" 2>/dev/null && echo alive || echo dead)"

# --------------------- real-script interruption: no next attempt after TERM ----
cat >"$t/qemu-stub" <<'STUB'
#!/usr/bin/env bash
trap "" TERM
ps -o pgid= -p $$ > "$(dirname "$0")/stub.pgid"
sleep 301
STUB
chmod +x "$t/qemu-stub"
before2=$(find "$out" -maxdepth 1 -type d -name 'run-2-*' | wc -l)
QEMU="$t/qemu-stub" SKIP_STAGE=1 RUN_TIMEOUT=40 START_ATTEMPT=1 "$main" boot >"$t/int.log" 2>&1 &
sp=$!
sleep 28
kill -TERM "$sp"
rc=0; wait "$sp" || rc=$?
want "TERM mid-attempt exits 143" "143" "$rc"
after2=$(find "$out" -maxdepth 1 -type d -name 'run-2-*' | wc -l)
want "no attempt-2 started after TERM" "$before2" "$after2"
owned_alive=$(python3 - "$t/stub.pgid" <<'PYOWN'
import pathlib,sys
pgid=int(pathlib.Path(sys.argv[1]).read_text())
count=0
for p in pathlib.Path('/proc').glob('[0-9]*/stat'):
    try:
        tail=p.read_text().rsplit(')',1)[1].split()
        count += int(int(tail[2]) == pgid and tail[0] != 'Z')
    except (FileNotFoundError, ProcessLookupError): pass
print(count)
PYOWN
)
want "no owned stub group remains after TERM" "0" "$owned_alive"

printf '\nhost-repair selftest: %d passed, %d failed\n' "$pass" "$fail"
(( fail == 0 ))
