//! `trace-events` — the open-kimono event journal schema for
//! Redox-on-OnePlus-6 (task C2, docs/PLAN.md B1).
//!
//! Every interesting thing the phone does is one journal record: mic frame
//! levels, live transcripts, cleaned text, model spans, tool calls, grants,
//! service starts/stops, power changes. The schema is copied, not invented
//! (PLAN Part B rule):
//!
//! - **`gen_ai`** payloads follow the OpenTelemetry GenAI semantic
//!   conventions: span names `{gen_ai.operation.name} {target}` and the
//!   `gen_ai.*` attribute keys (`op6-refs/otel-semconv`, Apache-2.0).
//! - **`ag_ui`** payloads follow the AG-UI protocol event vocabulary:
//!   `{"type":"TEXT_MESSAGE_START",...}` with camelCase fields
//!   (`op6-refs/ag-ui` spec/1.0, MIT).
//! - **`op6`** payloads are our own mic/STT/power/service/grant/hand-off
//!   events, each carrying its reason (VISION §3–§5).
//!
//! The journal itself is an [`EventRing`]: fixed capacity, monotonic `seq`,
//! explicit `dropped` counter, tailable with `since`. Transport is JSONL —
//! one [`Event`] per line — the shape the `trace:` scheme serves to the UI.
//!
//! ```
//! use trace_events::{EventRing, Op6Event, Payload, to_jsonl, from_jsonl};
//!
//! let mut journal = EventRing::new(1024);
//! journal.push(1_730_000_000_000_000, Payload::Op6(Op6Event::MicStarted { device: None }));
//! journal.push(1_730_000_000_100_000, Payload::Op6(Op6Event::MicFrame { level: 0.25 }));
//! let text = to_jsonl(&journal.snapshot()).unwrap();
//! let back = from_jsonl(&text).unwrap();
//! assert_eq!(back.len(), 2);
//! ```

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

mod agui;
mod envelope;
mod genai;
mod jsonl;
mod op6;
mod ring;

pub use agui::{
    AgUiEvent, EncryptedValueSubtype, EventType, Interrupt, RunFinishedOutcome, SubagentFinishedOutcome,
    TextMessageRole, TokenUsage,
};
pub use envelope::{Event, Payload, SpanId, TraceId, SCHEMA_VERSION};
pub use genai::{keys as genai_keys, AttrMap, AttrValue, GenAiEvent, GenAiOperation, SpanStatus};
pub use jsonl::{from_json_line, from_jsonl, to_json_line, to_jsonl};
pub use op6::{HandoffStatus, Op6Event, PowerState};
pub use ring::EventRing;
