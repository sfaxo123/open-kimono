//! Bounded in-memory trace journal shared by the Linux and Redox transports.
//! Subscribers hold cursors, never queues: a stalled reader cannot block publish.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use trace_events::{Event, EventRing, Op6Event, Payload, SpanId};

pub const MAX_LINE_BYTES: usize = 4096;
const MAX_SERVICES: usize = 64;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Filter {
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub service: Option<String>,
    #[serde(default)]
    pub span: Option<SpanId>,
}

impl Filter {
    fn matches(&self, event: &Event) -> bool {
        let kind = match event.payload {
            Payload::Op6(_) => "op6",
            Payload::AgUi(_) => "ag_ui",
            Payload::GenAi(_) => "gen_ai",
        };
        self.kind.as_deref().is_none_or(|value| value == kind)
            && self
                .service
                .as_deref()
                .is_none_or(|value| event.service.as_deref() == Some(value))
            && self.span.is_none_or(|value| event.span_id == Some(value))
    }
}

#[derive(Clone, Debug)]
pub struct Subscription {
    next_seq: u64,
    filter: Filter,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PublishError {
    RateLimited,
    TooLarge,
    TooManyServices,
}

#[derive(Clone, Copy)]
struct Rate {
    second: u64,
    count: usize,
}

pub struct Journal {
    ring: EventRing,
    rates: HashMap<String, Rate>,
    per_service_per_second: usize,
    rate_dropped: u64,
}

impl Journal {
    pub fn new(capacity: usize, per_service_per_second: usize) -> Self {
        assert!(per_service_per_second > 0);
        Self {
            ring: EventRing::new(capacity),
            rates: HashMap::new(),
            per_service_per_second,
            rate_dropped: 0,
        }
    }

    pub fn publish(&mut self, event: Event, now_us: u64) -> Result<u64, PublishError> {
        if trace_events::to_json_line(&event)
            .map_err(|_| PublishError::TooLarge)?
            .len()
            > MAX_LINE_BYTES
        {
            return Err(PublishError::TooLarge);
        }
        let second = now_us / 1_000_000;
        self.rates.retain(|_, rate| rate.second == second);
        let service = event.service.as_deref().unwrap_or("");
        if !self.rates.contains_key(service) && self.rates.len() >= MAX_SERVICES {
            self.rate_dropped += 1;
            return Err(PublishError::TooManyServices);
        }
        let rate = self
            .rates
            .entry(service.to_owned())
            .or_insert(Rate { second, count: 0 });
        if rate.count >= self.per_service_per_second {
            self.rate_dropped += 1;
            return Err(PublishError::RateLimited);
        }
        rate.count += 1;
        Ok(self.ring.push_event(event))
    }

    /// `backlog` is a maximum event count, capped by ring capacity.
    pub fn subscribe(&self, backlog: usize, filter: Filter) -> Subscription {
        Subscription {
            next_seq: self
                .ring
                .next_seq()
                .saturating_sub(backlog.min(self.ring.capacity()) as u64),
            filter,
        }
    }

    /// Returns retained matching events and advances the cursor. A gap is a
    /// synthetic `journal_dropped` event before subsequent retained records.
    pub fn poll(&self, sub: &mut Subscription, limit: usize) -> Vec<Event> {
        if limit == 0 {
            return Vec::new();
        }
        let mut out = Vec::new();
        if let Some(oldest) = self.ring.oldest_seq() {
            if sub.next_seq < oldest {
                out.push(Event {
                    seq: oldest - 1,
                    ts_us: self.ring.iter().next().map_or(0, |event| event.ts_us),
                    service: None,
                    trace_id: None,
                    span_id: None,
                    parent_span_id: None,
                    payload: Payload::Op6(Op6Event::JournalDropped {
                        count: oldest - sub.next_seq,
                        reason: "subscriber_backpressure".into(),
                    }),
                });
                sub.next_seq = oldest;
                if out.len() == limit {
                    return out;
                }
            }
        }
        for event in self.ring.iter() {
            if event.seq < sub.next_seq {
                continue;
            }
            sub.next_seq = event.seq + 1;
            if sub.filter.matches(event) {
                out.push(event.clone());
            }
            if out.len() == limit {
                break;
            }
        }
        out
    }

    pub fn len(&self) -> usize {
        self.ring.len()
    }
    pub fn ring_dropped(&self) -> u64 {
        self.ring.dropped()
    }
    pub fn rate_dropped(&self) -> u64 {
        self.rate_dropped
    }
    pub fn next_seq(&self) -> u64 {
        self.ring.next_seq()
    }
}

impl Subscription {
    pub fn next_seq(&self) -> u64 {
        self.next_seq
    }
}
