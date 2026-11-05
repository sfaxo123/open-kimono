#!/usr/bin/env bash
# Ephemeral, read-only oracle tracing. Raw files stay on the capture host.
set -euo pipefail
usage() { echo "Usage: $0 [--dry-run] {geni-i2c|rmi4|spmi|ufs|dwc3|audio} <ssh-target> <outdir> [seconds: 1..30]" >&2; exit 2; }
dry_run=0
if [[ ${1:-} == --dry-run ]]; then dry_run=1; shift; fi
[[ $# -ge 3 && $# -le 4 ]] || usage
profile=$1 target=$2 outdir=$3 seconds=${4:-10}
case $profile in geni-i2c|rmi4|spmi|ufs|dwc3|audio) ;; *) usage ;; esac
[[ -n $target && -n $outdir && $seconds =~ ^[0-9]+$ ]] || usage
(( 10#$seconds >= 1 && 10#$seconds <= 30 )) || usage
remote=$(dirname "$0")/remote.sh
[[ -f $remote ]] || exit 1
cmd=(timeout "$((10#$seconds + 40))s" ssh -o BatchMode=yes -o ConnectTimeout=8 "$target" "sudo -n sh -s -- $profile $((10#$seconds))")
if (( dry_run )); then
    printf '%q ' "${cmd[@]}"
    printf '< %q > %q\n' "$remote" "$outdir/$profile.raw.txt"
    exit 0
fi
mkdir -p "$outdir"
raw=$outdir/$profile.raw.txt
printf 'Capture %s for %ss. Perform the stated read-only stimulus now; do not unbind drivers.\n' "$profile" "$seconds" >&2
if ! "${cmd[@]}" < "$remote" > "$raw"; then
    echo "Capture failed; partial raw output retained at $raw" >&2
    exit 1
fi
sha256sum "$raw" > "$raw.sha256"
printf '%s\n' "$raw"
