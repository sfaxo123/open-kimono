#!/usr/bin/env bash
# E1-NET-1 — ONE bounded guest->host TCP nonce exchange over the proven
# rtl8139/INTx path, emulator only. Plan: $HOME/artifacts/handoff/
# redox-perplexity/E1-NETWORK-NEXT-GATE.md (Cursor read-only analysis).
#
# Worktree-local COPY of e1-intx-swap.sh with EXACTLY THREE deltas (plan §3):
#   1. packet capture: -object filter-dump,id=e1dump,netdev=e1net,file=<pcap>
#   2. host peer started BEFORE QEMU, loopback-only, bounded:
#      timeout 95 socat -u TCP-LISTEN:$port,bind=127.0.0.1,reuseaddr OPEN:<recv>
#      (slirp maps guest 10.0.2.2:PORT -> host 127.0.0.1:PORT)
#   3. login probe carries a unique per-run nonce via ion redirect:
#      echo $nonce > /scheme/tcp/10.0.2.2:$port
# Everything else — machine, AAVMF, ISO (readonly=on), DTB, rtl8139, console
# Enter nudge, login-probe mechanism, classifier, stop_qemu — is unchanged.
#
# Budget: max 2 boots x 90 s (INTX_ATTEMPTS/INTX_TIMEOUT validated numerically
# before any boot work); peer bounded by timeout 95 s; the EXIT trap kills BOTH
# peer and QEMU. Stop on first success or identical stage+signature twice.
# No pre-check boots; ISO and DTB are hashed before and after the run.
#
# Verdict requires ALL THREE domains: the exact nonce at the host peer (HOST),
# the full ARP->SYN->SYN/ACK->data+nonce sequence in the pcap with addresses
# OBSERVED from the capture, never hard-coded (HOST), and a clean guest serial
# (GUEST). A nonce echoed on the console alone is NOT proof; the recv file is
# written by socat only, never synthesized. This gate claims NO dhcpd lease,
# DNS, internet reachability or phone behaviour. E1-EXP-1 attempts 1-5 and
# intx-ledger.tsv are untouched; every file here is e1net-prefixed.
set -euo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
fork=${REDOX_FORKS:-$HOME/Developer/forks/redox}
image="$fork/redox/build/aarch64/desktop-minimal/e1-live.iso"
tftp="$fork/bootloader-e1/build/tftp-live"
out="$repo/out/integration"
dtb="$out/simplefb-virt.dtb"
nic=${INTX_NIC:-rtl8139}
run_timeout=${INTX_TIMEOUT:-90}
attempts=${INTX_ATTEMPTS:-2}          # E1-NET-1 budget: max 2 boots x 90 s
serial="$out/e1net-serial.txt"
monitor="$out/e1net-monitor.sock"
qlog="$out/e1net-qemu.log"
ledger=${E1NET_LEDGER:-$out/e1net-ledger.tsv}

# Host peer (HOST domain): slirp maps guest 10.0.2.2:PORT -> host 127.0.0.1:PORT.
# The receive file is created and written by socat ALONE — never synthesized.
port=${E1NET_PORT:-47011}
nonce="E1NET-$(date -u +%s)-$RANDOM"  # unique per run; must arrive at the peer

# Console probe commands, '|' separated; the first two are the login credentials.
# Only shell builtins are safe here: /usr/bin/ls dies in an EL0 exception on this image.
# ion `>` redirect to /scheme/tcp/<remote> = connect (netstack tcp.rs:75-76,122-127);
# whether ion's redirect open flags reach the tcp scheme is the one link this boot tests.
IFS='|' read -r -a probe_cmds <<<"${INTX_PROBE_CMDS:-root|password|echo /scheme/netcfg/ifaces/eth0/addr/*|echo $nonce > /scheme/tcp/10.0.2.2:$port|echo E1NET-DONE}"

# ---- startup validation (numeric, budget-capped; runs before ANY boot work) --
[[ "$nic" =~ ^[A-Za-z0-9._-]+$ ]] || {
    printf 'INTX_NIC must match [A-Za-z0-9._-]+ (QEMU device name), got %s\n' "$nic" >&2
    exit 2
}
[[ "$attempts" =~ ^[0-9]+$ ]] && (( attempts >= 1 && attempts <= 4 )) || {
    printf 'INTX_ATTEMPTS must be an integer 1..4 (plan budget: <=4 attempts x <=90 s), got %s\n' "$attempts" >&2
    exit 2
}
[[ "$run_timeout" =~ ^[0-9]+$ ]] && (( run_timeout >= 1 && run_timeout <= 90 )) || {
    printf 'INTX_TIMEOUT must be an integer 1..90 seconds (plan budget: <=4 attempts x <=90 s), got %s\n' "$run_timeout" >&2
    exit 2
}
[[ "$port" =~ ^[0-9]+$ ]] && (( port >= 1024 && port <= 65535 )) || {
    printf 'E1NET_PORT must be an integer 1024..65535 (host peer bind port), got %s\n' "$port" >&2
    exit 2
}

# Failure signatures named by the feasibility plan §3, each with its next action.
sig_names=(msix-panic no-interrupts-at-all no-legacy-interrupts failed-open-irq-file
    failed-create-network-scheme failed-register-network-scheme no-driver-for
    fatal-not-an-svc failed-get-next-event)
sig_regexes=('MSI-X is not implemented' 'no interrupts supported at all'
    'no legacy interrupts supported' 'failed to open IRQ file'
    'failed to create network scheme' 'failed to regitster network scheme'
    'no driver for' 'FATAL: Not an SVC' 'failed to get next event')

strip() { sed 's/\x1b\[[0-9;]*[a-zA-Z]//g; s/\x1b\[=[0-9;]*[a-zA-Z]//g; s/\x1b[()][A-Z0-9]//g; s/\r//g' "$1"; }

# The plan's markers are about the NIC path. The live ISO's virtio-blkd panics in
# the MSI-X stub on every run (recorded in E1-REDOX-SERIAL.txt long before this
# experiment), so the NIC verdict is taken from the serial from the NIC driver's
# spawn onward; panics before that are counted separately, never hidden.
nic_scope() {  # $1 = serial file, $2 = destination file
    if grep -qE 'pcid-spawner: spawn ".*(rtl8139d|e1000d)"' "$1" 2>/dev/null; then
        awk '/pcid-spawner: spawn ".*(rtl8139d|e1000d)"/{f=1} f' "$1" >"$2"
    else
        cat "$1" >"$2"
    fi
}

pre_nic_panics() {  # $1 = serial file -> count of panic lines before the NIC spawn
    awk '/pcid-spawner: spawn ".*(rtl8139d|e1000d)"/{f=1} !f && (/MSI-X is not implemented/ || /FATAL: Not an SVC/)' \
        "$1" 2>/dev/null | wc -l
}

# A "FATAL: Not an SVC" line is a NIC-path failure ONLY when it is not the tail of
# a userspace crash (kernel::context::signal names the dying binary) and not the
# pre-existing virtio-blkd MSI-X abort. Both other sources are counted, never hidden.
fatal_unattributed() {  # $1 = scoped serial file
    local f u v
    f=$(grep -c 'FATAL: Not an SVC' "$1" 2>/dev/null || true)
    u=$(grep -cE 'UNHANDLED EXCEPTION.*NAME [^ ]*' "$1" 2>/dev/null || true)
    v=$(grep -c 'ERROR virtio-blkd' "$1" 2>/dev/null || true)
    printf '%s\n' "$(( f > u + v ? f - u - v : 0 ))"
}

signature() {  # $1 = scoped serial file -> signature string on stdout
    local file=$1 hits=() i
    for i in "${!sig_names[@]}"; do
        [[ "${sig_regexes[$i]}" == 'FATAL: Not an SVC' ]] && continue
        grep -q -- "${sig_regexes[$i]}" "$file" 2>/dev/null && hits+=("${sig_names[$i]}")
    done
    (( $(fatal_unattributed "$file") > 0 )) && hits+=("fatal-not-an-svc")
    if ((${#hits[@]})); then
        (IFS=+; printf '%s\n' "${hits[*]}")
    elif ! grep -qE 'pcid-spawner: spawn ".*(rtl8139d|e1000d)"' "$file" 2>/dev/null; then
        printf 'no-net-driver-spawn\n'
    elif tier_scheme "$file"; then
        printf 'scheme-registered-no-failure\n'
    else
        printf 'spawned-no-failure-no-net-scheme-proof\n'
    fi
}

# ---------------------------------------------------------------------------
# Proof tiers — each tier requires POSITIVE evidence in the capture. Higher
# tiers never claim what a lower tier did not prove:
#
#   driver  pcid spawned the NIC driver (rtl8139d/e1000d)
#   irq     the driver banner carries an interrupt spec AND the kernel opened
#           the DTB phandle IRQ (open_phandle_irq) — the INTx path
#   scheme  a POSITIVE `/scheme/network.<...>_<driver>` line appears in the
#           capture (only obtainable via the opt-in login probe). This is the
#           network-scheme registration the plan's marker 3 asks for.
#   lease   retained as [U] for compatibility: serial text does not validate a
#           lease. dhcp_log reports ordered native progress markers only;
#           scheme registration is NOT traffic and is never reported as such.
# ---------------------------------------------------------------------------
tier_driver() { grep -qE 'pcid-spawner: spawn ".*(rtl8139d|e1000d)"' "$1" 2>/dev/null; }
tier_irq()    { tier_driver "$1" \
                && grep -qE '(RTL8139|E1000).*IRQ: \(phandle' "$1" 2>/dev/null \
                && grep -qE 'open_phandle_irq' "$1" 2>/dev/null; }
tier_scheme() { tier_irq "$1" \
                && grep -qE '/scheme/network\.[^ ]*(rtl8139|e1000)' "$1" 2>/dev/null; }
dhcp_log_progress() {
    awk '
        /DHCP: Sent Discover/ && step == 0 { step = 1; next }
        /DHCP: Offer IP:/ && step == 1 { step = 2; next }
        /DHCP: New IP:/ && step == 2 { step = 3; next }
        /DHCP: Sent Request/ && step == 3 { step = 4; next }
        /DHCP: Ack IP:/ && step == 4 { step = 5; next }
        END { exit(step == 5 ? 0 : 1) }
    ' "$1" 2>/dev/null
}

# Serial markers show client progress only. Even all five markers do not prove
# that the ACK was accepted or that the guest installed a working lease.
tier_lease() { return 1; }

proof_tiers() {  # $1 = scoped serial -> lease stays [U]; dhcp_log reports progress only
    local t dhcp_log='dhcp_log=none'
    local -a out=()
    for t in driver irq scheme lease; do
        if "tier_$t" "$1"; then out+=("$t=[V]"); else out+=("$t=[U]"); fi
    done
    if dhcp_log_progress "$1"; then dhcp_log='dhcp_log=ordered-progress'; fi
    out+=("$dhcp_log")
    (IFS=' '; printf '%s\n' "${out[*]}")
}

accepted() {  # $1 = scoped serial file -> 0 ONLY on positive scheme registration
              # plus a clean NIC window. Spawn + banner + no failure line is the
              # OLD false positive and is NOT acceptance (review finding).
    tier_scheme "$1" || return 1
    local i
    for i in "${!sig_names[@]}"; do
        [[ "${sig_regexes[$i]}" == 'FATAL: Not an SVC' ]] && continue
        grep -q -- "${sig_regexes[$i]}" "$1" 2>/dev/null && return 1
    done
    (( $(fatal_unattributed "$1") > 0 )) && return 1
    return 0
}

# A run is only "decided" early when a NAMED failure signature appears in the NIC
# scope. Absence of a spawn line is NOT terminal — the guest has not reached pcid.
terminal_failure() {  # $1 = scoped serial file
    local i
    for i in "${!sig_names[@]}"; do
        [[ "${sig_regexes[$i]}" == 'FATAL: Not an SVC' ]] && continue
        grep -q -- "${sig_regexes[$i]}" "$1" 2>/dev/null && return 0
    done
    (( $(fatal_unattributed "$1") > 0 )) && return 0
    return 1
}

# How far the guest got. Signatures are only comparable within the same stage —
# a hang in the UEFI/bootloader menu is a different condition from a driver panic.
stage() {  # $1 = serial file
    local f=$1
    if grep -qE 'pcid-spawner: spawn' "$f" 2>/dev/null; then printf 'pcid\n'
    elif grep -qE 'init: ' "$f" 2>/dev/null; then printf 'redox-init\n'
    elif grep -qE 'Redox OS Bootloader' "$f" 2>/dev/null; then printf 'bootloader-menu\n'
    else printf 'uefi\n'
    fi
}

# ---------------------------------------------------------------------------
# E1-NET-1 exchange classifier — three independent domains, ALL required:
#   peer   (HOST)   the exact per-run nonce arrived at the 127.0.0.1 listener
#   pcap   (HOST)   ARP->SYN->SYN/ACK->data+nonce sequence in the filter-dump
#                   capture; addresses/MACs are OBSERVED from the capture
#                   (ARP "tell" IP, SYN source), never hard-coded
#   serial (GUEST)  login reached, no tcp-connect panic, no smoltcpd error,
#                   no unattributed FATAL in the NIC window
# A nonce echoed on the console alone is NOT proof.
# ---------------------------------------------------------------------------
peer_verdict() {  # $1 = host recv file, $2 = nonce
    [[ -s "$1" ]] || { printf 'peer-empty\n'; return 1; }
    grep -qxF -- "$2" "$1" && { printf 'peer-nonce-exact\n'; return 0; }
    printf 'peer-wrong-nonce\n'; return 1
}

pcap_verdict() {  # $1 = tcpdump -nn -e text, $2 = tcpdump -nn -A text, $3 = port, $4 = nonce
    local e=$1 a=$2 port=$3 nonce=$4
    [[ -n "$e" ]] || { printf 'pcap-empty\n'; return 1; }
    local arp tell syn sport n_arp n_syn n_sack n_data
    arp=$(grep -m1 -E 'Request who-has 10\.0\.2\.2.*tell' <<<"$e")
    [[ -n "$arp" ]] || { printf 'no-arp-request\n'; return 1; }
    tell=$(sed -E 's/.*who-has 10\.0\.2\.2.*tell ([0-9.]+).*/\1/' <<<"$arp")
    n_arp=$(grep -n -m1 -E 'Request who-has 10\.0\.2\.2.*tell' <<<"$e" | cut -d: -f1)
    local syn
    syn=$(grep -m1 -E "> 10\.0\.2\.2[.:]$port: Flags \[S\]," <<<"$e")
    [[ -n "$syn" ]] || { printf 'arp-only-no-syn\n'; return 1; }
    n_syn=$(grep -n -m1 -E "> 10\.0\.2\.2[.:]$port: Flags \[S\]," <<<"$e" | cut -d: -f1)
    sport=$(grep -oE "[0-9.]+\.[0-9]+ > 10\.0\.2\.2[.:]$port" <<<"$syn" | head -1 | sed -E 's/.*\.([0-9]+) > .*/\1/')
    grep -qE "${tell//./\\.}\.$sport > 10\.0\.2\.2" <<<"$syn" || { printf 'syn-source-not-arp-tell\n'; return 1; }
    n_sack=$(grep -n -m1 -E "10\.0\.2\.2[.:]$port > ${tell//./\\.}\.$sport: Flags \[S\.\]" <<<"$e" | cut -d: -f1)
    [[ -n "$n_sack" ]] || { printf 'syn-no-synack\n'; return 1; }
    n_data=$(grep -n -m1 -E "> 10\.0\.2\.2[.:]$port: Flags \[P\.\]" <<<"$e" | cut -d: -f1)
    [[ -n "$n_data" ]] || { printf 'synack-no-data\n'; return 1; }
    (( n_arp < n_syn && n_syn < n_sack && n_sack < n_data )) || { printf 'segments-out-of-order\n'; return 1; }
    grep -qF -- "$nonce" <<<"$a" || { printf 'data-no-nonce-payload\n'; return 1; }
    printf 'full-sequence\n'; return 0
}

serial_clean() {  # $1 = full serial file, $2 = scoped serial file
    grep -q 'redox login:' "$1" || { printf 'login-not-reached\n'; return 1; }
    grep -q "Can't connect tcp socket" "$2" && { printf 'tcp-connect-panic\n'; return 1; }
    grep -qE 'smoltcpd: ' "$2" && { printf 'smoltcpd-error\n'; return 1; }
    (( $(fatal_unattributed "$2") > 0 )) && { printf 'fatal-in-nic-window\n'; return 1; }
    printf 'clean\n'; return 0
}

# ---- no-boot self-test (fixtures run BEFORE any boot is allowed) --------------
run_selftest() {
    local st p=0 f=0 N='E1NET-TEST-1234' P=47011
    st=$(mktemp -d); trap 'rm -rf "$st"' RETURN
    okx()  { p=$((p + 1)); printf 'ok    %s\n' "$1"; }
    badx() { f=$((f + 1)); printf 'FAIL  %s\n' "$1"; }
    vcheck() { # $1 desc, $2 expected verdict, rest = func+args
        local desc=$1 want=$2 got
        shift 2
        got=$("$@" 2>/dev/null || true)
        [[ "$got" == "$want" ]] && okx "$desc -> $got" || badx "$desc: want=$want got=$got"
    }
    # Realistic tcpdump -nn -e text; the classifier must OBSERVE tell/sport here.
    cat >"$st/e" <<'EOF'
06:22:53.123456 52:54:00:12:34:56 (oui Unknown) > Broadcast, ethertype ARP (0x0806), length 42: Request who-has 10.0.2.2 (ff:ff:ff:ff:ff:ff) tell 10.0.2.15, length 28
06:22:53.223456 52:54:00:12:00:00 (oui Unknown) > 52:54:00:12:34:56 (oui Unknown), ethertype ARP (0x0806), length 42: Reply 10.0.2.2 is-at 52:54:00:12:00:00, length 28
06:22:53.323456 52:54:00:12:34:56 (oui Unknown) > 52:54:00:12:00:00 (oui Unknown), ethertype IPv4 (0x0800), length 74: 10.0.2.15.49514 > 10.0.2.2.47011: Flags [S], seq 111, win 1024, options [mss 1460], length 0
06:22:53.423456 52:54:00:12:00:00 (oui Unknown) > 52:54:00:12:34:56 (oui Unknown), ethertype IPv4 (0x0800), length 74: 10.0.2.2.47011 > 10.0.2.15.49514: Flags [S.], seq 1, ack 112, win 65535, length 0
06:22:53.523456 52:54:00:12:34:56 (oui Unknown) > 52:54:00:12:00:00 (oui Unknown), ethertype IPv4 (0x0800), length 84: 10.0.2.15.49514 > 10.0.2.2.47011: Flags [P.], seq 112:129, ack 2, win 1024, length 17
EOF
    printf '06:22:53.5 IP 10.0.2.15.49514 > 10.0.2.2.47011: Flags [P.], length 17\n%s\n' "$N" >"$st/a"

    vcheck 'pcap full sequence' full-sequence pcap_verdict "$(cat "$st/e")" "$(cat "$st/a")" "$P" "$N"
    vcheck 'pcap ARP only (RX broken shape)' arp-only-no-syn pcap_verdict "$(head -2 "$st/e")" "$(cat "$st/a")" "$P" "$N"
    vcheck 'pcap no SYN/ACK (return path broken)' syn-no-synack pcap_verdict "$(grep -v 'Flags \[S\.\]' "$st/e")" "$(cat "$st/a")" "$P" "$N"
    vcheck 'pcap no data segment' synack-no-data pcap_verdict "$(grep -v 'Flags \[P\.\]' "$st/e")" "$(cat "$st/a")" "$P" "$N"
    vcheck 'pcap empty' pcap-empty pcap_verdict '' "$(cat "$st/a")" "$P" "$N"
    vcheck 'data without nonce payload' data-no-nonce-payload pcap_verdict "$(cat "$st/e")" 'garbage payload' "$P" "$N"
    vcheck 'stale run nonce in pcap' data-no-nonce-payload pcap_verdict "$(cat "$st/e")" "$(cat "$st/a")" "$P" 'E1NET-STALE-9999'

    : >"$st/recv.empty"
    printf 'E1NET-OLD-RUN-0001\n' >"$st/recv.stale"
    printf 'prefix noise\n%s\ntrailing\n' "$N" >"$st/recv.good"
    vcheck 'peer empty (console-only nonce shape)' peer-empty peer_verdict "$st/recv.empty" "$N"
    vcheck 'peer stale nonce' peer-wrong-nonce peer_verdict "$st/recv.stale" "$N"
    vcheck 'peer exact nonce line' peer-nonce-exact peer_verdict "$st/recv.good" "$N"
    vcheck 'peer substring-only is not exact' peer-wrong-nonce peer_verdict "$st/recv.stale" 'E1NET-OLD'

    printf 'Redox OS Bootloader\nredox login: \n' >"$st/s.login"
    printf 'redox login: \nion:/$ ' >"$st/s.scope"
    vcheck 'serial login+clean' clean serial_clean "$st/s.login" "$st/s.scope"
    printf 'redox login: \nthread panicked: Can'"'"'t connect tcp socket\n' >"$st/s.panic"
    vcheck 'serial tcp-connect panic' tcp-connect-panic serial_clean "$st/s.login" "$st/s.panic"
    printf 'smoltcpd: Multiple network adapters\n' >"$st/s.smol"
    vcheck 'serial smoltcpd error' smoltcpd-error serial_clean "$st/s.login" "$st/s.smol"
    : >"$st/s.nologin"
    vcheck 'serial login not reached' login-not-reached serial_clean "$st/s.nologin" "$st/s.scope"

    cat >"$st/s.dhcp" <<'EOF'
DHCP: Sent Discover
DHCP: Offer IP: 10.0.2.15, Server IP: 10.0.2.2
DHCP: New IP: 10.0.2.15/24
DHCP: Sent Request
DHCP: Ack IP: [10, 0, 2, 15], Server IP: [10, 0, 2, 2]
EOF
    vcheck 'native DHCP markers are progress only' \
        'driver=[U] irq=[U] scheme=[U] lease=[U] dhcp_log=ordered-progress' proof_tiers "$st/s.dhcp"
    sed '/DHCP: Ack IP:/d' "$st/s.dhcp" >"$st/s.dhcp-no-ack"
    vcheck 'native DHCP markers missing ACK' \
        'driver=[U] irq=[U] scheme=[U] lease=[U] dhcp_log=none' proof_tiers "$st/s.dhcp-no-ack"
    cat >"$st/s.dhcp-out-of-order" <<'EOF'
DHCP: Sent Discover
DHCP: Offer IP: 10.0.2.15, Server IP: 10.0.2.2
DHCP: Ack IP: [10, 0, 2, 15], Server IP: [10, 0, 2, 2]
DHCP: New IP: 10.0.2.15/24
DHCP: Sent Request
EOF
    vcheck 'native DHCP markers out of order' \
        'driver=[U] irq=[U] scheme=[U] lease=[U] dhcp_log=none' proof_tiers "$st/s.dhcp-out-of-order"
    printf 'dhcpd failed to acquire lease\nudhcpc\n' >"$st/s.dhcp-failure"
    vcheck 'DHCP failure text and udhcpc do not verify lease' \
        'driver=[U] irq=[U] scheme=[U] lease=[U] dhcp_log=none' proof_tiers "$st/s.dhcp-failure"

    # command faults: bad port must exit 2 BEFORE the ISO/boot path is reachable
    local err rc bad
    for bad in 'abc' '80' '70000'; do
        err=$(REDOX_FORKS="$st/noforks" INTX_SKIP_PRECHECK=1 INTX_SKIP_DTB=1 \
              env E1NET_PORT="$bad" bash "${BASH_SOURCE[0]}" 2>&1 >/dev/null; printf '@@RC=%s' "$?")
        rc=${err##*@@RC=}; err=${err%%@@RC=*}
        if (( rc == 2 )) && grep -qF 'E1NET_PORT must be an integer' <<<"$err" \
           && ! grep -q 'missing ISO' <<<"$err"; then
            okx "command fault E1NET_PORT=$bad (exit 2, boot path never reached)"
        else
            badx "command fault E1NET_PORT=$bad (rc=$rc): ${err:0:160}"
        fi
    done
    printf '\nselftest: %s passed, %s failed\n' "$p" "$f"
    return "$f"
}

# Fixture mode: expose the classifier + validation to a sourcing fixture script.
# Nothing below this guard runs, so sourcing has no side effects and boots nothing.
if [[ ${E1_INTX_LIB:-0} == 1 ]]; then
    return 0 2>/dev/null || exit 0
fi

if [[ ${E1NET_SELFTEST:-0} == 1 ]]; then
    run_selftest
    exit $?
fi

test -f "$image" || { printf 'missing ISO %s (image-content task, not this gate)\n' "$image" >&2; exit 1; }
test -f "$tftp/bootaa64.efi" || { printf 'missing TFTP bootfile %s\n' "$tftp/bootaa64.efi" >&2; exit 1; }
mkdir -p "$out"

stop_qemu() {  # bounded cleanup of the owned child only
    local qpid=${1:-${QPID:-}}
    [[ -n "$qpid" ]] || return 0
    if [[ -S "$monitor" ]]; then
        printf 'quit\n' | timeout 5 socat - "UNIX-CONNECT:$monitor" >/dev/null 2>&1 || true
    fi
    kill -TERM "$qpid" 2>/dev/null || true
    for _ in 1 2 3 4 5; do
        kill -0 "$qpid" 2>/dev/null || break
        sleep 1
    done
    kill -KILL "$qpid" 2>/dev/null || true
    wait "$qpid" 2>/dev/null || true
    QPID=''
}
trap 'stop_qemu; [[ -n "${PEER:-}" ]] && kill "$PEER" 2>/dev/null; true' EXIT

# ---- host-side pre-checks (<=60 s each, no guest boot) -----------------------
precheck() {  # $1 = label, rest = extra QEMU device args for the dumpdtb
    local label=$1; shift
    local dtb_out="$out/intx-precheck-$label.dtb"
    timeout 60 qemu-system-aarch64 -machine "virt,acpi=off,gic-version=3,dumpdtb=$dtb_out" \
        -cpu max -smp 1 -m 2048 "$@" >"$out/intx-precheck-$label.log" 2>&1 || {
        printf 'precheck %s: QEMU dumpdtb failed\n' "$label" >&2
        cat "$out/intx-precheck-$label.log" >&2
        return 1
    }
    fdtdump "$dtb_out" >"$out/intx-precheck-$label.fdtdump"
    dtc -I dtb -O dts "$dtb_out" >"$out/intx-precheck-$label.dts"
    local map_cells endpoint
    map_cells=$(awk '/interrupt-map = </{f=1; sub(/.*interrupt-map = /,"")} f{print} f&&/>;/{exit}' \
        "$out/intx-precheck-$label.dts" | tr -d '<>;' | wc -w)
    endpoint=$(grep -icE 'ethernet@|rtl8139|e1000' "$out/intx-precheck-$label.dts" || true)
    printf 'precheck %s: dtb=%s sha256=%s interrupt-map-cells=%s (plan: 160) endpoint-nodes=%s\n' \
        "$label" "$dtb_out" "$(sha256sum "$dtb_out" | cut -d ' ' -f1)" "$map_cells" "$endpoint"
}

if [[ ${INTX_SKIP_PRECHECK:-0} != 1 ]]; then
    precheck nic -device "$nic,netdev=e1net" -netdev user,id=e1net \
        || printf 'precheck nic: FAILED (see %s/intx-precheck-nic.log)\n' "$out"
    precheck bridge -device pcie-pci-bridge -device "$nic,bus=pci.0,netdev=e1net" \
        -netdev user,id=e1net \
        || printf 'precheck bridge: FAILED (see %s/intx-precheck-bridge.log)\n' "$out"
fi

# ---- boot DTB, built in worktree out/ exactly like scripts/qemu-demo/boot.sh --
if [[ ${INTX_SKIP_DTB:-0} != 1 ]]; then
    timeout 60 qemu-system-aarch64 -machine virt,acpi=off,gic-version=3,dumpdtb="$out/virt.dtb" \
        -cpu max -smp 1 -m 2048 >/dev/null 2>&1
    dtc -@ -I dts -O dtb -o "$out/simplefb.dtbo" "$repo/docs/workbench/K8B-SIMPLEFB.dts"
    fdtoverlay -i "$out/virt.dtb" -o "$dtb" "$out/simplefb.dtbo"
fi
test -f "$dtb" || { printf 'missing %s\n' "$dtb" >&2; exit 1; }

# ---- attempts ----------------------------------------------------------------
# The ledger and the per-attempt receipts are append-only across invocations: a
# re-run can never overwrite the evidence backing an earlier attempt.
[[ -f "$ledger" ]] || printf 'attempt\tstarted_utc\tnic\texit\tserial_sha256\tdtb_sha256\tiso_sha256\tstage\tpre_nic_panics\tsignature\texchange\tpeer_sha256\tpcap_sha256\tnonce\n' >"$ledger"
iso_sha=$(sha256sum "$image" | cut -d ' ' -f1)      # input hash BEFORE; re-checked after the run
dtb_sha=$(sha256sum "$dtb" | cut -d ' ' -f1)
start=1
while [[ -e "$out/e1net-attempt-$start.txt" ]]; do start=$((start + 1)); done
prev_key=$(tail -n +2 "$ledger" | tail -1 | awk -F'\t' '{print $8":"$10}')
repeat=0

# The console line is captured to a file AND kept writable, so one Enter can be
# pushed to the guest over the same UART that AAVMF uses as its console input.
serial_in="$out/e1net-serial-in.fifo"
[[ -p "$serial_in" ]] || mkfifo "$serial_in"
exec 3<>"$serial_in"   # read-write open: does not block waiting for a reader

for attempt in $(seq "$start" "$((start + attempts - 1))"); do
    rm -f "$serial" "$monitor" "$qlog"
    scope="$out/e1net-attempt-$attempt-nicscope.txt"
    pcap="$out/e1net-attempt-$attempt.pcap"
    pcap_txt="$out/e1net-attempt-$attempt-pcap.txt"
    pcap_pay="$out/e1net-attempt-$attempt-pcap-payload.txt"
    recv="$out/e1net-host-recv-$attempt.txt"
    rm -f "$pcap" "$pcap_txt" "$pcap_pay" "$recv"   # fresh per attempt; earlier attempts immutable
    printf 'attempt %s: nic=%s timeout=%ss port=%s nonce=%s\n' "$attempt" "$nic" "$run_timeout" "$port" "$nonce"

    # Host peer BEFORE QEMU: loopback-only, bounded by timeout 95 s; the recv
    # file is created and written by socat alone (never synthesized here).
    timeout 95 socat -u "TCP-LISTEN:$port,bind=127.0.0.1,reuseaddr" \
        "OPEN:$recv,creat,append" & PEER=$!

    timeout --signal=TERM --kill-after=5 "$run_timeout" qemu-system-aarch64 \
        -machine virt,acpi=off,gic-version=3 -cpu max -smp 1 -m 2048 \
        -bios /usr/share/qemu-efi-aarch64/QEMU_EFI.fd \
        -drive "if=none,id=e1disk,file=$image,format=raw,readonly=on" \
        -device virtio-blk-pci,drive=e1disk,bootindex=2 \
        -device ramfb -dtb "$dtb" \
        -netdev "user,id=e1net,tftp=$tftp,bootfile=bootaa64.efi" \
        -object "filter-dump,id=e1dump,netdev=e1net,file=$pcap" \
        -device "$nic,netdev=e1net,bootindex=1" \
        -display none -serial stdio \
        -monitor "unix:$monitor,server,nowait" <"$serial_in" >"$serial" 2>"$qlog" &
    QPID=$!
    qpid=$QPID

    # Fail fast: a rejected command line produces no serial to wait for.
    sleep 3
    if ! kill -0 "$qpid" 2>/dev/null; then
        printf 'attempt %s: QEMU exited immediately (launch failure):\n' "$attempt" >&2
        cat "$qlog" >&2
        kill "$PEER" 2>/dev/null || true
        printf '%s\t%s\t%s\tlaunch\t%s\t%s\t%s\t%s\t%s\t%s\tno\tn/a\tn/a\t%s\n' "$attempt" "$(date -u +%FT%TZ)" \
            "$nic" 'n/a' "$dtb_sha" "$iso_sha" 'none' '0' 'launch-failed' "$nonce" >>"$ledger"
        break
    fi

    # Bounded poll, with two documented host-side console interactions (same UART,
    # no image or fork change):
    #
    # 1. Menu Enter. Swapping the NIC also removes the PXE boot path: AAVMF 2024.02
    #    has no UEFI PXE driver for rtl8139, while the recorded chain PXE-booted the
    #    `live`-feature bootloader from TFTP. AAVMF therefore boots the ISO's own
    #    bootloader, whose live-mode menu waits for a keypress (bootloader-e1
    #    src/main.rs:170-286; the auto-select branch is cfg-gated on `live`/`ramboot`).
    # 2. Optional login probe (INTX_LOGIN_PROBE=1): answers the plan's open question
    #    "does the network scheme register?" by logging in and listing /scheme.
    nudge=0
    probe=0
    waited=3
    while (( waited < run_timeout )); do
        if [[ $nudge -lt 2 && -f "$serial" ]] && grep -q 'Press e to edit boot environment' "$serial"; then
            if (( nudge == 0 )); then
                printf '\r' >&3   # Enter over the console UART (AAVMF ConIn)
            else
                printf 'sendkey ret\n' | timeout 5 socat - "UNIX-CONNECT:$monitor" >/dev/null 2>&1 || true
            fi
            nudge=$((nudge + 1))
            printf 'attempt %s: console Enter #%s at %ss\n' "$attempt" "$nudge" "$waited"
        fi

        if [[ ${INTX_LOGIN_PROBE:-0} == 1 && $probe -eq 0 ]] && grep -q 'redox login:' "$serial"; then
            probe=1
            for line in "${probe_cmds[@]}"; do
                printf '%s\r' "$line" >&3
                sleep 2
            done
            printf 'attempt %s: console login probe sent at %ss\n' "$attempt" "$waited"
        fi

        if [[ -s "$serial" ]]; then
            nic_scope "$serial" "$scope"
            # Decide only once the NIC driver has actually spawned: earlier panics
            # belong to other drivers (virtio-blkd) and are not this gate's verdict.
            if grep -qE 'pcid-spawner: spawn ".*(rtl8139d|e1000d)"' "$serial"; then
                # Early success: the exact nonce arrived at the host peer.
                if [[ -f "$recv" ]] && grep -qxF -- "$nonce" "$recv"; then
                    printf 'attempt %s: nonce arrived at host peer after %ss\n' "$attempt" "$waited"
                    break
                fi
                if terminal_failure "$scope"; then
                    printf 'attempt %s: decided after %ss\n' "$attempt" "$waited"
                    break
                fi
            fi
        fi
        kill -0 "$qpid" 2>/dev/null || { printf 'attempt %s: QEMU exited at %ss\n' "$attempt" "$waited"; break; }
        sleep 5
        waited=$((waited + 5))
    done
    stop_qemu "$qpid"
    kill "$PEER" 2>/dev/null || true
    wait "$PEER" 2>/dev/null || true

    # Packet-proof text: extracted from the pcap by tcpdump; the classifier
    # OBSERVES addresses/MACs from the capture, never hard-codes them.
    pcap_e='' pcap_a=''
    if [[ -s "$pcap" ]]; then
        timeout 20 tcpdump -nn -e -r "$pcap" >"$pcap_txt" 2>/dev/null || true
        timeout 20 tcpdump -nn -A -r "$pcap" 2>/dev/null | head -c 65536 >"$pcap_pay" || true
        pcap_e=$(cat "$pcap_txt")
        pcap_a=$(cat "$pcap_pay")
    fi
    if [[ ! -s "$serial" ]]; then
        sig='no-serial-captured'
        pre_panics=0
        stage=none
        tiers='driver=[U] irq=[U] scheme=[U] lease=[U] dhcp_log=none'
        peer_v='peer-no-capture'; pcap_v='pcap-empty'; serial_v='no-serial-captured'
        ok=no
    else
        nic_scope "$serial" "$scope"
        sig=$(signature "$scope")
        pre_panics=$(pre_nic_panics "$serial")
        tiers=$(proof_tiers "$scope")
        stage=$(stage "$serial")
        peer_v=$(peer_verdict "$recv" "$nonce" || true)
        pcap_v=$(pcap_verdict "$pcap_e" "$pcap_a" "$port" "$nonce" || true)
        serial_v=$(serial_clean "$serial" "$scope" || true)
        ok=no
        [[ "$peer_v" == peer-nonce-exact && "$pcap_v" == full-sequence && "$serial_v" == clean ]] \
            && ok=yes-exchange
    fi
    # The digest must pin the IMMUTABLE saved copy of this attempt's capture,
    # never the live capture file — the next attempt deletes and rewrites it.
    raw_saved="$out/e1net-attempt-$attempt-raw.txt"
    if [[ -s "$serial" ]]; then
        cp -f "$serial" "$raw_saved"
        serial_sha=$(sha256sum "$raw_saved" | cut -d ' ' -f1)
    else
        serial_sha='none'
    fi
    strip "$serial" >"$out/e1net-attempt-$attempt-serial.txt" 2>/dev/null || true
    peer_sha=$( [[ -f "$recv" ]] && sha256sum "$recv" | cut -d ' ' -f1 || printf 'none' )
    pcap_sha=$( [[ -s "$pcap" ]] && sha256sum "$pcap" | cut -d ' ' -f1 || printf 'none' )
    {
        printf 'attempt %s  %s  nic=%s  stage=%s  pre_nic_panics=%s  nic_signature=%s  exchange=%s\n' \
            "$attempt" "$(date -u +%FT%TZ)" "$nic" "$stage" "$pre_panics" "$sig" "$ok"
        printf 'proof tiers: %s\n' "$tiers"
        printf 'exchange verdict: peer=%s  pcap=%s  serial=%s  (nonce=%s)\n' "$peer_v" "$pcap_v" "$serial_v" "$nonce"
        printf '  (peer-nonce-exact + full-sequence + clean serial = ONE guest-originated TCP exchange;\n'
        printf '   a nonce echoed on the console alone is NOT proof; no DHCP/DNS/internet/phone claim)\n'
        printf 'serial sha256=%s (of saved immutable copy %s)  dtb sha256=%s  iso sha256=%s\n' \
            "$serial_sha" "$raw_saved" "$dtb_sha" "$iso_sha"
        printf 'pcap sha256=%s  host-recv sha256=%s (recv written by socat only)\n' "$pcap_sha" "$peer_sha"
        printf 'exact qemu command line:\n'
        printf '  timeout --signal=TERM --kill-after=5 %s qemu-system-aarch64 -machine virt,acpi=off,gic-version=3 -cpu max -smp 1 -m 2048 -bios /usr/share/qemu-efi-aarch64/QEMU_EFI.fd -drive if=none,id=e1disk,file=%s,format=raw,readonly=on -device virtio-blk-pci,drive=e1disk,bootindex=2 -device ramfb -dtb %s -netdev user,id=e1net,tftp=%s,bootfile=bootaa64.efi -object filter-dump,id=e1dump,netdev=e1net,file=%s -device %s,netdev=e1net,bootindex=1 -display none -serial stdio -monitor unix:%s,server,nowait\n' \
            "$run_timeout" "$image" "$dtb" "$tftp" "$pcap" "$nic" "$monitor"
        printf 'host peer: timeout 95 socat -u TCP-LISTEN:%s,bind=127.0.0.1,reuseaddr OPEN:%s,creat,append\n' "$port" "$recv"
        printf 'console probe commands: %s\n' "${probe_cmds[*]}"
        printf 'qemu stderr:\n'; cat "$qlog"
    } >"$out/e1net-attempt-$attempt.txt"
    printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "$attempt" "$(date -u +%FT%TZ)" "$nic" \
        'ran' "$serial_sha" "$dtb_sha" "$iso_sha" "$stage" "$pre_panics" "$sig" "$ok" "$peer_sha" "$pcap_sha" "$nonce" >>"$ledger"
    printf 'attempt %s: stage=%s pre_nic_panics=%s nic_signature=%s exchange=%s (peer=%s pcap=%s serial=%s; serial %s)\n' \
        "$attempt" "$stage" "$pre_panics" "$sig" "$ok" "$peer_v" "$pcap_v" "$serial_v" "$out/e1net-attempt-$attempt-serial.txt"

    key="$stage:$sig"
    if [[ "$key" == "$prev_key" ]]; then
        repeat=$((repeat + 1))
        if (( repeat >= 1 )); then
            printf 'identical failure signature twice at the same stage (%s) — stopping per budget rule\n' "$key" >&2
            break
        fi
    else
        repeat=0
    fi
    prev_key=$key
    [[ "$ok" == yes-exchange ]] && {
        printf 'E1-NET-1 PROVEN on attempt %s: guest-originated TCP nonce reached the host peer with a matching packet capture\n' "$attempt"
        break
    }
    # Plan fallback: attempt 1 showed the pcap EMPTY (guest never transmitted) —
    # attempt 2 separates "no IP" from "redirect not sent" with netcfg reads only.
    if [[ "$pcap_v" == pcap-empty && "$attempt" -lt "$((start + attempts - 1))" ]]; then
        IFS='|' read -r -a probe_cmds <<<"root|password|read a < /scheme/netcfg/ifaces/eth0/addr/list|echo IP=\$a|read r < /scheme/netcfg/route/list|echo ROUTE=\$r|echo E1NET-DONE"
        printf 'fallback probe armed for the next attempt (netcfg read, per plan)\n'
    fi
done

iso_sha_after=$(sha256sum "$image" | cut -d ' ' -f1)
dtb_sha_after=$(sha256sum "$dtb" | cut -d ' ' -f1)
printf '\ninput hashes before/after: iso %s/%s  dtb %s/%s\n' "$iso_sha" "$iso_sha_after" "$dtb_sha" "$dtb_sha_after"
[[ "$iso_sha" == "$iso_sha_after" && "$dtb_sha" == "$dtb_sha_after" ]] || \
    printf 'ALARM: a read-only input changed during the run\n' >&2
printf '\nledger: %s\n' "$ledger"
column -t -s $'\t' "$ledger"
