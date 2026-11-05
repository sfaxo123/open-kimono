//! JSONL transport: line-per-event, round-trip, blank-line tolerance.

use trace_events::{from_jsonl, to_jsonl, EventRing, Op6Event, Payload};

#[test]
fn jsonl_round_trip_and_line_count() {
    let mut r = EventRing::new(8);
    r.push(1_730_000_000_000_000, Payload::Op6(Op6Event::MicStarted { device: None }));
    r.push(1_730_000_000_001_000, Payload::Op6(Op6Event::MicFrame { level: 0.5 }));
    r.push(
        1_730_000_000_002_000,
        Payload::Op6(Op6Event::MicStopped { reason: "silence".into() }),
    );
    let text = to_jsonl(&r.snapshot()).unwrap();
    assert_eq!(text.lines().count(), 3);
    assert!(text.ends_with('\n'));
    let back = from_jsonl(&text).unwrap();
    assert_eq!(back, r.snapshot());
}

#[test]
fn blank_lines_are_skipped() {
    let one = trace_events::to_json_line(&{
        let mut r = EventRing::new(1);
        r.push(9, Payload::Op6(Op6Event::MicFrame { level: 0.0 }));
        r.snapshot().into_iter().next().unwrap()
    })
    .unwrap();
    let text = format!("\n{one}\n\n");
    let back = from_jsonl(&text).unwrap();
    assert_eq!(back.len(), 1);
    assert_eq!(back[0].seq, 0);
}

#[test]
fn malformed_line_is_an_error() {
    assert!(from_jsonl("{\"seq\":oops}\n").is_err());
}
