//! A tiny journal builder for the fixtures: assigns the envelope's `seq` and
//! `ts_us` exactly the way `trace_events::EventRing` does (monotonic seq,
//! wall-clock µs) and carries the current trace/span ids so span pairs and
//! tool calls correlate the way agentd's will.

use trace_events::{Event, Payload, SpanId, TraceId, to_jsonl};

pub struct Journal {
    seq: u64,
    ts_us: u64,
    service: Option<String>,
    trace_id: Option<TraceId>,
    span_id: Option<SpanId>,
    parent_span_id: Option<SpanId>,
    events: Vec<Event>,
}

impl Journal {
    /// Start a journal at a fixed wall clock (µs since epoch) so the fixtures
    /// are byte-stable across runs.
    pub fn new(ts_us: u64) -> Self {
        Journal { seq: 0, ts_us, service: None, trace_id: None, span_id: None, parent_span_id: None, events: Vec::new() }
    }

    /// Set the trace id for subsequent events (32 hex chars).
    pub fn trace(&mut self, hex: &str) -> &mut Self {
        self.trace_id = Some(TraceId::from_hex(hex).expect("fixture trace id must be 32 hex chars"));
        self
    }

    /// Producer identity, e.g. "agentd" or "voiced".
    pub fn service(&mut self, name: &str) -> &mut Self {
        self.service = Some(name.into());
        self
    }

    /// Change the producer identity mid-scene (a scene can span services).
    pub fn set_service(&mut self, name: &str) {
        self.service = Some(name.into());
    }

    /// Open a span: it becomes the current `span_id` and the previous span
    /// becomes its parent (the shape agentd uses for tool spans under a chat).
    pub fn span(&mut self, hex: &str) -> &mut Self {
        self.parent_span_id = self.span_id;
        self.span_id = Some(SpanId::from_hex(hex).expect("fixture span id must be 16 hex chars"));
        self
    }

    /// Close the current span, returning to its parent.
    pub fn close_span(&mut self) -> &mut Self {
        self.span_id = self.parent_span_id;
        self.parent_span_id = None;
        self
    }

    /// Advance the clock by `dt_us` before the next event.
    pub fn tick(&mut self, dt_us: u64) -> &mut Self {
        self.ts_us += dt_us;
        self
    }

    /// Append one event; returns its assigned `seq`.
    pub fn push(&mut self, payload: Payload) -> u64 {
        let seq = self.seq;
        self.events.push(Event {
            seq,
            ts_us: self.ts_us,
            service: self.service.clone(),
            trace_id: self.trace_id,
            span_id: self.span_id,
            parent_span_id: self.parent_span_id,
            payload,
        });
        self.seq += 1;
        seq
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Serialize as JSONL (the SSE `data:` payload shape).
    pub fn jsonl(&self) -> Result<String, String> {
        to_jsonl(&self.events).map_err(|e| e.to_string())
    }
}
