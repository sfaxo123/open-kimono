# device-control-host notes (module map)

- `lib.rs` — re-exports + crate-level honesty doc.
- `store.rs` — `HostLogStore`: create-exclusive private file, append validated
  framed bytes with quota pre-check, write_all + sync_data, poison on I/O error.
- `replay.rs` — bounded streaming reader: fixed buffer, announced-length cap
  before allocation, corrupt/truncated tail detection, last-good offset.
- `sink.rs` — `ByteSink` trait + real `FileSink` + in-memory `FakeSink` used by
  integration tests to inject write/sync failures into the shared writer path.
