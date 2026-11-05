use trace_events::{Event, Op6Event, Payload, SpanId};
use tracejournal::{Filter, Journal, PublishError};

fn event(service: &str, span: u64) -> Event {
    Event {
        seq: 900,
        ts_us: 1,
        service: Some(service.into()),
        trace_id: None,
        span_id: Some(SpanId(span)),
        parent_span_id: None,
        payload: Payload::Op6(Op6Event::MicStarted { device: None }),
    }
}

#[test]
fn backlog_then_live_is_ordered() {
    let mut j = Journal::new(4, 10);
    for n in 0..3 {
        j.publish(event("voice", n), 1).unwrap();
    }
    let mut sub = j.subscribe(2, Filter::default());
    assert_eq!(
        j.poll(&mut sub, 10)
            .iter()
            .map(|e| e.seq)
            .collect::<Vec<_>>(),
        [1, 2]
    );
    j.publish(event("voice", 3), 1).unwrap();
    assert_eq!(j.poll(&mut sub, 10)[0].seq, 3);
}

#[test]
fn slow_consumer_gets_drop_event_without_blocking_producer() {
    let mut j = Journal::new(2, 20);
    let mut sub = j.subscribe(0, Filter::default());
    for n in 0..8 {
        j.publish(event("voice", n), 1).unwrap();
    }
    let batch = j.poll(&mut sub, 10);
    assert!(matches!(
        &batch[0].payload,
        Payload::Op6(Op6Event::JournalDropped { count: 6, .. })
    ));
    assert_eq!(batch[1].seq, 6);
    assert_eq!(batch[2].seq, 7);
    assert_eq!(j.ring_dropped(), 6);
}

#[test]
fn filters_kind_service_and_span() {
    let mut j = Journal::new(8, 10);
    j.publish(event("other", 4), 1).unwrap();
    j.publish(event("voice", 5), 1).unwrap();
    j.publish(event("voice", 4), 1).unwrap();
    let filter = Filter {
        kind: Some("op6".into()),
        service: Some("voice".into()),
        span: Some(SpanId(4)),
    };
    let mut sub = j.subscribe(8, filter);
    let batch = j.poll(&mut sub, 10);
    assert_eq!(batch.len(), 1);
    assert_eq!(batch[0].seq, 2);
}

#[test]
fn rate_limit_is_per_service_and_resets_each_second() {
    let mut j = Journal::new(8, 2);
    j.publish(event("a", 1), 1).unwrap();
    j.publish(event("a", 2), 1).unwrap();
    assert_eq!(j.publish(event("a", 3), 1), Err(PublishError::RateLimited));
    j.publish(event("b", 3), 1).unwrap();
    j.publish(event("a", 4), 1_000_001).unwrap();
    assert_eq!(j.rate_dropped(), 1);
}

#[test]
fn oversized_and_malformed_events_are_rejected() {
    let mut j = Journal::new(8, 2);
    let mut huge = event("a", 1);
    huge.service = Some("a".repeat(5000));
    assert_eq!(j.publish(huge, 1), Err(PublishError::TooLarge));
    assert!(trace_events::from_json_line("not json").is_err());
    assert_eq!(j.len(), 0);
}

#[test]
fn ring_remains_bounded() {
    let mut j = Journal::new(3, 100);
    for n in 0..20 {
        j.publish(event("a", n), 1).unwrap();
    }
    assert_eq!(j.len(), 3);
    assert_eq!(j.ring_dropped(), 17);
}
