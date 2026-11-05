//! The audit trail (design R5): every grant decision and every power change is an event, and
//! the trail is a *bounded ring*, because a phone cannot grow a log forever.
//!
//! The reason a ring buffer is in the design instead of an append-only file is honesty: an
//! unbounded trail is a promise nobody can keep, and a trail that silently overwrites while
//! looking complete is worse than none (R5). So: fixed capacity, oldest evicted, and the
//! eviction is *counted* — `dropped` + `dropped_kinds` keep "how much did I lose, and of what
//! kind?" answerable, and `attract_id` (the join key) stays stable even when the detail ages
//! out. That is the whole reason grant numbers are never reused.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::clock::Nanos;
use crate::grant::GrantId;

/// The event kinds the broker emits. These are *provisional names*: `docs/design/CAPS-SANDBOX.md`
/// §8 lists them for C2 (`crates/trace-events`, B1/C2) to align with the OTel GenAI span names
/// (`invoke_agent`, `execute_tool {name}`) so the on-phone feed, the homelab and any viewer
/// interoperate. Keeping the names in one place (here) is the point: if the journal schema and
/// the grant model disagree about what an event is called, R5's transparency contract is a
/// broken promise with two spellings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    /// A new authority exists (or was refused: see `Decision`).
    GrantGranted,
    /// A use of a live grant happened: `at` + grant + what it cost.
    GrantUsed,
    /// A request that was refused, with the reason (`Denied` in `grant.rs`).
    GrantDenied,
    /// A grant's time ran out (deadline or idle timer).
    GrantExpired,
    /// A grant was taken back early, cancelling in-flight work (R4).
    GrantRevoked,
    /// A grant was closed because the service stopped or the route closed (R7).
    GrantReleased,
    /// A wake lock was taken (something stays powered).
    PowerOn,
    /// A wake lock was released, *including* when the platform drops power on the floor's
    /// authority (R8: thermal, panic, watchdog).
    PowerOff,
    /// A wake *window* opened (work happens here).
    WakeAcquire,
    /// A wake window closed.
    WakeRelease,
    /// A service started (R12: start-on-demand).
    ServiceStart,
    /// A service stopped, with the reason (R12: stop-after).
    ServiceStop,
    /// The ring overflowed: the honesty marker. If this ever fires, the trail lied by omission.
    JournalOverflow,
}

impl EventKind {
    /// The name the journal schema uses. Kept as strings so C2 can diff against its own list.
    pub fn name(&self) -> &'static str {
        match self {
            EventKind::GrantGranted => "grant.granted",
            EventKind::GrantUsed => "grant.used",
            EventKind::GrantDenied => "grant.denied",
            EventKind::GrantExpired => "grant.expired",
            EventKind::GrantRevoked => "grant.revoked",
            EventKind::GrantReleased => "grant.released",
            EventKind::PowerOn => "power.on",
            EventKind::PowerOff => "power.off",
            EventKind::WakeAcquire => "wake.acquire",
            EventKind::WakeRelease => "wake.release",
            EventKind::ServiceStart => "service.start",
            EventKind::ServiceStop => "service.stop",
            EventKind::JournalOverflow => "journal.overflow",
        }
    }
    /// R5: a *denied* event is as interesting as a granted one. Nothing here is "just debug".
    pub fn is_decision(&self) -> bool {
        matches!(
            self,
            EventKind::GrantDenied | EventKind::GrantRevoked | EventKind::GrantExpired
        )
    }
}

/// One journal line. `reason` is the human-facing sentence (R5: *why*, not just *what*).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub seq: u64,
    pub at: Nanos,
    pub kind: EventKind,
    pub grant: GrantId,
    pub service: String,
    pub reason: String,
}

impl Event {
    pub fn new(at: Nanos, kind: EventKind, grant: GrantId, service: &str, reason: &str) -> Self {
        Event {
            seq: 0,
            at,
            kind,
            grant,
            service: service.to_string(),
            reason: reason.to_string(),
        }
    }
}

/// The ring. `push` never blocks a caller and never grows the buffer.
#[derive(Debug, Clone)]
pub struct Journal {
    events: Vec<Event>,
    capacity: usize,
    /// Total events ever recorded, so `len()` is not mistaken for "how much happened".
    total: u64,
    /// Events that were evicted for lack of room (counted, not hidden: R5).
    dropped: u64,
    dropped_kinds: Vec<(EventKind, u64)>,
    next_seq: u64,
}

impl Journal {
    pub fn new(capacity: usize) -> Self {
        let cap = if capacity == 0 { 1 } else { capacity };
        Journal {
            events: Vec::with_capacity(cap),
            capacity: cap,
            total: 0,
            dropped: 0,
            dropped_kinds: Vec::new(),
            next_seq: 1,
        }
    }

    /// Record one event, evicting the oldest if the ring is full. Returns the `attract_id`
    /// (sequence number) so a grant can point at the decision that minted it.
    pub fn push(&mut self, event: Event) -> u64 {
        let seq = self.next_seq;
        self.next_seq += 1;
        self.total += 1;
        let mut event = event;
        event.seq = seq;
        if self.events.len() == self.capacity {
            let old = self.events.remove(0);
            self.dropped += 1;
            match self.dropped_kinds.iter_mut().find(|(k, _)| *k == old.kind) {
                Some((_, n)) => *n += 1,
                None => self.dropped_kinds.push((old.kind, 1)),
            }
        }
        self.events.push(event);
        seq
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
    pub fn total(&self) -> u64 {
        self.total
    }
    pub fn capacity(&self) -> usize {
        self.capacity
    }
    pub fn dropped(&self) -> u64 {
        self.dropped
    }
    pub fn dropped_of(&self, kind: EventKind) -> u64 {
        self.dropped_kinds
            .iter()
            .find(|(k, _)| *k == kind)
            .map(|(_, n)| *n)
            .unwrap_or(0)
    }
    pub fn get(&self, seq: u64) -> Option<&Event> {
        self.events.iter().find(|e| e.seq == seq)
    }
    pub fn last(&self) -> Option<&Event> {
        self.events.last()
    }
    pub fn iter(&self) -> impl Iterator<Item = &Event> {
        self.events.iter()
    }
    /// R12's "one clean line": the visible trail is the *tail* of the ring, so the window a
    /// human scrolls through has a fixed size no matter how long the phone has been running.
    pub fn tail(&self, n: usize) -> Vec<Event> {
        let start = self.events.len().saturating_sub(n);
        self.events[start..].to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn journal_keeps_the_tail_and_counts_what_it_lost() {
        // R5: a *bounded* trail that admits overflow beats an unbounded one that lies.
        let mut j = Journal::new(4);
        for i in 0..10 {
            j.push(Event::new(i, EventKind::GrantUsed, 1, "nav", "used"));
        }
        assert_eq!(j.len(), 4, "the ring must stay at its capacity");
        assert_eq!(j.total(), 10, "the count is not the same thing as the length");
        assert_eq!(j.dropped(), 6);
        assert_eq!(j.dropped_of(EventKind::GrantUsed), 6);
        assert_eq!(j.tail(2).len(), 2);
        // Sequence numbers never repeat, so an old row still points at its decision.
        assert!(j.get(1).is_none() && j.get(10).is_some());
        assert!(EventKind::GrantDenied.is_decision());
        assert!(!EventKind::GrantUsed.is_decision());
        assert_eq!(EventKind::GrantDenied.name(), "grant.denied");
    }
}
