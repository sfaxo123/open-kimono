#!/usr/bin/env bash
# E1-EXP-1 — ONE bounded rtl8139 NIC-swap boot, emulator only.
#
# This is a worktree-local COPY of the scripts/e1-qemu.sh chain with EXACTLY ONE
# change, per docs/workbench/INTX-NET-FEASIBILITY.md §3:
#
#   -    -device virtio-net-pci,netdev=e1net,bootindex=1 \
#   +    -device rtl8139,netdev=e1net,bootindex=1 \
#
# Machine flags, the ISO drive, ramfb, -dtb simplefb-virt.dtb and the user-netdev
# (same tftp dir + bootfile) are kept identical. The shared scripts
# (e1-qemu.sh, e1-live.sh, integrate.sh) are NOT touched; forks/ is read-only.
#
# Budget: <= INTX_ATTEMPTS attempts x INTX_TIMEOUT seconds, both validated
# numerically at startup and capped at the plan's 4 x 90 s; plus <=2 host-side
# dumpdtb pre-checks of <=60 s each. Identical stage+signature twice = stop.
# No probe (login or otherwise) is ever sent automatically — INTX_LOGIN_PROBE=1
# is the only way console probes happen.
#
# Acceptance is tiered and POSITIVE (see proof_tiers/accepted below): a driver
# spawn, a banner, or the mere absence of a failure line is NOT acceptance.
# scheme-pass requires a positive `/scheme/network.<nic>` registration line in
# the capture, and even that never implies a dhcpd lease or any traffic.
# E1-EXP-1 attempts 1-5 keep their historic labels; nothing here re-scores them.
set -euo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
fork=${REDOX_FORKS:-$HOME/Developer/forks/redox}
image="$fork/redox/build/aarch64/desktop-minimal/e1-live.iso"
tftp="$fork/bootloader-e1/build/tftp-live"
out="$repo/out/integration"
dtb="$out/simplefb-virt.dtb"
nic=${INTX_NIC:-rtl8139}
run_timeout=${INTX_TIMEOUT:-90}
attempts=${INTX_ATTEMPTS:-4}
serial="$out/intx-swap-serial.txt"
monitor="$out/intx-swap-monitor.sock"
qlog="$out/intx-swap-qemu.log"
ledger=${INTX_LEDGER:-$out/intx-ledger.tsv}

# Console probe commands, '|' separated; the first two are the login credentials.
# Only shell builtins are safe here: /usr/bin/ls dies in an EL0 exception on this image.
IFS='|' read -r -a probe_cmds <<<"${INTX_PROBE_CMDS:-root|password|ls /scheme|ls /scheme/net}"

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
#   lease   a dhcpd lease / acquired-IP line appears — informational only;
#           scheme registration is NOT traffic and is never reported as such.
# ---------------------------------------------------------------------------
tier_driver() { grep -qE 'pcid-spawner: spawn ".*(rtl8139d|e1000d)"' "$1" 2>/dev/null; }
tier_irq()    { tier_driver "$1" \
                && grep -qE '(RTL8139|E1000).*IRQ: \(phandle' "$1" 2>/dev/null \
                && grep -qE 'open_phandle_irq' "$1" 2>/dev/null; }
tier_scheme() { tier_irq "$1" \
                && grep -qE '/scheme/network\.[^ ]*(rtl8139|e1000)' "$1" 2>/dev/null; }
tier_lease()  { grep -qiE 'dhcpd.*(lease|bound)|udhcpc|acquired (an )?IP' "$1" 2>/dev/null; }

proof_tiers() {  # $1 = scoped serial file -> "driver=.. irq=.. scheme=.. lease=.."
    local t
    local -a out=()
    for t in driver irq scheme lease; do
        if "tier_$t" "$1"; then out+=("$t=[V]"); else out+=("$t=[U]"); fi
    done
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

# Fixture mode: expose the classifier + validation to the no-boot fixture
# script (e1-intx-classifier-fixtures.sh). Nothing below this guard runs, so
# sourcing has no side effects and boots nothing.
if [[ ${E1_INTX_LIB:-0} == 1 ]]; then
    return 0 2>/dev/null || exit 0
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
trap 'stop_qemu' EXIT

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
[[ -f "$ledger" ]] || printf 'attempt\tstarted_utc\tnic\texit\tserial_sha256\tdtb_sha256\tiso_sha256\tstage\tpre_nic_panics\tsignature\taccepted\n' >"$ledger"
iso_sha=$(sha256sum "$image" | cut -d ' ' -f1)
dtb_sha=$(sha256sum "$dtb" | cut -d ' ' -f1)
start=1
while [[ -e "$out/intx-attempt-$start.txt" ]]; do start=$((start + 1)); done
prev_key=$(tail -n +2 "$ledger" | tail -1 | awk -F'\t' '{print $8":"$10}')
repeat=0

# The console line is captured to a file AND kept writable, so one Enter can be
# pushed to the guest over the same UART that AAVMF uses as its console input.
serial_in="$out/intx-swap-serial-in.fifo"
[[ -p "$serial_in" ]] || mkfifo "$serial_in"
exec 3<>"$serial_in"   # read-write open: does not block waiting for a reader

for attempt in $(seq "$start" "$((start + attempts - 1))"); do
    rm -f "$serial" "$monitor" "$qlog"
    scope="$out/intx-attempt-$attempt-nicscope.txt"
    printf 'attempt %s: nic=%s timeout=%ss\n' "$attempt" "$nic" "$run_timeout"
    timeout --signal=TERM --kill-after=5 "$run_timeout" qemu-system-aarch64 \
        -machine virt,acpi=off,gic-version=3 -cpu max -smp 1 -m 2048 \
        -bios /usr/share/qemu-efi-aarch64/QEMU_EFI.fd \
        -drive "if=none,id=e1disk,file=$image,format=raw,readonly=on" \
        -device virtio-blk-pci,drive=e1disk,bootindex=2 \
        -device ramfb -dtb "$dtb" \
        -netdev "user,id=e1net,tftp=$tftp,bootfile=bootaa64.efi" \
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
        printf '%s\t%s\t%s\tlaunch\t%s\t%s\t%s\t%s\t%s\t%s\tno\n' "$attempt" "$(date -u +%FT%TZ)" \
            "$nic" 'n/a' "$dtb_sha" "$iso_sha" 'none' '0' 'launch-failed' >>"$ledger"
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
                if terminal_failure "$scope" || accepted "$scope"; then
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

    if [[ ! -s "$serial" ]]; then
        sig='no-serial-captured'
        pre_panics=0
        ok=no
        stage=none
        tiers='driver=[U] irq=[U] scheme=[U] lease=[U]'
    else
        nic_scope "$serial" "$scope"
        sig=$(signature "$scope")
        pre_panics=$(pre_nic_panics "$serial")
        ok=no
        accepted "$scope" && ok=yes-scheme
        tiers=$(proof_tiers "$scope")
        stage=$(stage "$serial")
    fi
    # The digest must pin the IMMUTABLE saved copy of this attempt's capture,
    # never the live capture file — the next attempt deletes and rewrites it.
    raw_saved="$out/intx-attempt-$attempt-raw.txt"
    if [[ -s "$serial" ]]; then
        cp -f "$serial" "$raw_saved"
        serial_sha=$(sha256sum "$raw_saved" | cut -d ' ' -f1)
    else
        serial_sha='none'
    fi
    strip "$serial" >"$out/intx-attempt-$attempt-serial.txt" 2>/dev/null || true
    {
        printf 'attempt %s  %s  nic=%s  stage=%s  pre_nic_panics=%s  nic_signature=%s  accepted=%s\n' \
            "$attempt" "$(date -u +%FT%TZ)" "$nic" "$stage" "$pre_panics" "$sig" "$ok"
        printf 'proof tiers: %s\n' "$tiers"
        printf '  (scheme=[V] proves network-scheme registration ONLY — never a dhcpd lease or traffic)\n'
        printf 'serial sha256=%s (of saved immutable copy %s)  dtb sha256=%s  iso sha256=%s\n' \
            "$serial_sha" "$raw_saved" "$dtb_sha" "$iso_sha"
        printf 'exact qemu command line:\n'
        printf '  timeout --signal=TERM --kill-after=5 %s qemu-system-aarch64 -machine virt,acpi=off,gic-version=3 -cpu max -smp 1 -m 2048 -bios /usr/share/qemu-efi-aarch64/QEMU_EFI.fd -drive if=none,id=e1disk,file=%s,format=raw,readonly=on -device virtio-blk-pci,drive=e1disk,bootindex=2 -device ramfb -dtb %s -netdev user,id=e1net,tftp=%s,bootfile=bootaa64.efi -device %s,netdev=e1net,bootindex=1 -display none -serial stdio -monitor unix:%s,server,nowait\n' \
            "$run_timeout" "$image" "$dtb" "$tftp" "$nic" "$monitor"
        printf 'console probe commands: %s\n' "${probe_cmds[*]}"
        printf 'qemu stderr:\n'; cat "$qlog"
    } >"$out/intx-attempt-$attempt.txt"
    printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "$attempt" "$(date -u +%FT%TZ)" "$nic" \
        'ran' "$serial_sha" "$dtb_sha" "$iso_sha" "$stage" "$pre_panics" "$sig" "$ok" >>"$ledger"
    printf 'attempt %s: stage=%s pre_nic_panics=%s nic_signature=%s accepted=%s tiers: %s (serial %s)\n' \
        "$attempt" "$stage" "$pre_panics" "$sig" "$ok" "$tiers" "$out/intx-attempt-$attempt-serial.txt"

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
    [[ "$ok" == yes-scheme ]] && {
        printf 'scheme registration PROVEN on attempt %s — dhcpd lease/traffic remains UNPROVEN\n' "$attempt"
        break
    }
done

printf '\nledger: %s\n' "$ledger"
column -t -s $'\t' "$ledger"
