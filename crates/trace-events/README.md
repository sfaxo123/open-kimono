# trace-events

The event journal schema for Redox-on-OnePlus-6 (docs/PLAN.md B1). `no_std` +
`alloc`, serde JSON. One line per event (JSONL), three vocabularies sharing one
envelope:

- **`gen_ai`** — OpenTelemetry GenAI spans: a model/tool/agent call is a
  `span_start`/`span_end` pair correlated by the envelope's `span_id`, named
  `{gen_ai.operation.name} {target}` per the OTel GenAI conventions.
- **`ag_ui`** — AG-UI 1.0 protocol events (agent↔UI): text messages, tool
  calls, state snapshots/deltas, runs/steps, reasoning, subagents. Serialized
  exactly as the AG-UI spec (`"type": "TEXT_MESSAGE_START"`, camelCase), so a
  stock AG-UI consumer can read journal lines unmodified.
- **`op6`** — our own device events: mic/STT, power state and wake locks,
  service lifecycle, capability grants, handoff. Every event carries a
  `reason` where one exists (VISION principles 3–5: open kimono, honest).

```rust
let mut ring = trace_events::EventRing::new(1024);
ring.push(now_us, Payload::Op6(Op6Event::MicStarted { device: None }));
// ... later, drain for the workbench:
let jsonl = trace_events::to_jsonl(ring.iter());
```

The ring buffer assigns a monotonic `seq`, counts what it evicted (`dropped` —
the journal never silently loses), and `since(seq)` tails without re-reading
old lines.

Provenance of the borrowed vocabularies: `docs/PROVENANCE.md` (spec-only
references to OTel semantic-conventions and AG-UI; no code copied).
