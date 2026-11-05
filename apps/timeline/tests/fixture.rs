//! The demo fixture must parse against the real C2 schema and drive the
//! model to a coherent final screen — this is the `--demo` contract.

use timeline::feed::parse_chunk;
use timeline::model::TimelineModel;

const DEMO: &str = include_str!("../fixtures/demo.jsonl");

#[test]
fn demo_fixture_parses_clean_against_the_c2_schema() {
    let batch = parse_chunk(DEMO);
    assert_eq!(batch.bad_lines, 0, "fixture lines must all parse");
    assert_eq!(batch.events.len(), 28);
    // seq is gapless and ordered in the fixture
    for (i, e) in batch.events.iter().enumerate() {
        assert_eq!(e.seq, i as u64 + 1);
    }
}

#[test]
fn demo_fixture_drives_a_coherent_screen() {
    let batch = parse_chunk(DEMO);
    let mut m = TimelineModel::default();
    for e in &batch.events {
        m.apply(e);
    }

    assert!(!m.mic_live, "mic closed at the end");
    assert_eq!(m.mic_reason.as_deref(), Some("utterance complete"));
    assert_eq!(m.transcript, "Navigate me home.");
    assert!(m.transcript_final);
    assert_eq!(m.cleaned.as_deref(), Some("navigate to home"));
    assert!(m.reasoning_done);
    assert!(m.reasoning.contains("Offline maps cover this area"));
    assert_eq!(m.tools.len(), 1);
    assert_eq!(m.tools[0].name, "navigate_map");
    assert!(m.tools[0].result.as_deref().unwrap().contains("eta_min"));
    assert!(m.services.iter().any(|s| s.name == "maps" && s.running));
    assert!(m.power.iter().any(|p| p.component == "gps" && p.on));
    assert!(m.power.iter().any(|p| p.component == "maps" && p.reason.contains("wake lock")));
    assert_eq!(
        m.handoff.as_ref().map(|h| (h.destination.as_str(), h.status)),
        Some(("homelab:route-cache", trace_events::HandoffStatus::Completed))
    );
    assert_eq!(m.model_span, None, "genai span pair closed");
    assert_eq!(m.error, None);
    assert_eq!(m.events_applied, 28);
    assert_eq!(m.last_seq, Some(28));
}
