# tracejournal / traced-linux

Bounded in-memory event journal for the `trace:` scheme and local development.
Each event is a `trace-events` JSON line. The journal overwrites producer `seq`, retains
256 events of at most 4096 JSON bytes each, and accepts up to 100 events per second
per service. The rate table holds at most 64 active services. No network listener,
persistence, or background worker exists.

Linux: `cargo run --manifest-path crates/tracejournal/Cargo.toml --bin traced-linux -- /tmp/traced.sock`

- Producer: connect to the Unix socket and send newline-terminated `trace-events`
  JSON. Each accepted line returns `{"seq":N}`; invalid input and limits return
  an `{"error":"..."}` line. The producer's `seq` is ignored. Include `service`
  in the event envelope for attribution and rate limits.
- Consumer: connect and send `{"subscribe":{},"backlog":256}` followed by a
  newline. The server sends backlog oldest first, then live JSONL. Optional
  filters are `kind` (`op6`, `gen_ai`, `ag_ui`), `service`, and hexadecimal `span`.
  A stalled consumer never blocks producers. If events are evicted before the
  consumer reads them, the first resumed line is an `op6/journal_dropped`
  event with the exact skipped count.

Redox: `trace:publish` accepts the same JSONL. `trace:subscribe` starts at live
head. `trace:subscribe/{JSON filter}` starts with retained backlog. Reads return
`EAGAIN` when no event is currently available; retry on the next UI wakeup.
The Redox binary vendors the two MIT crates under `base/traced/vendor` so the
base worktree builds independently. Update both copies together.

Current limits: memory-only history is lost on daemon restart. Producer identities
are supplied by the sender, with no capability verification yet. Redox has no
readiness notification for an idle subscription; callers must arrange their own
wakeup. The scheme is host-compiled but has not been run on Redox/QEMU.
