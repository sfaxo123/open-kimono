//! svcd's side of the open-kimono journal: every trigger, step, grant, power
//! switch, bus message and budget stop becomes a `trace-events` record.
//!
//! Mapping: executions are AG-UI runs (`RUN_STARTED`/`RUN_FINISHED`/
//! `RUN_ERROR`, threadId = service, runId = ledger run), actions are steps,
//! commits are `STATE_DELTA`, host calls are tool calls, and svcd-specific
//! facts are `CUSTOM` events named `svc.*`. Lifecycle, grants and power use
//! the `op6` vocabulary. No `gen_ai` event is ever emitted: no LLM runs here.

use serde_json::Value;
use trace_events::{AgUiEvent, Event, EventRing, Op6Event, Payload, SpanId, TraceId};

/// Inline payload cap for action outputs (SERVICE-IR.md §7 rule 7).
pub const INLINE_MAX: usize = 4096;

/// Producer identity stamped on every event (the trace-events `service`
/// field), so a journal reader knows which process produced a record —
/// the same contract voice-bridge (C5) and a2ui-render (C6) fulfil.
pub const SERVICE_NAME: &str = "svcd";

pub struct Journal {
    ring: EventRing,
    wall_anchor_us: u64,
    /// Correlation for the execution in progress.
    pub trace: Option<TraceId>,
    pub span: Option<SpanId>,
}

fn hash_u128(s: &str) -> u128 {
    let h = blake3::hash(s.as_bytes());
    u128::from_le_bytes(h.as_bytes()[..16].try_into().unwrap())
}

impl Journal {
    pub fn new(capacity: usize, wall_anchor_us: u64) -> Self {
        Self { ring: EventRing::new(capacity), wall_anchor_us, trace: None, span: None }
    }

    pub fn ring(&self) -> &EventRing {
        &self.ring
    }

    pub fn push(&mut self, now_ms: u64, payload: Payload) {
        self.ring.push_event(Event {
            seq: 0,
            ts_us: self.wall_anchor_us + now_ms * 1000,
            service: Some(SERVICE_NAME.to_string()),
            trace_id: self.trace,
            span_id: self.span,
            parent_span_id: None,
            payload,
        });
    }

    pub fn op6(&mut self, now_ms: u64, ev: Op6Event) {
        self.push(now_ms, Payload::Op6(ev));
    }

    pub fn agui(&mut self, now_ms: u64, ev: AgUiEvent) {
        self.push(now_ms, Payload::AgUi(ev));
    }

    pub fn custom(&mut self, now_ms: u64, name: &str, value: Value) {
        self.agui(now_ms, AgUiEvent::Custom { name: name.to_string(), value });
    }

    /// Start correlating records with one ledger run.
    pub fn begin_run(&mut self, run_id: &str) {
        self.trace = Some(TraceId(hash_u128(run_id)));
        self.span = None;
    }

    pub fn step(&mut self, run_id: &str, action_id: &str) {
        self.span = Some(SpanId(hash_u128(&format!("{run_id}/{action_id}")) as u64));
    }

    pub fn end_run(&mut self) {
        self.trace = None;
        self.span = None;
    }
}

/// Cap a value's inline size; larger outputs are replaced by a marker.
pub fn inline(v: &Value) -> Value {
    let text = v.to_string();
    if text.len() <= INLINE_MAX {
        v.clone()
    } else {
        let digest = blake3::hash(text.as_bytes()).to_hex();
        serde_json::json!({ "truncated": true, "bytes": text.len(), "blake3": digest.as_str() })
    }
}
