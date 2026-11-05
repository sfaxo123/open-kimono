//! Envelope + id encoding tests: exact JSON shape and hex id round-trips.

use trace_events::{Event, Op6Event, Payload, SpanId, TraceId};

#[test]
fn trace_id_hex_roundtrip_and_rejects() {
    let id = TraceId(0x4bf9_2f35_77b3_4da6_a3ce_929d_0e0e_4736);
    assert_eq!(id.to_hex(), "4bf92f3577b34da6a3ce929d0e0e4736");
    assert_eq!(TraceId::from_hex("4bf92f3577b34da6a3ce929d0e0e4736"), Some(id));
    // Wrong length, non-hex, and `0x` prefix must all be rejected.
    assert_eq!(TraceId::from_hex("4bf92f3577b34da6a3ce929d0e0e473"), None);
    assert_eq!(TraceId::from_hex("4bf92f3577b34da6a3ce929d0e0e473g"), None);
    assert_eq!(TraceId::from_hex("0x4bf92f3577b34da6a3ce929d0e0e47"), None);
}

#[test]
fn span_id_hex_roundtrip_and_rejects() {
    let id = SpanId(0x0123_4567_89ab_cdef);
    assert_eq!(id.to_hex(), "0123456789abcdef");
    assert_eq!(SpanId::from_hex("0123456789abcdef"), Some(id));
    assert_eq!(SpanId::from_hex("0123456789abcde"), None);
    assert_eq!(SpanId::from_hex("0123456789abcdeg"), None);
}

#[test]
fn event_json_exact_shape() {
    let ev = Event {
        seq: 7,
        ts_us: 1_730_000_000_000_000,
        service: None,
        trace_id: Some(TraceId(7)),
        span_id: Some(SpanId(42)),
        parent_span_id: None,
        payload: Payload::Op6(Op6Event::MicFrame { level: 0.25 }),
    };
    let json = trace_events::to_json_line(&ev).unwrap();
    assert_eq!(
        json,
        r#"{"seq":7,"ts_us":1730000000000000,"trace_id":"00000000000000000000000000000007","span_id":"000000000000002a","payload":{"kind":"op6","type":"mic_frame","level":0.25}}"#
    );
    assert_eq!(trace_events::from_json_line(&json).unwrap(), ev);
}

#[test]
fn event_json_skips_absent_ids() {
    let ev = Event {
        seq: 0,
        ts_us: 1,
        service: None,
        trace_id: None,
        span_id: None,
        parent_span_id: None,
        payload: Payload::Op6(Op6Event::MicStopped { reason: "user asked to stop".into() }),
    };
    let json = trace_events::to_json_line(&ev).unwrap();
    assert!(!json.contains("trace_id"), "absent ids must not appear: {json}");
    assert!(!json.contains("span_id"), "absent ids must not appear: {json}");
    assert_eq!(trace_events::from_json_line(&json).unwrap(), ev);
}

#[test]
fn bad_ids_fail_to_parse() {
    let bad = r#"{"seq":1,"ts_us":1,"trace_id":"nope","payload":{"kind":"op6","type":"mic_frame","level":0.5}}"#;
    assert!(trace_events::from_json_line(bad).is_err());
}
