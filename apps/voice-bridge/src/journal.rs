//! The trace-events journal sink: every step becomes one JSONL line on the
//! sink (stdout for the CLI, a buffer in tests), and the same events are
//! kept in an [`EventRing`] so tests can assert on the structured journal.
//!
//! Sequence numbers come from the ring (gapless), timestamps are wall-clock
//! microseconds since the Unix epoch, and trace/span ids are allocated from
//! a deterministic counter — no RNG dependency, and a run's JSONL is
//! byte-stable apart from the timestamps.

use std::io::Write;
use std::time::{SystemTime, UNIX_EPOCH};

use trace_events::{Event, EventRing, Payload, SpanId, to_json_line, TraceId};

/// Journal capacity: a voice session emits a handful of events per second;
/// 8192 covers hours of chunk levels without evicting the tail.
const RING_CAPACITY: usize = 8192;

/// Wall clock in microseconds since the Unix epoch.
pub fn now_us() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_micros() as u64)
        .unwrap_or(0)
}

/// Producer identity stamped on every event (the trace-events `service` field),
/// so a consumer can filter and rate-limit this app in the shared journal.
pub const SERVICE_NAME: &str = "voice-bridge";

pub struct Journal<W: Write> {
    ring: EventRing,
    sink: W,
    next_id: u64,
    service: String,
}

impl<W: Write> Journal<W> {
    pub fn new(sink: W) -> Self {
        Self::with_service(sink, SERVICE_NAME.to_string())
    }

    /// As [`Journal::new`], with an explicit producer id for embedding the
    /// bridge under a different service name.
    pub fn with_service(sink: W, service: String) -> Self {
        Self { ring: EventRing::new(RING_CAPACITY), sink, next_id: 1, service }
    }

    /// Allocate a session trace id.
    pub fn new_trace(&mut self) -> TraceId {
        let id = self.next_id;
        self.next_id += 1;
        TraceId(id as u128)
    }

    /// Allocate a span id inside the current trace.
    pub fn new_span(&mut self) -> SpanId {
        let id = self.next_id;
        self.next_id += 1;
        SpanId(id)
    }

    /// Append one event: stamped, sequenced, written as a JSONL line, flushed.
    pub fn emit(
        &mut self,
        trace: TraceId,
        span: Option<SpanId>,
        parent: Option<SpanId>,
        payload: Payload,
    ) -> Result<(), std::io::Error> {
        let event = Event {
            seq: 0, // assigned by the ring
            ts_us: now_us(),
            service: Some(self.service.clone()),
            trace_id: Some(trace),
            span_id: span,
            parent_span_id: parent,
            payload,
        };
        self.ring.push_event(event);
        let last = self.ring.last().expect("ring just pushed");
        let line = to_json_line(last)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        self.sink.write_all(line.as_bytes())?;
        self.sink.write_all(b"\n")?;
        self.sink.flush()?;
        Ok(())
    }

    /// All events emitted so far, oldest → newest (for test assertions).
    pub fn events(&self) -> Vec<Event> {
        self.ring.snapshot()
    }

    /// Borrow the raw sink (e.g. the byte buffer in tests) so the written
    /// JSONL itself can be inspected.
    pub fn sink(&self) -> &W {
        &self.sink
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use trace_events::{from_jsonl, HandoffStatus, Op6Event};

    #[test]
    fn emitted_lines_parse_back_with_gapless_seq() {
        let mut journal = Journal::new(Vec::<u8>::new());
        let trace = journal.new_trace();
        let span = journal.new_span();
        journal
            .emit(trace, None, None, Payload::Op6(Op6Event::MicStarted { device: None }))
            .unwrap();
        journal
            .emit(
                trace,
                Some(span),
                None,
                Payload::Op6(Op6Event::Handoff {
                    destination: "stt@127.0.0.1:1".into(),
                    reason: "test".into(),
                    status: HandoffStatus::Started,
                }),
            )
            .unwrap();

        let text = String::from_utf8(journal.sink.clone()).unwrap();
        let parsed = from_jsonl(&text).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].seq, 0);
        assert_eq!(parsed[1].seq, 1);
        assert_eq!(parsed, journal.events());
        assert_eq!(parsed[1].span_id, Some(span));
        assert_eq!(parsed[1].trace_id, Some(trace));
        assert!(parsed[0].ts_us > 1_600_000_000_000_000, "ts_us must be wall clock");
    }

    #[test]
    fn every_event_carries_the_producer_identity() {
        let mut journal = Journal::new(Vec::<u8>::new());
        let trace = journal.new_trace();
        journal
            .emit(trace, None, None, Payload::Op6(Op6Event::MicStarted { device: None }))
            .unwrap();
        journal
            .emit(trace, None, None, Payload::Op6(Op6Event::MicFrame { level: 0.5 }))
            .unwrap();

        let text = String::from_utf8(journal.sink.clone()).unwrap();
        for event in from_jsonl(&text).unwrap() {
            assert_eq!(event.service.as_deref(), Some(SERVICE_NAME));
        }
        // A line on the wire must name its producer, not just the in-memory copy.
        assert!(text.lines().all(|l| l.contains(r#""service":"voice-bridge""#)), "{text}");
    }

    #[test]
    fn producer_identity_is_overridable() {
        let mut journal = Journal::with_service(Vec::<u8>::new(), "voice-bridge-stt".into());
        let trace = journal.new_trace();
        journal
            .emit(trace, None, None, Payload::Op6(Op6Event::MicStarted { device: None }))
            .unwrap();
        assert_eq!(journal.events()[0].service.as_deref(), Some("voice-bridge-stt"));
    }

    #[test]
    fn ids_are_unique_and_monotonic() {
        let mut journal = Journal::new(Vec::<u8>::new());
        let a = journal.new_trace();
        let c = journal.new_trace();
        let b = journal.new_span();
        let d = journal.new_span();
        assert_ne!(a, c);
        assert!(a.0 < c.0);
        assert_ne!(b, d);
        assert!(b.0 < d.0);
    }
}
