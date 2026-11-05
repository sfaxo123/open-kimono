#!/usr/bin/env bash
# Clean harness run, then each negative control: must COMPILE, then FAIL the
# named tests (and only current/parity tests). Ends by regenerating clean.
set -u
cd "$(dirname "$0")"
export PATH=$HOME/.cargo/bin:$PATH CARGO_NET_OFFLINE=true CARGO_BUILD_JOBS=2
export CARGO_TARGET_DIR="${GICV3_RD_EVIDENCE:?}/harness-target"
LOGS=../logs; mkdir -p "$LOGS"
rc_all=0

declare -A EXPECT=(
  [helper_logs]="helper_missing_returns_owned_diagnostic_without_log helper_preserves_intid_and_affinity_in_diagnostic wrapper_missing_logs_exact_legacy_line_once parity_missing_cases"
  [wrong_affinity]="wrapper_uses_packed_mpidr_affinity wrapper_mapped_returns_some_without_log parity_mapped_cases parity_missing_cases"
  [wrong_diag_affinity]="helper_preserves_intid_and_affinity_in_diagnostic wrapper_missing_logs_exact_legacy_line_once parity_missing_cases"
  [suppressed_log]="wrapper_missing_logs_exact_legacy_line_once wrapper_logs_in_call_order wrapper_uses_packed_mpidr_affinity parity_missing_cases"
)

clean() {
  python3 gen.py >"$LOGS/gen-clean.log" 2>&1 || { echo "gen clean FAILED"; return 1; }
  cargo test -- --nocapture --test-threads=1 >"$LOGS/test-clean.log" 2>&1
  local rc=$?
  echo "clean: cargo test exit=$rc; $(grep '^test result' "$LOGS/test-clean.log" | head -1)"
  return $rc
}

clean || rc_all=1
for m in helper_logs wrong_affinity wrong_diag_affinity suppressed_log; do
  python3 gen.py --mutant "$m" >"$LOGS/gen-$m.log" 2>&1 || { echo "$m: gen FAILED"; rc_all=1; continue; }
  cargo test --no-run >"$LOGS/build-$m.log" 2>&1; brc=$?
  cargo test >"$LOGS/test-$m.log" 2>&1; trc=$?
  failed=$(sed -n 's/^test \(.*\) \.\.\. FAILED$/\1/p' "$LOGS/test-$m.log" | sort)
  ok=1
  [ $brc -eq 0 ] || ok=0
  [ $trc -ne 0 ] || ok=0
  for t in ${EXPECT[$m]}; do echo "$failed" | grep -q "::$t\$" || { ok=0; echo "  missing expected failure: $t"; }; done
  if echo "$failed" | grep -qE '^model::|baseline_legacy'; then ok=0; echo "  unexpected model/baseline failure"; fi
  echo "$m: build exit=$brc test exit=$trc verdict=$([ $ok -eq 1 ] && echo PASS || echo FAIL)"
  echo "$failed" | sed 's/^/  failed: /'
  [ $ok -eq 1 ] || rc_all=1
done
clean || rc_all=1
echo "run_all exit=$rc_all"
exit $rc_all
