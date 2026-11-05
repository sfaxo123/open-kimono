#!/usr/bin/env bash
# e1-intx-classifier-fixtures.sh — deterministic NO-BOOT fixtures for the
# E1-EXP-1 classifier in e1-intx-swap.sh (owned by this lane; shared
# tests.sh/boot.sh/evidence.sh/check-digest.sh are NOT touched).
#
# Proves, without QEMU, ISO, or any forks/ write:
#   1. the OLD accepted() false positive — driver spawn + banner + absence of
#      any failure line — is now REJECTED (scheme registration unproven);
#   2. scheme-pass requires a POSITIVE /scheme/network.<...>_<driver> line;
#   3. the tiers never claim a dhcpd lease or traffic from scheme registration,
#      and a lease-looking line alone never passes the scheme gate;
#   4. failure signatures, userspace-fatal attribution, and pre-NIC panic
#      counting keep working through the tiered classifier;
#   5. numeric/budget validation rejects bad INTX_ATTEMPTS / INTX_TIMEOUT /
#      INTX_NIC with exit 2 BEFORE any boot path is reachable.
#
# Fixture serial lines are verbatim from the recorded E1-EXP-1 captures
# (docs/workbench/INTX-E1-EXP1.txt), so the classifier is scored on real data.
set -uo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
E1_INTX_LIB=1 source "$here/e1-intx-swap.sh"   # classifier + validation only
set +e                                          # the sourced -e must not abort fixtures

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
pass=0 fail=0
ok()  { pass=$((pass + 1)); printf 'ok    %s\n' "$1"; }
bad() { fail=$((fail + 1)); printf 'FAIL  %s\n' "$1"; }

# Verbatim lines from attempts 3/4/5 (INTX-E1-EXP1.txt "VERBATIM MARKER LINES").
SPAWN='2026-10-25T06-22-52.946Z [@pcid_spawner:87 INFO] pcid-spawner: spawn "/usr/lib/drivers/rtl8139d"'
BANNER='2026-10-25T06-22-53.009Z [@rtl8139d:60 INFO]  + RTL8139 pci-0000-00-02.0 on: 0=P0000 1=10080000 IRQ: (phandle 32770, [0, 5, 4])'
IRQOPEN='kernel::scheme::irq:DEBUG -- open_phandle_irq  virq=37'
SCHEME='/scheme/network.pci-0000-00-02.0_rtl8139'
FATAL='kernel::arch::aarch64::interrupt::exception:ERROR -- FATAL: Not an SVC induced synchronous exception (ty=111100)'
USERSPC='kernel::context::signal:INFO -- UNHANDLED EXCEPTION, CPU #0, PID 40, NAME /usr/bin/ls, CONTEXT 0xfffffe8000152230'

mk() { local name=$1; shift; printf '%s\n' "$@" >"$tmp/$name"; }

# ---- 1. driver spawn alone: NOT accepted, driver tier only -------------------
mk f_spawn "$SPAWN"
accepted "$tmp/f_spawn" && bad "1a spawn-only must NOT be accepted" || ok "1a spawn-only rejected"
[[ "$(proof_tiers "$tmp/f_spawn")" == 'driver=[V] irq=[U] scheme=[U] lease=[U]' ]] \
    && ok "1b spawn-only tiers driver=[V] only" || bad "1b tiers: $(proof_tiers "$tmp/f_spawn")"
[[ "$(signature "$tmp/f_spawn")" == 'spawned-no-failure-no-net-scheme-proof' ]] \
    && ok "1c spawn-only signature honest" || bad "1c signature: $(signature "$tmp/f_spawn")"

# ---- 2. THE false-positive regression: spawn+banner+IRQ, no failure, no scheme
mk f_irq "$SPAWN" "$BANNER" "$IRQOPEN"
accepted "$tmp/f_irq" && bad "2a spawn+banner+no-failure must NOT be accepted (old false positive)" \
                      || ok "2a old false positive now REJECTED"
[[ "$(proof_tiers "$tmp/f_irq")" == 'driver=[V] irq=[V] scheme=[U] lease=[U]' ]] \
    && ok "2b irq tier without scheme claim" || bad "2b tiers: $(proof_tiers "$tmp/f_irq")"

# ---- 3. positive scheme line: accepted, lease still [U] ----------------------
mk f_scheme "$SPAWN" "$BANNER" "$IRQOPEN" "$SCHEME"
accepted "$tmp/f_scheme" && ok "3a positive scheme accepted" || bad "3a scheme capture rejected"
[[ "$(proof_tiers "$tmp/f_scheme")" == 'driver=[V] irq=[V] scheme=[V] lease=[U]' ]] \
    && ok "3c scheme proven, lease NOT claimed" || bad "3c tiers: $(proof_tiers "$tmp/f_scheme")"
[[ "$(signature "$tmp/f_scheme")" == 'scheme-registered-no-failure' ]] \
    && ok "3d scheme signature" || bad "3d signature: $(signature "$tmp/f_scheme")"

# ---- 4. lease line never implies scheme; scheme gate never passed by lease ---
mk f_lease 'dhcpd: bound lease 10.0.2.15 for 08:00:27:12:34:56'
tier_lease "$tmp/f_lease" && ok "4a lease line detected as its own tier" || bad "4a lease tier missed"
accepted "$tmp/f_lease" && bad "4b lease line alone must NOT pass the scheme gate" || ok "4b lease-only rejected"
mk f_scheme_lease "$SPAWN" "$BANNER" "$IRQOPEN" "$SCHEME" 'dhcpd: bound lease 10.0.2.15'
[[ "$(proof_tiers "$tmp/f_scheme_lease")" == 'driver=[V] irq=[V] scheme=[V] lease=[V]' ]] \
    && ok "4c lease tier reported separately from scheme" || bad "4c tiers: $(proof_tiers "$tmp/f_scheme_lease")"

# ---- 5. failure signature kills acceptance even with scheme line -------------
mk f_fail "$SPAWN" "$BANNER" "$IRQOPEN" "$SCHEME" 'failed to regitster network scheme'
accepted "$tmp/f_fail" && bad "5a scheme line + failure signature must NOT be accepted" || ok "5a failure overrides scheme"
[[ "$(signature "$tmp/f_fail")" == 'failed-register-network-scheme' ]] \
    && ok "5b failure signature named" || bad "5b signature: $(signature "$tmp/f_fail")"
terminal_failure "$tmp/f_fail" && ok "5c failure is terminal (early stop)" || bad "5c not terminal"

# ---- 6. userspace FATAL attribution survives the tiered classifier -----------
mk f_fatal "$SPAWN" "$BANNER" "$IRQOPEN" "$FATAL" "$USERSPC"
[[ "$(fatal_unattributed "$tmp/f_fatal")" == 0 ]] \
    && ok "6a userspace-attributed FATAL not counted" || bad "6a fatal_unattributed=$(fatal_unattributed "$tmp/f_fatal")"
accepted "$tmp/f_fatal" && bad "6b attributed-fatal capture still lacks scheme proof" || ok "6b no scheme proof -> rejected"

# ---- 7. pre-NIC panics counted, never hidden; NIC window scored clean --------
mk f_pre 'not implemented: virtio: MSI-X is not implemented on this architecture' \
         '[ERROR virtio-blkd@src/header/stdlib/mod.rs:121] Abort' "$FATAL" \
         "$SPAWN" "$BANNER" "$IRQOPEN"
[[ "$(pre_nic_panics "$tmp/f_pre")" == 2 ]] \
    && ok "7a pre_nic_panics=2 counted" || bad "7a pre_nic_panics=$(pre_nic_panics "$tmp/f_pre")"
nic_scope "$tmp/f_pre" "$tmp/f_pre.sc"
[[ "$(signature "$tmp/f_pre.sc")" == 'spawned-no-failure-no-net-scheme-proof' ]] \
    && ok "7b NIC-window signature excludes pre-spawn panics" || bad "7b signature: $(signature "$tmp/f_pre.sc")"

# ---- 8. no driver spawn at all -----------------------------------------------
mk f_nospawn 'Redox OS Bootloader' 'Press e to edit boot environment'
[[ "$(signature "$tmp/f_nospawn")" == 'no-net-driver-spawn' ]] \
    && ok "8a no-spawn signature" || bad "8a signature: $(signature "$tmp/f_nospawn")"
accepted "$tmp/f_nospawn" && bad "8b no-spawn must NOT be accepted" || ok "8b no-spawn rejected"

# ---- 9. command faults: validation exits 2 BEFORE any boot path --------------
# REDOX_FORKS points at an empty dir and the pre-checks are skipped, so even a
# validation bug could never reach QEMU here; the assertions prove validation
# fired first (exit 2 + its message, and NOT the later 'missing ISO' path).
mkdir -p "$tmp/noforks"
fault() {
    local desc=$1 frag=$2; shift 2
    local err rc
    err=$(REDOX_FORKS="$tmp/noforks" INTX_SKIP_PRECHECK=1 INTX_SKIP_DTB=1 \
          env "$@" bash "$here/e1-intx-swap.sh" 2>&1 >/dev/null)
    rc=$?
    if (( rc == 2 )) && grep -qF "$frag" <<<"$err" && ! grep -q 'missing ISO' <<<"$err"; then
        ok "9  command fault: $desc (exit 2, boot path never reached)"
    else
        bad "9  command fault: $desc (rc=$rc): ${err:0:160}"
    fi
}
fault 'INTX_ATTEMPTS=abc'  'INTX_ATTEMPTS must be an integer 1..4' INTX_ATTEMPTS=abc
fault 'INTX_ATTEMPTS=5'    'INTX_ATTEMPTS must be an integer 1..4' INTX_ATTEMPTS=5
fault 'INTX_ATTEMPTS=0'    'INTX_ATTEMPTS must be an integer 1..4' INTX_ATTEMPTS=0
fault 'INTX_TIMEOUT=91'    'INTX_TIMEOUT must be an integer 1..90' INTX_TIMEOUT=91
fault 'INTX_TIMEOUT=xyz'   'INTX_TIMEOUT must be an integer 1..90' INTX_TIMEOUT=xyz
fault 'INTX_NIC injection' 'INTX_NIC must match'                   INTX_NIC='rtl8139;rm -rf'

printf '\nfixtures: %s passed, %s failed\n' "$pass" "$fail"
exit "$fail"
