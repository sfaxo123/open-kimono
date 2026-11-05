//! Fixed-capacity journal ring (PLAN B1: the `trace:` scheme tail shape).
//!
//! The ring owns sequence numbers: every pushed event gets the next `seq`,
//! gapless for the life of the journal. When full, the oldest event is
//! evicted and counted in [`EventRing::dropped`] — the journal never lies
//! about what it lost (VISION §5: honest and explained). Consumers tail with
//! [`EventRing::since`].

use alloc::collections::VecDeque;
use alloc::vec::Vec;

use crate::envelope::{Event, Payload};

#[derive(Clone, Debug)]
pub struct EventRing {
    buf: VecDeque<Event>,
    capacity: usize,
    next_seq: u64,
    dropped: u64,
}

impl EventRing {
    /// Panics on a zero capacity — a journal that can hold nothing is a bug.
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "EventRing capacity must be > 0");
        Self { buf: VecDeque::new(), capacity, next_seq: 0, dropped: 0 }
    }

    /// Append an event, assigning the next `seq`. Returns the assigned seq.
    pub fn push(&mut self, ts_us: u64, payload: Payload) -> u64 {
        self.push_event(Event {
            seq: 0,
            ts_us,
            service: None,
            trace_id: None,
            span_id: None,
            parent_span_id: None,
            payload,
        })
    }

    /// Append a fully-built event; its `seq` field is overwritten with the
    /// journal's next sequence number. Returns the assigned seq.
    pub fn push_event(&mut self, mut event: Event) -> u64 {
        let seq = self.next_seq;
        self.next_seq += 1;
        event.seq = seq;
        self.buf.push_back(event);
        while self.buf.len() > self.capacity {
            self.buf.pop_front();
            self.dropped += 1;
        }
        seq
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Events evicted by capacity so far.
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// The seq the next pushed event will receive.
    pub fn next_seq(&self) -> u64 {
        self.next_seq
    }

    /// Seq of the oldest event still held, if any.
    pub fn oldest_seq(&self) -> Option<u64> {
        self.buf.front().map(|e| e.seq)
    }

    /// Newest event still held, if any.
    pub fn last(&self) -> Option<&Event> {
        self.buf.back()
    }

    /// All held events, oldest → newest.
    pub fn iter(&self) -> impl Iterator<Item = &Event> {
        self.buf.iter()
    }

    /// Held events with `seq > after`, oldest → newest (for tailing clients).
    pub fn since(&self, after: u64) -> impl Iterator<Item = &Event> + '_ {
        self.buf.iter().filter(move |e| e.seq > after)
    }

    /// Snapshot of held events, oldest → newest.
    pub fn snapshot(&self) -> Vec<Event> {
        self.buf.iter().cloned().collect()
    }

    /// Drop all held events. Sequence numbers are **not** reset — a journal
    /// never reuses a seq, so late tailers can tell a restart from a gap.
    pub fn clear(&mut self) {
        self.buf.clear();
    }
}
