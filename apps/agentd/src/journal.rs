//! The live open-kimono journal: a `trace-events` ring plus the SSE listeners
//! currently watching it.
//!
//! One rule from the schema crate carries over: the journal assigns sequence
//! numbers and never lies about what it dropped. agentd adds the second half —
//! every pushed record is also handed to live subscribers, so the kimono view
//! sees a step the moment it happens.

use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

use trace_events::{Event, EventRing, Payload, to_json_line};

use crate::smoke::now_us;

/// Default ring capacity: a day of phone steps, a few hundred KB of RAM.
pub const DEFAULT_CAPACITY: usize = 4096;

#[derive(Default)]
struct State {
    ring: Option<EventRing>,
    subscribers: Vec<Sender<Event>>,
}

/// Cheap to clone: the journal is shared by the HTTP threads and the agent.
#[derive(Clone, Default)]
pub struct Journal {
    state: Arc<Mutex<State>>,
}

impl Journal {
    pub fn new(capacity: usize) -> Journal {
        Journal {
            state: Arc::new(Mutex::new(State { ring: Some(EventRing::new(capacity)), subscribers: Vec::new() })),
        }
    }

    /// Append an event stamped with the current wall clock. Returns its `seq`.
    pub fn push(&self, payload: Payload) -> u64 {
        self.push_at(now_us(), payload)
    }

    /// Append at an explicit timestamp (tests and replay use this).
    pub fn push_at(&self, ts_us: u64, payload: Payload) -> u64 {
        self.push_event(Event {
            seq: 0,
            ts_us,
            service: Some("agentd".to_string()),
            trace_id: None,
            span_id: None,
            parent_span_id: None,
            payload,
        })
    }

    /// Append a fully built event; its `seq` is reassigned by the ring.
    pub fn push_event(&self, mut event: Event) -> u64 {
        let mut state = self.state.lock().expect("journal lock poisoned");
        let ring = state.ring.as_mut().expect("journal ring initialized");
        let seq = ring.push_event(event.clone());
        event.seq = seq;
        // Drop listeners whose reader went away; the journal never blocks on a
        // slow or dead client.
        state.subscribers.retain(|tx| tx.send(event.clone()).is_ok());
        seq
    }

    /// Register a live listener. It receives events pushed after this call;
    /// backfill comes from [`Journal::since`].
    pub fn subscribe(&self) -> Receiver<Event> {
        let (tx, rx) = channel();
        let mut state = self.state.lock().expect("journal lock poisoned");
        state.subscribers.push(tx);
        rx
    }

    /// Held events with `seq > after`, oldest first (SSE backfill / reconnect).
    pub fn since(&self, after: u64) -> Vec<Event> {
        let state = self.state.lock().expect("journal lock poisoned");
        let ring = state.ring.as_ref().expect("journal ring initialized");
        ring.since(after).cloned().collect()
    }

    pub fn snapshot(&self) -> Vec<Event> {
        let state = self.state.lock().expect("journal lock poisoned");
        let ring = state.ring.as_ref().expect("journal ring initialized");
        ring.snapshot()
    }

    pub fn len(&self) -> usize {
        let state = self.state.lock().expect("journal lock poisoned");
        state.ring.as_ref().map(|r| r.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Events evicted by the ring's capacity since startup.
    pub fn dropped(&self) -> u64 {
        let state = self.state.lock().expect("journal lock poisoned");
        state.ring.as_ref().map(|r| r.dropped()).unwrap_or(0)
    }

    /// The whole held journal as JSONL — what a test saves as evidence.
    pub fn jsonl(&self) -> Result<String, serde_json::Error> {
        let mut out = String::new();
        for event in self.snapshot() {
            out.push_str(&to_json_line(&event)?);
            out.push('\n');
        }
        Ok(out)
    }
}
