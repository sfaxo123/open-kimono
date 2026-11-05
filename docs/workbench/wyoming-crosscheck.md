# wyoming Rust<->Python cross-check
date: 2026-10-24T23:51:11Z (UTC)
reference: $HOME/Developer/forks/op6-refs/wyoming @ 08a0c25

## 1. Python reference write_event -> golden.wyoming (59 typed events) + golden.json
   generator: crates/wyoming/tests/gen_golden.py

## 2. Rust decode of Python bytes (cargo test --test golden_fixtures)

running 2 tests
..
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s


## 3. Rust encode (examples/export_golden) -> Python reference read_event
wrote 59 events
byte-identical: Rust encode == Python write_event bytes
CROSS-CHECK OK: 59 events read by Python reference from Rust encoder

## 4. Full Rust suite
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 18 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 39 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
