//! Ring buffer semantics: seq assignment, eviction, dropped accounting,
//! chronological tailing.

use trace_events::{Event, EventRing, Op6Event, Payload};

fn frame(level: f32) -> Payload {
    Payload::Op6(Op6Event::MicFrame { level })
}

#[test]
fn seq_assigned_monotonically() {
    let mut r = EventRing::new(8);
    assert_eq!(r.push(1, frame(0.1)), 0);
    assert_eq!(r.push(2, frame(0.2)), 1);
    assert_eq!(r.push(3, frame(0.3)), 2);
    assert_eq!(r.next_seq(), 3);
    assert_eq!(r.oldest_seq(), Some(0));
    assert_eq!(r.last().unwrap().seq, 2);
}

#[test]
fn push_event_overwrites_caller_seq() {
    let mut r = EventRing::new(4);
    let ev = Event {
        seq: 999, // caller-supplied junk must be replaced
        ts_us: 5,
        service: None,
        trace_id: None,
        span_id: None,
        parent_span_id: None,
        payload: frame(0.5),
    };
    assert_eq!(r.push_event(ev), 0);
    assert_eq!(r.last().unwrap().seq, 0);
}

#[test]
fn eviction_keeps_newest_and_counts_dropped() {
    let mut r = EventRing::new(3);
    for i in 0..5 {
        r.push(i, frame(i as f32 / 10.0));
    }
    assert_eq!(r.len(), 3);
    assert_eq!(r.capacity(), 3);
    assert_eq!(r.dropped(), 2);
    let seqs: Vec<u64> = r.iter().map(|e| e.seq).collect();
    assert_eq!(seqs, [2, 3, 4]);
    assert_eq!(r.oldest_seq(), Some(2));
    assert_eq!(r.next_seq(), 5, "seq numbers never reuse evicted values");
}

#[test]
fn since_returns_strictly_newer_in_order() {
    let mut r = EventRing::new(4);
    for i in 0..4 {
        r.push(i, frame(0.0));
    }
    let got: Vec<u64> = r.since(1).map(|e| e.seq).collect();
    assert_eq!(got, [2, 3]);
    assert_eq!(r.since(3).count(), 0);
    // After eviction the window still works against held events only.
    r.push(4, frame(0.0)); // evicts seq 0; held: 1,2,3,4
    let got: Vec<u64> = r.since(0).map(|e| e.seq).collect();
    assert_eq!(got, [1, 2, 3, 4]);
    let got: Vec<u64> = r.since(2).map(|e| e.seq).collect();
    assert_eq!(got, [3, 4]);
}

#[test]
fn clear_keeps_seq_never_reused() {
    let mut r = EventRing::new(2);
    r.push(1, frame(0.0));
    r.clear();
    assert!(r.is_empty());
    assert_eq!(r.push(2, frame(0.0)), 1, "a restart must not reuse seq 0");
}

#[test]
fn iter_is_chronological() {
    let mut r = EventRing::new(3);
    for i in 0..3 {
        r.push(10 + i, frame(i as f32));
    }
    let ts: Vec<u64> = r.iter().map(|e| e.ts_us).collect();
    assert_eq!(ts, [10, 11, 12]);
}

#[test]
#[should_panic(expected = "capacity must be > 0")]
fn zero_capacity_panics() {
    EventRing::new(0);
}
