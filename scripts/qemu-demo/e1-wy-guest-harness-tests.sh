#!/usr/bin/env bash
# HOST-ONLY tests for e1-wy-guest-harness.sh. NO VM, NO real images, NO mounts,
# NO builds, NO downloads. The host fixture and the pcap verifier are tested
# against REAL sockets and the REAL host-built wyoming-probe binary (positional
# CLI), exchanging REAL Wyoming frames via the pinned Python reference. The
# full-run fake uses WG_FAKE_TEST=1 + WG_FAKE_ROOT redirects ONLY (production
# pins unchangeable in run mode), and its stub guest drives the real host
# binary through a real relay so the pcap carries genuine ping/pong bytes.
set -uo pipefail
repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
S=$repo/scripts/qemu-demo/e1-wy-guest-harness.sh
FIX=$repo/scripts/qemu-demo/e1-wy-guest-harness-fixture.py
PCV=$repo/scripts/qemu-demo/e1-wy-guest-harness-pcap.py
HOSTBIN=$HOME/Developer/experiments/redox-oneplus6-wy/out/wyoming-probe/host/release/wyoming-probe
HOSTBIN_SHA=099629ead454ef305b40d5e747f3a7c9a85db22d47036b54f6d04ee7d7325de7
PYREF=$HOME/Developer/forks/op6-refs/wyoming
PYMETA=$repo/crates/wyoming/tests/pyref_meta
T=$(mktemp -d "${TMPDIR:-/tmp}/wy-guest-tests.XXXXXX")
trap 'rm -rf "$T"' EXIT
pass=0; fail=0
ok()  { printf 'PASS: %s\n' "$1"; pass=$((pass+1)); }
bad() { printf 'FAIL: %s — expected %s got %s\n' "$1" "$2" "$3"; fail=$((fail+1)); }
chk() { [[ "$3" == "$2" ]] && ok "$1" || bad "$1" "$2" "$3"; }
sha() { sha256sum "$1" | cut -d ' ' -f1; }
NOW=$(date +%s)
[[ -x "$HOSTBIN" && "$(sha "$HOSTBIN")" == "$HOSTBIN_SHA" ]] || { echo "BLOCKER: pinned host wyoming-probe missing/changed" >&2; exit 3; }

# ---- fake-mode image + seams under WG_FAKE_ROOT ------------------------------
IMG=$T/staged.iso; head -c 4096 /dev/zero >"$IMG"
touch "$T/dtb" "$T/firmware"
IMG_SHA=$(sha "$IMG")
export WG_FAKE_TEST=1 WG_FAKE_ROOT=$T

# ---- stub qemu: REAL prompts + runs the REAL host binary through a relay ----
cat >"$T/relay.py" <<'EOF'
import json, select, socket, sys, argparse
ap = argparse.ArgumentParser(); ap.add_argument("--listen", type=int)
ap.add_argument("--fwd", type=int); ap.add_argument("--log")
a = ap.parse_args()
srv = socket.socket(); srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
srv.bind(("127.0.0.1", a.listen)); srv.listen(1); srv.settimeout(15)
conn, _ = srv.accept()
up = socket.create_connection(("127.0.0.1", a.fwd), 5)
log = open(a.log, "a")
def pump(src, dst, tag):
    while True:
        try: b = src.recv(65536)
        except OSError: break
        if not b: break
        log.write(json.dumps({"dir": tag, "hex": b.hex()}) + "\n"); log.flush()
        if tag == "up":
            try: dst.sendall(b)
            except OSError: break
        # downlink from the fixture is relayed too
        if tag == "down":
            try: dst.sendall(b)
            except OSError: break
import threading
t1 = threading.Thread(target=pump, args=(conn, up, "up"))
t2 = threading.Thread(target=pump, args=(up, conn, "down"))
t1.start(); t2.start(); t1.join(timeout=20); t2.join(timeout=20)
EOF
cat >"$T/mkpcap.py" <<'EOF'
import json, struct, argparse
ap = argparse.ArgumentParser(); ap.add_argument("--log"); ap.add_argument("--out")
ap.add_argument("--port", type=int); ap.add_argument("--leg", type=int, default=0)
a = ap.parse_args()
g = bytes(int(x) for x in "10.0.2.15".split(".")); h = bytes(int(x) for x in "10.0.2.2".split("."))
def frame(src, dst, sport, dport, payload, seq):
    ip = struct.pack(">BBHHHBBH4s4s", 0x45, 0, 40 + len(payload), 1, 0, 64, 6, 0, src, dst)
    tcp = struct.pack(">HHIIBBHHH", sport, dport, seq, 0, (5 << 4), 0x18, 65535, 0, 0)
    return b"\x02\x00\x00\x00\x00\x01" * 2 + b"\x08\x00" + ip + tcp + payload
def rec(p): return struct.pack("<IIII", 0, 0, len(p), len(p)) + p
d = struct.pack("<IHHiIII", 0xa1b2c3d4, 2, 4, 0, 0, 65535, 1)
su = sd = 1000
for i, line in enumerate(open(a.log)):
    e = json.loads(line); p = bytes.fromhex(e["hex"])
    if e["dir"] == "up":
        d += rec(frame(g, h, 40000 + a.leg, a.port, p, su)); su += len(p)
    else:
        d += rec(frame(h, g, a.port, 40000 + a.leg, p, sd)); sd += len(p)
open(a.out, "ab" if a.leg else "wb").write(d if not a.leg else d[24:])
EOF
cat >"$T/qemu" <<'EOF'
#!/usr/bin/env bash
# fake guest console: real prompt sequence; ECHOES every command like a real
# tty, emits command output FRAGMENTED and SPLITS prompts across writes — the
# harness must still anchor correctly and never match a partial PROBE line.
pcap=""; for a in "$@"; do case $a in *filter-dump*) pcap=${a##*file=} ;; esac; done
printf 'Redox bootloader\nPress e to edit boot environment\n'
IFS= read -r -d $'\r' l1; IFS= read -r -d $'\r' l2
printf 'redox login: '; IFS= read -r -d $'\r' u
printf 'Password: '; IFS= read -r -d $'\r' p
printf 'root:~'; sleep 0.15; printf '# '
leg=0
while IFS= read -r -d $'\r' cmd; do
    printf '%s\r\n' "$cmd"
    case $cmd in
        echo\ WYARM) printf 'WYAR'; sleep 0.15; printf 'M\r\n'; printf 'root:~'; sleep 0.15; printf '# ' ;;
        /usr/bin/wyoming-probe\ *)
            set -- $cmd; addr=$2; nonce=$3; dl=$4; port=${addr##*:}
            rp=$(( 21000 + leg * 7 + RANDOM % 100 ))
            python3 "$WG_T/relay.py" --listen "$rp" --fwd "$port" --log "$WG_T/frames-$leg.jsonl" & rl=$!
            sleep 0.3
            out=$(timeout "$(( dl + 5 ))" "$WG_HOSTBIN" "127.0.0.1:$rp" "$nonce" "$dl" 2>&1)
            wait "$rl" 2>/dev/null || true
            python3 "$WG_T/mkpcap.py" --log "$WG_T/frames-$leg.jsonl" --out "$pcap" --port "$port" --leg "$leg"
            half=$(( ${#out} / 2 ))
            printf '%s' "${out:0:half}"; sleep 0.15; printf '%s\r\n' "${out:half}"
            leg=$(( leg + 1 ))
            printf 'root:~'; sleep 0.15; printf '# ' ;;
        *) printf 'root:~# ' ;;
    esac
done
sleep 300 &
exit 0
EOF
chmod +x "$T/qemu"
pass=0
run() {  # fake-mode invocation: every redirect under WG_FAKE_ROOT
    WG_OUT=${WG_OUT:-$T/out} WG_IMAGE=${WG_IMAGE:-$IMG} WG_QEMU_BIN=${WG_QEMU_BIN:-$T/qemu} \
    WG_DTB=${WG_DTB:-$T/dtb} WG_FIRMWARE=${WG_FIRMWARE:-$T/firmware} \
    WG_ORIG_ISO=${WG_ORIG_ISO:-$T/absent-orig.iso} WG_BAD_ISO=${WG_BAD_ISO:-$T/absent-bad.iso} \
    WG_ARM_B_ISO=${WG_ARM_B_ISO:-$T/absent-armB.iso} WG_STAGED_ISO_SHA=$IMG_SHA WG_FAKE_ROOT=$T \
    WG_VM_BOUND=${WG_VM_BOUND:-60} WG_PROBE_DEADLINE=${WG_PROBE_DEADLINE:-3} WG_GRACE=${WG_GRACE:-5} \
    WG_CODEX_PHASE=${WG_CODEX_PHASE:-} WG_T=$T WG_HOSTBIN=$HOSTBIN \
    bash "$S" "$@"
}
ph() { printf 'wy-%s-%s' "$NOW" "$1"; }
chk() { [[ "$3" == "$2" ]] && ok "$1" || bad "$1" "$2" "$3"; }
has() { grep -q "$2" "$T/out/$1/results.tsv" 2>/dev/null; }

# T1-T5 guards
run dry-run --image "$IMG" >/dev/null 2>&1; chk "T1 missing phase -> 2" 2 $?
run dry-run --phase "net-x-123" --image "$IMG" >/dev/null 2>&1; chk "T2 bad phase ns -> 2" 2 $?
run run --phase "$(ph t3)" --image "$IMG" >/dev/null 2>&1; chk "T3 no --approve -> 2" 2 $?
run run --phase "$(ph t4)" --image "$IMG" --approve >/dev/null 2>&1; chk "T2 no admission token -> 2" 2 $?
run run --phase "$(ph t3)" --image "$IMG" --approve --boot-cmd "true" >/dev/null 2>&1; chk "T3 --boot-cmd -> 2" 2 $?
WG_FAKE_TEST=0 WG_FAKE_ROOT= run run --phase "$(ph t4)" --image "$IMG" --approve >/dev/null 2>&1
chk "T4 image override outside pins -> 2" 2 $?
WG_REF_HEAD=incorrect run dry-run --phase "$(ph refpin)" --image "$IMG" >"$T/refpin.out" 2>&1
chk "T4b changed reference HEAD refused before claim" 1 $?
[[ ! -e "$T/out/$(ph refpin)" ]] && ok "T4c reference refusal has no claim" || bad "T4c claim" absent present
# T5 dry-run + consumed claim
run dry-run --phase "$(ph t5)" --image "$IMG" >/dev/null 2>&1; chk "T5 dry-run -> 0" 0 $?
run dry-run --phase "$(ph t5)" --image "$IMG" >/dev/null 2>&1; chk "T5b consumed claim -> 4" 4 $?
# T6 protected mismatch fails closed before any launch
printf 'x' >"$T/armB.iso"
WG_ARM_B_ISO=$T/armB.iso WG_CODEX_PHASE=wy-$NOW-t6 run run --phase "$(ph t6)" --image "$IMG" --approve >/dev/null 2>&1
chk "T6 protected mismatch -> 1" 1 $?
rm -f "$T/armB.iso"

# ---- T7 REAL fixture + REAL host binary: positional roundtrip --------------
nonce=wy-test-$NOW-$RANDOM
PYTHONPATH="$PYREF:$PYMETA" python3 "$FIX" --receipt "$T/r1.json" --deadline $(( $(date +%s) + 20 )) >"$T/ready1.json" 2>"$T/fix1.err" & FX=$!
for _ in $(seq 1 40); do [[ -s "$T/ready1.json" ]] && break; sleep 0.25; done
AP=$(python3 -c "import json;print(json.load(open('$T/ready1.json'))['answer_port'])" 2>/dev/null)
SP=$(python3 -c "import json;print(json.load(open('$T/ready1.json'))['silent_port'])" 2>/dev/null)
[[ -n "$AP" && -n "$SP" ]] && ok "T7a fixture readiness (ports $AP/$SP)" || bad "T7a readiness" "ports" "none"
out=$("$HOSTBIN" "127.0.0.1:$AP" "$nonce" 5 2>&1); rc=$?
chk "T7b REAL host binary positional -> 0" 0 "$rc"
grep -q "PROBE PASS addr=127.0.0.1:$AP nonce=$nonce" <<<"$out" && ok "T7c exact anchored PASS line" || bad "T7c PASS line" "anchored" "$out"
out=$("$HOSTBIN" --addr "127.0.0.1:$AP" --nonce x 2>&1); rc=$?
chk "T7d flag form rejected by REAL CLI -> 2" 2 "$rc"
for _ in $(seq 1 1); do :; done

# ---- T8 REAL silent peer: actual timeout -----------------------------------
t0=$(date +%s)
out=$("$HOSTBIN" "127.0.0.1:$SP" "$nonce-s" 3 2>&1); rc=$?
el=$(( $(date +%s) - t0 ))
chk "T8a silent peer -> 1" 1 "$rc"
grep -q "PROBE FAIL describe: socket read timed out (silent peer)" <<<"$out" && ok "T8b exact silent-peer FAIL line" || bad "T8b line" "timeout msg" "$out"
(( el >= 2 && el <= 9 )) && ok "T8c timeout measured near deadline (${el}s)" || bad "T8c elapsed" "2..9" "$el"
kill -TERM "$FX" 2>/dev/null
for _ in $(seq 1 40); do [[ -f "$T/r1.json" ]] && break; sleep 0.25; done
wait "$FX" 2>/dev/null || true
python3 - "$T/r1.json" <<'PR'
import json, sys
d = json.load(open(sys.argv[1]))
assert len(d.get("silent_accepts", [])) >= 1 and d.get("silent_writes", 1) == 0
assert d.get("silent_events", [{}])[0].get("type") == "describe"
PR
chk "T8d silent receipt: accept+describe read, zero writes" 0 $?
ev=$(python3 - "$T/r1.json" "$nonce" <<'PR'
import json, sys
d = json.load(open(sys.argv[1])); c = d.get("answer_counts", {})
print("desc" if c.get("describe") else "-", "info" if c.get("info") else "-",
      "ping" if c.get("ping") else "-", d.get("answer_writes", 0),
      "nonce" if d.get("nonce_seen") == sys.argv[2] else "-", "eof" if d.get("answer_eof") else "-")
PR
)
chk "T7e receipt: describe/info/ping + writes + nonce + eof" "desc info ping 3 nonce eof" "$ev"


# ---- T9 pcap verifier: REAL Wyoming frames, exact tuple, negatives ----------
python3 - "$T" "$AP" "$nonce" <<'MK'
import io, json, os, struct, sys
sys.path.insert(0, os.environ.get("PYMETA", ""))
MK
gen() {  # $1=out $2=nonce $3=mode
    PYTHONPATH="$PYREF:$PYMETA" python3 - "$1" "$2" "$3" "$AP" <<'MK'
import io, struct, sys
from wyoming.event import write_event
from wyoming.ping import Ping, Pong
out, nonce, mode, port = sys.argv[1], sys.argv[2], sys.argv[3], int(sys.argv[4])
b = io.BytesIO(); write_event(Ping(text=nonce).event(), b); up = b.getvalue()
b = io.BytesIO(); write_event(Pong(text=nonce if mode != "wrongnonce" else "OTHER").event(), b); dn = b.getvalue()
if mode == "missingpong": dn = b""
if mode == "noncepayload":
    dn = ('{"type": "pong", "text": "OTHER", "data_length": %d}\n' % len(nonce)).encode() + nonce.encode()
if mode == "truncheader": up = b'{"type": "ping", "text": "no'
g = bytes(int(x) for x in "10.0.2.15".split("."))
g2 = bytes(int(x) for x in "10.0.2.16".split("."))
h = bytes(int(x) for x in "10.0.2.2".split("."))
dport = port if mode != "wrongport" else port + 1
gsrc = g if mode != "wrongclientip" else g2
cdport = 40000 if mode != "wrongclientport" else 40001
def frame(src, dst, sp, dp, pl, seq):
    ip = struct.pack(">BBHHHBBH4s4s", 0x45, 0, 40 + len(pl), 1, 0, 64, 6, 0, src, dst)
    tcp = struct.pack(">HHIIBBHHH", sp, dp, seq, 0, (5 << 4), 0x18, 65535, 0, 0)
    return b"\x02\x00\x00\x00\x00\x01" * 2 + b"\x08\x00" + ip + tcp + pl
def rec(p): return struct.pack("<IIII", 0, 0, len(p), len(p)) + p
d = struct.pack("<IHHiIII", 0xa1b2c3d4, 2, 4, 0, 0, 65535, 1)
if up: d += rec(frame(gsrc, h, 40000, dport, up, 1000))
if mode == "retransmit": d += rec(frame(gsrc, h, 40000, dport, up + b"X", 1000))
if mode == "gap": d += rec(frame(gsrc, h, 40000, dport, up, 1000 + len(up) + 7))
if dn: d += rec(frame(h, g, port, cdport, dn, 2000))
if mode == "tailheader": d += struct.pack("<III", 0, 0, 50)   # partial record header
if mode == "truncated": d = d[:-8]
open(out, "wb").write(d)
MK
}
gen "$T/p-good.pcap" "$nonce" good
python3 "$PCV" "$T/p-good.pcap" "$AP" "$nonce" >/dev/null 2>&1; chk "T9a real frames exact tuple -> 0" 0 $?
gen "$T/p-port.pcap" "$nonce" wrongport
python3 "$PCV" "$T/p-port.pcap" "$AP" "$nonce" >/dev/null 2>&1; chk "T9b unrelated port -> 1" 1 $?
gen "$T/p-nopong.pcap" "$nonce" missingpong
python3 "$PCV" "$T/p-nopong.pcap" "$AP" "$nonce" >/dev/null 2>&1; chk "T9c missing pong -> 1" 1 $?
gen "$T/p-wrong.pcap" "$nonce" wrongnonce
python3 "$PCV" "$T/p-wrong.pcap" "$AP" "$nonce" >/dev/null 2>&1; chk "T9d pong nonce mismatch -> 1" 1 $?
gen "$T/p-trunc.pcap" "$nonce" truncated
python3 "$PCV" "$T/p-trunc.pcap" "$AP" "$nonce" >/dev/null 2>&1; chk "T9e truncated record -> 2" 2 $?
gen "$T/p-cport.pcap" "$nonce" wrongclientport
python3 "$PCV" "$T/p-cport.pcap" "$AP" "$nonce" >/dev/null 2>&1; chk "T9f pong on wrong client port -> 1" 1 $?
gen "$T/p-cip.pcap" "$nonce" wrongclientip
python3 "$PCV" "$T/p-cip.pcap" "$AP" "$nonce" >/dev/null 2>&1; chk "T9g ping from wrong client IP -> 1" 1 $?
gen "$T/p-npl.pcap" "$nonce" noncepayload
python3 "$PCV" "$T/p-npl.pcap" "$AP" "$nonce" >/dev/null 2>&1; chk "T9h nonce only in unrelated payload -> 1" 1 $?
gen "$T/p-thdr.pcap" "$nonce" truncheader
python3 "$PCV" "$T/p-thdr.pcap" "$AP" "$nonce" >/dev/null 2>&1; chk "T9i truncated wyoming header -> 2" 2 $?
gen "$T/p-retr.pcap" "$nonce" retransmit
python3 "$PCV" "$T/p-retr.pcap" "$AP" "$nonce" >/dev/null 2>&1; chk "T9j conflicting retransmit -> 2" 2 $?
gen "$T/p-gap.pcap" "$nonce" gap
python3 "$PCV" "$T/p-gap.pcap" "$AP" "$nonce" >/dev/null 2>&1; chk "T9k seq gap -> 2" 2 $?
gen "$T/p-tail.pcap" "$nonce" tailheader
python3 "$PCV" "$T/p-tail.pcap" "$AP" "$nonce" >/dev/null 2>&1; chk "T9l trailing truncated record header -> 2" 2 $?

# ---- T10 full fake run: stub guest + REAL host binary + REAL fixture -------
rm -f "$T/frames.jsonl"
WG_QEMU_BIN=$T/qemu WG_HOSTBIN=$HOSTBIN WG_T=$T \
WG_ORIG_ISO=$T/absent.iso WG_BAD_ISO=$T/absent.iso WG_ARM_B_ISO=$T/absent.iso \
WG_STAGED_ISO_SHA=$IMG_SHA WG_VM_BOUND=60 WG_PROBE_DEADLINE=3 WG_GRACE=5 \
WG_CODEX_PHASE=wy-$NOW-t10 \
run run --phase "$(ph t10)" --image "$IMG" --approve >"$T/t10.out" 2>&1
rc=$?
chk "T10 full fake run -> 0" 0 "$rc"
grep -q 'verdict	GUEST_HARNESS_PASS' "$T/out/wy-$NOW-t10/results.tsv" 2>/dev/null && ok "T10b GUEST_HARNESS_PASS" || { printf 'FAIL: T10b verdict: %s\n' "$(grep -m1 verdict "$T/out/wy-$NOW-t10/results.tsv" 2>/dev/null)"; fail=$((fail+1)); }
grep -q 'fixture.nonce_seen	yes' "$T/out/wy-$NOW-t10/results.tsv" 2>/dev/null && ok "T10c nonce seen at REAL fixture" || { printf 'FAIL: T10c\n'; fail=$((fail+1)); }
grep -q 'pcap.ping_on_exact_port	yes' "$T/out/wy-$NOW-t10/results.tsv" 2>/dev/null && grep -q 'pcap.pong_on_exact_port	yes' "$T/out/wy-$NOW-t10/results.tsv" 2>/dev/null && ok "T10d pcap nonce ping+pong on exact ports" || { printf 'FAIL: T10d pcap keys: %s\n' "$(grep pcap "$T/out/wy-$NOW-t10/results.tsv" 2>/dev/null | tr '\n' ';')"; fail=$((fail+1)); }
grep -q 'fixture.silent_writes	0' "$T/out/wy-$NOW-t10/results.tsv" 2>/dev/null && ok "T10e silent peer wrote nothing" || { printf 'FAIL: T10e\n'; fail=$((fail+1)); }
grep -q 'orphan_pgids	0' "$T/out/wy-$NOW-t10/results.tsv" 2>/dev/null && ok "T10f owned PGIDs reaped" || { printf 'FAIL: T10f orphans\n'; fail=$((fail+1)); }

# ---- T11 silent-guest negative: stub never answers probe -> FAIL, no pass --
cat >"$T/qemu-silent" <<'EOF'
#!/usr/bin/env bash
printf 'Redox bootloader\nPress e to edit boot environment\n'
IFS= read -r -d $'\r' l1; IFS= read -r -d $'\r' l2
printf 'redox login: '; IFS= read -r -d $'\r' u
printf 'Password: '; IFS= read -r -d $'\r' p
printf 'root:~# '
while IFS= read -r -d $'\r' cmd; do :; done
EOF
chmod +x "$T/qemu-silent"
WG_QEMU_BIN=$T/qemu-silent WG_HOSTBIN=$HOSTBIN WG_T=$T \
WG_ORIG_ISO=$T/absent.iso WG_BAD_ISO=$T/absent.iso WG_ARM_B_ISO=$T/absent.iso \
WG_STAGED_ISO_SHA=$IMG_SHA WG_VM_BOUND=45 WG_PROBE_DEADLINE=3 WG_GRACE=3 \
WG_CODEX_PHASE=wy-$NOW-t11 \
run run --phase "$(ph t11)" --image "$IMG" --approve >"$T/t11.out" 2>&1
chk "T11 guest never answers -> 5" 5 $?
grep -q 'GUEST_HARNESS_FAIL\|LEG1_ARM_WAIT' "$T/out/wy-$NOW-t11/results.tsv" 2>/dev/null && ok "T11b fail-closed verdict" || { printf 'FAIL: T11b\n'; fail=$((fail+1)); }

# ---- T12 sentinel: protected file CREATED/mutated mid-run -> nonzero --------
sed '/wyoming-probe\\ \*)/a : > "$WG_ORIG_ISO"' "$T/qemu" > "$T/qemu-mutate"
chmod +x "$T/qemu-mutate"
rm -f "$T/sentinel-orig.iso"
WG_QEMU_BIN=$T/qemu-mutate WG_ORIG_ISO=$T/sentinel-orig.iso \
WG_HOSTBIN=$HOSTBIN WG_T=$T WG_ARM_B_ISO=$T/absent.iso WG_BAD_ISO=$T/absent.iso \
WG_STAGED_ISO_SHA=$IMG_SHA WG_VM_BOUND=60 WG_PROBE_DEADLINE=3 WG_GRACE=5 \
WG_CODEX_PHASE=wy-$NOW-t12 \
run run --phase "$(ph t12)" --image "$IMG" --approve >"$T/t12.out" 2>&1
chk "T12 protected mutated mid-run -> 5" 5 $?
grep -q 'protected.orig_iso.after.sha	MISMATCH' "$T/out/wy-$NOW-t12/results.tsv" 2>/dev/null \
    && grep -q 'protected.after_fail	1' "$T/out/wy-$NOW-t12/results.tsv" 2>/dev/null \
    && ok "T12b after-hash failure persisted + verdict fail" || { printf 'FAIL: T12b\n'; fail=$((fail+1)); }

# ---- T13 sentinel: protected file DISAPPEARS mid-run -> nonzero -------------
printf 'armB-content' >"$T/prot-armB.iso"; AB_SHA=$(sha "$T/prot-armB.iso")
sed '/wyoming-probe\\ \*)/a rm -f "$WG_ARM_B_ISO"' "$T/qemu" > "$T/qemu-vanish"
chmod +x "$T/qemu-vanish"
WG_QEMU_BIN=$T/qemu-vanish WG_ARM_B_ISO=$T/prot-armB.iso WG_ARM_B_ISO_SHA=$AB_SHA \
WG_HOSTBIN=$HOSTBIN WG_T=$T WG_ORIG_ISO=$T/absent.iso WG_BAD_ISO=$T/absent.iso \
WG_STAGED_ISO_SHA=$IMG_SHA WG_VM_BOUND=60 WG_PROBE_DEADLINE=3 WG_GRACE=5 \
WG_CODEX_PHASE=wy-$NOW-t13 \
run run --phase "$(ph t13)" --image "$IMG" --approve >"$T/t13.out" 2>&1
chk "T13 protected disappears mid-run -> 5" 5 $?
grep -q 'protected.armB.after	DISAPPEARED' "$T/out/wy-$NOW-t13/results.tsv" 2>/dev/null \
    && ok "T13b DISAPPEARED recorded" || { printf 'FAIL: T13b\n'; fail=$((fail+1)); }

# ---- T14 reap: stub child IGNORES SIGTERM; KILL escalation must clear PG ----
sed "s/^sleep 300 &\$/trap '' TERM; sleep 300 \&/" "$T/qemu" > "$T/qemu-stubborn"
chmod +x "$T/qemu-stubborn"
WG_QEMU_BIN=$T/qemu-stubborn WG_HOSTBIN=$HOSTBIN WG_T=$T \
WG_ORIG_ISO=$T/absent.iso WG_BAD_ISO=$T/absent.iso WG_ARM_B_ISO=$T/absent.iso \
WG_STAGED_ISO_SHA=$IMG_SHA WG_VM_BOUND=60 WG_PROBE_DEADLINE=3 WG_GRACE=5 \
WG_CODEX_PHASE=wy-$NOW-t14 \
run run --phase "$(ph t14)" --image "$IMG" --approve >"$T/t14.out" 2>&1
chk "T14a TERM-ignoring child still reaped -> 0" 0 $?
R14=$T/out/wy-$NOW-t14/results.tsv
pg=$(sed -n 's/^qemu.pg\t//p' "$R14" 2>/dev/null)
[[ -n "$pg" ]] && ! kill -0 -- "-$pg" 2>/dev/null && ok "T14b owned PGID $pg fully gone" || { printf 'FAIL: T14b pg=%s\n' "$pg"; fail=$((fail+1)); }
grep -q 'cleanup.reap_failed_pg' "$R14" 2>/dev/null && { printf 'FAIL: T14c reap_failed recorded\n'; fail=$((fail+1)); } || ok "T14c no reap failure recorded"
grep -q 'orphan_pgids	0' "$R14" 2>/dev/null && ok "T14d group-based orphan check 0" || { printf 'FAIL: T14d\n'; fail=$((fail+1)); }

# ---- T15 arbitrary fast PROBE FAIL never passes (time bound, both sides) ----
cat >"$T/qemu-badfail" <<'EOF'
#!/usr/bin/env bash
# leg1 real; leg2 prints the EXACT silent-peer FAIL line INSTANTLY (no socket).
pcap=""; for a in "$@"; do case $a in *filter-dump*) pcap=${a##*file=} ;; esac; done
pcap=""; for a in "$@"; do case $a in *filter-dump*) pcap=${a##*file=} ;; esac; done
printf 'Redox bootloader\nPress e to edit boot environment\n'
IFS= read -r -d $'\r' l1; IFS= read -r -d $'\r' l2
printf 'redox login: '; IFS= read -r -d $'\r' u
printf 'Password: '; IFS= read -r -d $'\r' p
printf 'root:~# '
leg=0
while IFS= read -r -d $'\r' cmd; do
    printf '%s\r\n' "$cmd"
    case $cmd in
        echo\ WYARM) printf 'WYARM\r\nroot:~# ' ;;
        /usr/bin/wyoming-probe\ *)
            if (( leg == 0 )); then
                set -- $cmd; addr=$2; nonce=$3; dl=$4; port=${addr##*:}
                rp=$(( 21000 + RANDOM % 100 ))
                python3 "$WG_T/relay.py" --listen "$rp" --fwd "$port" --log "$WG_T/frames-bf.jsonl" & rl=$!
                sleep 0.3
                out=$(timeout "$(( dl + 5 ))" "$WG_HOSTBIN" "127.0.0.1:$rp" "$nonce" "$dl" 2>&1)
                wait "$rl" 2>/dev/null || true
                python3 "$WG_T/mkpcap.py" --log "$WG_T/frames-bf.jsonl" --out "$pcap" --port "$port" --leg 0
                printf '%s\r\n' "$out"
            else
                printf 'PROBE FAIL describe: socket read timed out (silent peer)\r\n'
            fi
            leg=$(( leg + 1 ))
            printf 'root:~# ' ;;
        *) printf 'root:~# ' ;;
    esac
done
EOF
chmod +x "$T/qemu-badfail"
WG_QEMU_BIN=$T/qemu-badfail WG_HOSTBIN=$HOSTBIN WG_T=$T \
WG_ORIG_ISO=$T/absent.iso WG_BAD_ISO=$T/absent.iso WG_ARM_B_ISO=$T/absent.iso \
WG_STAGED_ISO_SHA=$IMG_SHA WG_VM_BOUND=45 WG_PROBE_DEADLINE=3 WG_GRACE=3 \
WG_CODEX_PHASE=wy-$NOW-t15 \
run run --phase "$(ph t15)" --image "$IMG" --approve >"$T/t15.out" 2>&1
chk "T15 instant fake FAIL line -> 5 (time bound both sides)" 5 $?
grep -q 'leg2.elapsed_s	[0-2]$' "$T/out/wy-$NOW-t15/results.tsv" 2>/dev/null && ok "T15b fast fake FAIL rejected" || { printf 'FAIL: T15b\n'; fail=$((fail+1)); }

printf '\n== e1-wy-guest-harness suite: %d passed, %d failed ==\n' "$pass" "$fail"
exit $(( fail > 0 ))
