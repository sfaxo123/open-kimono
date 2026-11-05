# K7 SPMI button host evidence

- `cargo test --manifest-path crates/spmid/Cargo.toml --offline`: **7 passed**, including negative cases for missing channel, timeout, bad status, and unsupported arbiter version. The successful strict replay covers PMIC type read, PON status, GPIO5 and GPIO6 reads. Edge tests cover initial baseline, volume edges, power gate and enabled power edges.
- `cargo test -p spmid-core -p spmid --offline` in `base-k7`: **7 passed**, including the same four negative tests.
- `cargo check -p spmid --features host-compile --offline` in `base-k7`: **passed**, compiles the daemon's Redox code path using host libraries.
- `cargo check -p spmid --target aarch64-unknown-redox --offline`: **blocked** (`E0463`: target `core`/`std` unavailable; installed targets are Linux only).

No phone access or QEMU SPMI model was used. The live DT check, Redox aarch64 build, and physical key event confirmation remain open. The daemon is not in an init target and therefore cannot start automatically.
