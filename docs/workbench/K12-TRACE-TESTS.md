# K12 offline trace-capture verification

No phone was contacted. All inputs to parser tests were synthetic.

| Check | Result |
| --- | --- |
| `shellcheck scripts/oracle/trace/capture.sh scripts/oracle/trace/remote.sh` | PASS, no diagnostics |
| `bash -n scripts/oracle/trace/capture.sh` and `sh -n scripts/oracle/trace/remote.sh` | PASS |
| `PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s scripts/oracle/trace/tests -v` | PASS, 4 tests: mapping/dedup/timing, caller/window filter, malformed event, CLI dry run without output |
| `scripts/oracle/trace/capture.sh --dry-run <profile> fake-phone /tmp/k12-dry-run 2` for all six profiles | PASS, each printed a timeout-bounded `ssh ... sudo -n sh -s` command; `/tmp/k12-dry-run` was not created |
| `cargo test --manifest-path crates/mmio-replay/Cargo.toml --offline` | PASS, 32 unit tests, 5 integration tests, 1 doc test |
| `git diff --check` | PASS |

Capture against pmOS, event availability, virtual-to-physical mapping, and actual replay fixtures remain unverified until the phone is online. See `scripts/oracle/trace/README.md` for the exact runbook and interpretation rules.
