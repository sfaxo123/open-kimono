#!/usr/bin/env bash
# op6-webcam - R-3 panel capture helper for op6-cycle (capture-only).
#
# Contract (REVIEW-20261031-1930 F-1 burst design): capture ONE frame
# and print exactly one line on stdout:
#   "ok <sha256>"     success; the frame is at the given path
#   "not-authorized"  macOS Camera (TCC) permission missing (cam exit 3)
#   "no-camera"       no capture device (cam exit 2)
#   "capture-failed"  anything else
#   "unavailable"     the capture tool itself is missing
# op6-cycle bursts this helper, keeps the last 3 frames, and does the
# OCR itself. Exit code mirrors the token (0 ok, 3/2/1/5 otherwise).
#
# Usage: op6-webcam <frame.jpg>
# Env hook: OP6_WEBCAM_CAM (default: op6-cam next to this script).
set -u
FRAME=${1:-}
[ -n "$FRAME" ] || { echo "usage: op6-webcam <frame.jpg>" >&2; echo capture-failed; exit 1; }
DIR=$(cd "$(dirname "$0")" && pwd)
CAM=${OP6_WEBCAM_CAM:-$DIR/op6-cam}
if [ ! -x "$CAM" ]; then echo unavailable; exit 5; fi
"$CAM" "$FRAME" >/dev/null 2>&1; RC=$?
if [ $RC -eq 0 ] && [ -s "$FRAME" ]; then
  echo "ok $(shasum -a 256 "$FRAME" | cut -d' ' -f1)"; exit 0
fi
case $RC in
  3) echo not-authorized; exit 3;;
  2) echo no-camera; exit 2;;
  *) echo capture-failed; exit 1;;
esac
