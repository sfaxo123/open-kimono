#!/usr/bin/env python3
"""muse-relay: plain-HTTP -> Meta Model API (HTTPS) relay for agentd.

agentd's model client speaks OpenAI-compatible chat/completions over raw TCP
(no TLS, by design - it runs on Meshnet). This relay lets it use Meta's Muse
Spark directly: it listens on plain HTTP, injects the Meta API key, forces the
model name, and streams the upstream response back chunk by chunk (SSE works).

  MUSE_KEY_FILE=~/.secrets/meta-model-api.key \
  python3 muse_relay.py --listen 127.0.0.1:8799 --model muse-spark-1.3

Then in agentd.toml:
  gateway_url   = "http://127.0.0.1:8799/v1/chat/completions"
  gateway_model = "muse-spark-1.3"

Stdlib only. The key is read from a file (never argv, never logged).
"""
import argparse
import http.server
import json
import os
import socketserver
import sys
import urllib.error
import urllib.request

UPSTREAM = "https://api.meta.ai/v1/chat/completions"


def load_key(path):
    with open(os.path.expanduser(path)) as f:
        for line in f:
            line = line.strip()
            if line.startswith("MODEL_API_KEY="):
                line = line.split("=", 1)[1]
            if line:
                return line.strip().strip('"').strip("'")
    raise SystemExit(f"no key in {path}")


def normalize_tool_args(req):
    """Meta's API requires assistant tool_calls[].function.arguments to be a
    JSON *string* that parses. Echoed history can carry an object, an empty
    string, or a string with trailing junk; repair each to valid JSON text."""
    fixed = 0
    for msg in req.get("messages", []):
        for call in msg.get("tool_calls") or []:
            fn = call.get("function") or {}
            args = fn.get("arguments")
            if isinstance(args, (dict, list)):
                fn["arguments"] = json.dumps(args)
                fixed += 1
                continue
            text = (args or "").strip()
            try:
                json.loads(text)
                continue
            except ValueError:
                pass
            repaired = "{}"
            if text:
                try:  # keep the first complete JSON value, drop any tail
                    val, _ = json.JSONDecoder().raw_decode(text)
                    repaired = json.dumps(val)
                except ValueError:
                    repaired = json.dumps({"raw": text})
            fn["arguments"] = repaired
            fixed += 1
    return fixed


class Relay(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    key = ""
    model = ""

    def log_message(self, fmt, *args):
        sys.stderr.write("muse-relay: " + (fmt % args) + "\n")

    def do_POST(self):
        if not self.path.endswith("/chat/completions"):
            self.send_error(404, "only /v1/chat/completions is relayed")
            return
        body = self.rfile.read(int(self.headers.get("Content-Length", 0)))
        try:
            req = json.loads(body)
        except ValueError:
            self.send_error(400, "body is not JSON")
            return
        asked = req.get("model")
        req["model"] = self.model
        # Fields some OpenAI-compatible servers reject; agentd may send them.
        for k in ("chat_template_kwargs", "cache_prompt"):
            req.pop(k, None)
        fixed = normalize_tool_args(req)
        if fixed:
            self.log_message("normalized %d tool-call argument field(s)", fixed)
        up = urllib.request.Request(
            UPSTREAM,
            data=json.dumps(req).encode(),
            headers={
                "Authorization": "Bearer " + self.key,
                "Content-Type": "application/json",
                "Accept": "text/event-stream" if req.get("stream") else "application/json",
            },
        )
        self.log_message("relay model=%s (asked %s) stream=%s", self.model, asked, bool(req.get("stream")))
        try:
            resp = urllib.request.urlopen(up, timeout=180)
            status = resp.status
        except urllib.error.HTTPError as e:
            resp, status = e, e.code
        except Exception as e:  # network failure: let agentd fall back to its next brain
            self.send_error(502, f"upstream unreachable: {e}")
            return
        self.send_response(status)
        self.send_header("Content-Type", resp.headers.get("Content-Type", "application/json"))
        self.send_header("Transfer-Encoding", "chunked")
        self.send_header("X-Brain", "meta/" + self.model)
        self.end_headers()
        while True:
            chunk = resp.read1(8192) if hasattr(resp, "read1") else resp.read(8192)
            if not chunk:
                break
            self.wfile.write(b"%x\r\n%s\r\n" % (len(chunk), chunk))
            self.wfile.flush()
        self.wfile.write(b"0\r\n\r\n")
        self.wfile.flush()


class Server(socketserver.ThreadingMixIn, http.server.HTTPServer):
    daemon_threads = True


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--listen", default="127.0.0.1:8799")
    ap.add_argument("--model", default="muse-spark-1.3")
    ap.add_argument("--key-file", default=os.environ.get("MUSE_KEY_FILE", "~/.secrets/meta-model-api.key"))
    a = ap.parse_args()
    Relay.key = load_key(a.key_file)
    Relay.model = a.model
    host, port = a.listen.rsplit(":", 1)
    print(f"muse-relay: {a.listen} -> {UPSTREAM} as {a.model}", file=sys.stderr)
    Server((host, int(port)), Relay).serve_forever()


if __name__ == "__main__":
    main()
