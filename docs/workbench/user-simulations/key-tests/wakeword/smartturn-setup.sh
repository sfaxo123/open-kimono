#!/bin/sh
# smartturn-setup.sh — one-time provisioning for the CV2 Smart Turn
# sidecar on the phone. Idempotent: re-running skips what exists.
# Provisioning containers use --network host: this phone's podman
# bridge networking (netavark/nft) is broken, and pip needs one
# networked run. The service itself always runs --network none.
# Run ON THE PHONE (it uses the voice container image for a glibc
# Python + pip; the phone's own musl Python cannot take the wheels).
#
# Creates, all additive under ~/agentic/smartturn/:
#   venv/    Python 3.11 venv (in-image) with pinned onnxruntime + numpy
#   model/   smart-turn-v3.2-gpu.onnx from the official Pipecat HF repo,
#            sha256-pinned (docs/PROVENANCE.md)
#   inbox/ outbox/   the engine's job contract dirs
set -eu
IMG=localhost/voice-snapshot:20261024
ST="$HOME/agentic/smartturn"
ORT_VERSION=1.31.1
NUMPY_VERSION=2.4.6
MODEL_URL="https://huggingface.co/pipecat-ai/smart-turn-v3/resolve/f766f81d3cfdf7737ac64aad813d91bbfd56bf93/smart-turn-v3.2-gpu.onnx"
MODEL_SHA256="ab8dc64b88713f90b571c15b714bd1330e6c883cad8763dacf65c9376dc539be"

mkdir -p "$ST/model" "$ST/inbox" "$ST/outbox"

if [ ! -x "$ST/venv/bin/python" ]; then
    echo "creating venv (in-image python3.11)..."
    sudo -n podman run --rm --network host -v "$ST:/st" "$IMG" /usr/bin/python3.11 -m venv /st/venv
    echo "installing onnxruntime==$ORT_VERSION numpy==$NUMPY_VERSION (needs network once)..."
    sudo -n podman run --rm --network host -v "$ST:/st" "$IMG" /st/venv/bin/pip install --no-cache-dir "onnxruntime==$ORT_VERSION" "numpy==$NUMPY_VERSION"
else
    echo "venv exists, skipping"
fi

if [ ! -f "$ST/model/smart-turn-v3.2-gpu.onnx" ]; then
    echo "downloading model from $MODEL_URL ..."
    curl -fL -o "$ST/model/.model.tmp" "$MODEL_URL" || wget -O "$ST/model/.model.tmp" "$MODEL_URL"
    echo "$MODEL_SHA256  $ST/model/.model.tmp" | sha256sum -c -
    mv "$ST/model/.model.tmp" "$ST/model/smart-turn-v3.2-gpu.onnx"
else
    echo "model exists, verifying hash"
    echo "$MODEL_SHA256  $ST/model/smart-turn-v3.2-gpu.onnx" | sha256sum -c -
fi
echo "smartturn setup complete: $ST"
