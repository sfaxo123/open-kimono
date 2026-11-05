#!/usr/bin/env bash
# run-all.sh - user-sim regression gate.
#
# Lints every corpus in the repo, mock-runs every corpus (must be
# 100%: the mock proves the harness plumbing), then runs the three
# deliberately bad agents over the core corpus (each must score
# < 100%: they prove the scoring bites). Journey gates: the mock
# journey agent must pass ALL persona journeys, and the forgetful
# agent (memory wiped daily) must pass < 100% of journeys and
# fail the commuter journey from day 2 onward. Prints one
# combined summary. Exit 0 only if all of that holds.
#
# Usage: tools/user-sim/run-all.sh   (from anywhere)
set -u

HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
BIN="$HERE/target/debug/user-sim"
CORE="$ROOT/docs/workbench/user-simulations/corpus/core.jsonl"
EDGE="$ROOT/docs/workbench/user-simulations/edge/edge.jsonl"
ASR="$ROOT/docs/workbench/user-simulations/voice/asr-hard.jsonl"
JOURNEYS="$ROOT/docs/workbench/user-simulations/journeys/journeys.jsonl"
SAMPLE="$HERE/sample.jsonl"

cd "$HERE" || exit 70
BUILD_LOG=$(mktemp /tmp/user-sim-build.XXXXXX.log)
if ! cargo build --offline > "$BUILD_LOG" 2>&1; then
  cargo build >> "$BUILD_LOG" 2>&1 || { echo "run-all: build failed (log: $BUILD_LOG)"; exit 70; }
fi
tail -1 "$BUILD_LOG"

FAILURES=0
SUMMARY=""

score_line() { # $1 = full stdout of a run; echoes "passed total"
  printf '%s\n' "$1" | sed -n 's/^user-sim: \([0-9][0-9]*\)\/\([0-9][0-9]*\) scenarios passed/\1 \2/p' | head -1
}
classes_line() {
  printf '%s\n' "$1" | sed -n 's/^failure classes: //p' | head -1
}
journeys_line() { # $1 = full stdout of a journeys run; echoes "passed total"
  printf '%s\n' "$1" | sed -n 's/^user-sim journeys: \([0-9][0-9]*\)\/\([0-9][0-9]*\) journeys passed/\1 \2/p' | head -1
}

echo "== lint =="
for f in "$SAMPLE" "$CORE" "$EDGE" "$ASR"; do
  out=$("$BIN" --lint "$f" 2>&1)
  rc=$?
  printf '%s\n' "$out" | grep -E '^(note|error):' || true
  if [ $rc -ne 0 ]; then
    echo "LINT FAIL: $f"
    SUMMARY="${SUMMARY}lint FAIL ${f##*/}\n"
    FAILURES=$((FAILURES + 1))
  else
    echo "LINT OK: $f"
    SUMMARY="${SUMMARY}lint ok ${f##*/}\n"
  fi
done

echo "== mock runs (must be 100%) =="
for f in "$SAMPLE" "$CORE" "$EDGE" "$ASR"; do
  out=$("$BIN" --mock --corpus "$f" 2>&1)
  read -r p t <<< "$(score_line "$out")"
  if [ -z "${p:-}" ] || [ "$p" != "$t" ]; then
    echo "MOCK NOT 100%: $f (${p:-?}/${t:-?})"
    printf '%s\n' "$out" | grep '^FAIL' || true
    SUMMARY="${SUMMARY}mock ${f##*/} ${p:-?}/${t:-?} FAIL\n"
    FAILURES=$((FAILURES + 1))
  else
    echo "MOCK 100%: $f ($p/$t)"
    SUMMARY="${SUMMARY}mock ${f##*/} $p/$t ok\n"
  fi
done

echo "== bad agents over core (must be < 100%) =="
for a in reckless deaf leaky; do
  out=$("$BIN" --agent "$a" --corpus "$CORE" 2>&1)
  read -r p t <<< "$(score_line "$out")"
  cls=$(classes_line "$out")
  if [ -z "${p:-}" ] || [ "$p" = "$t" ]; then
    echo "BAD AGENT DID NOT FAIL: $a scored ${p:-?}/${t:-?} on core"
    SUMMARY="${SUMMARY}agent $a core ${p:-?}/${t:-?} FAIL (scored 100% or errored)\n"
    FAILURES=$((FAILURES + 1))
  else
    echo "agent $a on core: $p/$t (failure classes: $cls)"
    SUMMARY="${SUMMARY}agent $a core $p/$t ok [$cls]\n"
  fi
done

echo "== journey runs =="
out=$("$BIN" --journeys --mock --corpus "$JOURNEYS" 2>&1)
read -r p t <<< "$(journeys_line "$out")"
if [ -z "${p:-}" ] || [ "$p" != "$t" ]; then
  echo "JOURNEY MOCK NOT 100%: (${p:-?}/${t:-?})"
  printf '%s\n' "$out" | grep -E '^(FAIL|JOURNEY FAIL)' || true
  SUMMARY="${SUMMARY}journeys mock ${p:-?}/${t:-?} FAIL\n"
  FAILURES=$((FAILURES + 1))
else
  echo "JOURNEY MOCK 100%: $p/$t journeys"
  printf '%s\n' "$out" | grep '^JOURNEY' || true
  SUMMARY="${SUMMARY}journeys mock $p/$t ok\n"
fi

out=$("$BIN" --journeys --agent forgetful --corpus "$JOURNEYS" 2>&1)
read -r p t <<< "$(journeys_line "$out")"
commuter_line=$(printf '%s\n' "$out" | grep '^JOURNEY FAIL commuter' || true)
if [ -z "${p:-}" ] || [ "$p" = "$t" ] || [ -z "$commuter_line" ] || printf '%s' "$commuter_line" | grep -q 'first failure: day 1$'; then
  echo "FORGETFUL DID NOT FAIL AS REQUIRED: journeys ${p:-?}/${t:-?}; commuter: ${commuter_line:-<no commuter failure>}"
  SUMMARY="${SUMMARY}journeys forgetful ${p:-?}/${t:-?} FAIL (must be < 100%, commuter failing from day 2)\n"
  FAILURES=$((FAILURES + 1))
else
  echo "agent forgetful on journeys: $p/$t ($commuter_line)"
  SUMMARY="${SUMMARY}journeys forgetful $p/$t ok [commuter fails from day 2+]\n"
fi

echo ""
echo "== run-all summary =="
printf '%b' "$SUMMARY"
if [ "$FAILURES" -ne 0 ]; then
  echo "run-all: FAILED ($FAILURES gate(s))"
  exit 1
fi
echo "run-all: ALL GATES PASS"
exit 0
