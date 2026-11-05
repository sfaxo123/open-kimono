//! op6 vocabulary: exact JSON shapes for the open-kimono events and a
//! round-trip over every variant.

use trace_events::{from_json_line, to_json_line, HandoffStatus, Op6Event, PowerState};

#[test]
fn power_change_json_carries_reason() {
    let ev = Op6Event::PowerChange {
        component: "wifi".into(),
        state: PowerState::On,
        reason: "map tile sync needs the homelab".into(),
    };
    let json = to_json_line(&wrap(&ev)).unwrap();
    assert_eq!(
        json,
        r#"{"seq":0,"ts_us":0,"payload":{"kind":"op6","type":"power_change","component":"wifi","state":"on","reason":"map tile sync needs the homelab"}}"#
    );
}

#[test]
fn grant_granted_json_shape() {
    let ev = Op6Event::GrantGranted {
        service: "navigator".into(),
        capability: "audio.capture".into(),
        scope: Some("while-visible".into()),
        expires_us: Some(1_730_000_600_000_000),
    };
    let json = to_json_line(&wrap(&ev)).unwrap();
    assert_eq!(
        json,
        r#"{"seq":0,"ts_us":0,"payload":{"kind":"op6","type":"grant_granted","service":"navigator","capability":"audio.capture","scope":"while-visible","expires_us":1730000600000000}}"#
    );
    assert_eq!(from_json_line(&json).unwrap().payload, trace_events::Payload::Op6(ev));
}

#[test]
fn every_variant_round_trips() {
    let all = vec![
        Op6Event::MicStarted { device: Some("wcd9340".into()) },
        Op6Event::MicStarted { device: None },
        Op6Event::MicStopped { reason: "wake word consumed".into() },
        Op6Event::MicFrame { level: -36.5 },
        Op6Event::SttPartial { engine: "whisper.cpp".into(), text: "turn on the".into(), confidence: Some(0.42) },
        Op6Event::SttFinal { engine: "whisper.cpp".into(), text: "turn on the lamp".into(), confidence: None, latency_us: Some(812_000) },
        Op6Event::TextCleaned { raw: "um turn on the lamp".into(), cleaned: "turn on the lamp".into() },
        Op6Event::PowerChange { component: "cpu4".into(), state: PowerState::Off, reason: "idle".into() },
        Op6Event::WakeLockAcquired { holder: "timer-service".into(), deadline_us: Some(1_730_000_100_000_000), max_us: None },
        Op6Event::WakeLockReleased { holder: "timer-service".into(), reason: Some("expired".into()) },
        Op6Event::ServiceStarted { service: "maps".into(), generation: 3, reason: "navigation request".into() },
        Op6Event::ServiceStopped { service: "maps".into(), generation: 3, reason: "arrived".into() },
        Op6Event::GrantGranted { service: "timer".into(), capability: "power.request-wake".into(), scope: None, expires_us: None },
        Op6Event::GrantRevoked { service: "timer".into(), capability: "power.request-wake".into(), reason: "user cancelled".into() },
        Op6Event::Handoff { destination: "homelab".into(), reason: "answer needs the 5090".into(), status: HandoffStatus::Started },
        Op6Event::Handoff { destination: "homelab".into(), reason: "done".into(), status: HandoffStatus::Completed },
        Op6Event::Handoff { destination: "cloud".into(), reason: "unreachable".into(), status: HandoffStatus::Failed },
    ];
    for ev in all {
        let json = to_json_line(&wrap(&ev)).unwrap();
        assert!(!json.contains('\n'));
        let back = from_json_line(&json).unwrap();
        assert_eq!(back.payload, trace_events::Payload::Op6(ev), "round-trip failed for {json}");
    }
}

#[test]
fn mic_stopped_requires_reason() {
    // `reason` is a required field: a closed mic must explain itself.
    let no_reason = r#"{"seq":0,"ts_us":0,"payload":{"kind":"op6","type":"mic_stopped"}}"#;
    assert!(from_json_line(no_reason).is_err());
    let with_reason = r#"{"seq":0,"ts_us":0,"payload":{"kind":"op6","type":"mic_stopped","reason":"silence"}}"#;
    assert!(from_json_line(with_reason).is_ok());
}

fn wrap(ev: &Op6Event) -> trace_events::Event {
    trace_events::Event {
        seq: 0,
        ts_us: 0,
        service: None,
        trace_id: None,
        span_id: None,
        parent_span_id: None,
        payload: trace_events::Payload::Op6(ev.clone()),
    }
}
