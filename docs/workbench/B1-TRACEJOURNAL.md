# B1 trace journal evidence — 2026-10-24 5:11 PM PT

Scope: `services/trace` project worktree and `op6/traced` base worktree only.
No phone access, network listener, or push.

## Verification

- `cargo test --manifest-path crates/trace-events/Cargo.toml --quiet`: **36 passed**
  (existing schema suite). New optional `service` and `journal_dropped` preserve
  old JSON parsing.
- `cargo test --manifest-path crates/tracejournal/Cargo.toml --quiet`: **7 passed**
  (6 journal, 1 Unix socket). The journal suite covers ordered backlog/live,
  a slow subscriber receiving a six-record drop event without blocking eight
  publishes, three-field filtering, a three-record ring bound under twenty
  publishes, per-service rate limiting, and oversized/malformed negative inputs.
  The socket test covers publish ACK, backlog, live delivery, and invalid input.
- In `base-b1`: `cargo check -p traced --quiet`, `cargo test -p traced --quiet`,
  and `cargo check -p tracejournal --quiet`: **passed** (host compile; traced has
  no direct host unit tests, core tests run in the project crate).
- `git diff --check` in both worktrees: **passed**.

## Gaps

- No aarch64 Redox target toolchain or QEMU runtime test was run for the scheme.
  Host compilation proves source compatibility only; Redox scheme open/read/write
  and startup remain runtime-unverified.
- Idle Redox reads return `EAGAIN`; readiness notifications are not wired.
  The UI must retry after its next wakeup. The Linux socket stream blocks until
  a new event arrives.
- The journal is memory-only. A daemon restart loses backlog; sender-supplied
  `service` is not authenticated until the capability broker is integrated.

## Reproduction

```sh
cargo test --manifest-path crates/trace-events/Cargo.toml --quiet
cargo test --manifest-path crates/tracejournal/Cargo.toml --quiet
(cd $HOME/Developer/forks/redox/base-b1 && cargo check -p traced --quiet)
```

