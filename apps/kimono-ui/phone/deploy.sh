#!/bin/sh
# Deploy the kimono view to the OnePlus 6 (PLAN A2: "serve it on the phone over
# the Meshnet address or 127.0.0.1 only"). No build step: the files are copied
# as they are, and agentd serves them from `static_dir`.
#
#   apps/kimono-ui/phone/deploy.sh
#
# Writes only under ~/agentic/ on the phone.
set -eu
root=$(CDPATH='' cd "$(dirname -- "$0")/../../.." && pwd)
phone=${PHONE:-oneplus}

ssh "$phone" 'mkdir -p ~/agentic/kimono'
# Only the six files the page needs — no server, no fixtures, no tests.
for f in index.html app.js render.mjs dom.mjs modules.mjs style.css; do
    scp -q "$root/apps/kimono-ui/web/$f" "$phone:agentic/kimono/$f"
done
ssh "$phone" 'ls -la ~/agentic/kimono'
cat <<'EOF'
Now point agentd at it — add to ~/agentic/agentd.toml:

    static_dir = "$HOME/agentic/kimono"

and the page is at http://127.0.0.1:8770/ (loopback only).
EOF
