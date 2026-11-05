//! Event→view model tests: every screen element must be driven by its events,
//! with the edge cases (clamping, pruning, out-of-order, unknown payloads).

use trace_events::{AgUiEvent, Event, GenAiEvent, HandoffStatus, Op6Event, Payload, PowerState, SpanId, TraceId};

use timeline::model::{ToolState, MAX_POWER, MAX_SERVICES, MAX_TOOLS, TimelineModel};

fn ev(seq: u64, payload: Payload) -> Event {
    Event {
        seq,
        ts_us: 1_764_000_000_000_000 + seq * 1000,
        service: None,
        trace_id: None,
        span_id: None,
        parent_span_id: None,
        payload,
    }
}

fn op6(seq: u64, e: Op6Event) -> Event {
    ev(seq, Payload::Op6(e))
}

fn agui(seq: u64, e: AgUiEvent) -> Event {
    ev(seq, Payload::AgUi(e))
}

#[test]
fn idle_model_is_empty() {
    let m = TimelineModel::default();
    assert!(!m.mic_live);
    assert_eq!(m.mic_level, 0.0);
    assert!(m.transcript.is_empty());
    assert_eq!(m.events_applied, 0);
}

#[test]
fn mic_lifecycle_with_reason() {
    let mut m = TimelineModel::default();
    m.apply(&op6(1, Op6Event::MicStarted { device: Some("mic0".into()) }));
    assert!(m.mic_live);
    assert_eq!(m.mic_reason, None);

    m.apply(&op6(2, Op6Event::MicFrame { level: 0.42 }));
    assert_eq!(m.mic_level, 0.42);

    m.apply(&op6(3, Op6Event::MicStopped { reason: "timeout".into() }));
    assert!(!m.mic_live);
    assert_eq!(m.mic_level, 0.0, "level resets when the mic closes");
    assert_eq!(m.mic_reason.as_deref(), Some("timeout"), "closing is never silent");
}

#[test]
fn mic_level_is_clamped() {
    let mut m = TimelineModel::default();
    m.apply(&op6(1, Op6Event::MicFrame { level: 3.5 }));
    assert_eq!(m.mic_level, 1.0);
    m.apply(&op6(2, Op6Event::MicFrame { level: -2.0 }));
    assert_eq!(m.mic_level, 0.0);
}

#[test]
fn stt_partial_then_final_then_cleaned() {
    let mut m = TimelineModel::default();
    m.apply(&op6(
        1,
        Op6Event::SttPartial { engine: "w".into(), text: "navigat".into(), confidence: None },
    ));
    assert_eq!(m.transcript, "navigat");
    assert!(!m.transcript_final);

    m.apply(&op6(
        2,
        Op6Event::SttFinal { engine: "w".into(), text: "Navigate me home.".into(), confidence: Some(0.9), latency_us: None },
    ));
    assert_eq!(m.transcript, "Navigate me home.");
    assert!(m.transcript_final);

    m.apply(&op6(3, Op6Event::TextCleaned { raw: "Navigate me home.".into(), cleaned: "navigate to home".into() }));
    assert_eq!(m.cleaned.as_deref(), Some("navigate to home"));
}

#[test]
fn reasoning_streams_and_resets_per_message_id() {
    let mut m = TimelineModel::default();
    m.apply(&agui(1, AgUiEvent::ReasoningStart { message_id: "r1".into() }));
    m.apply(&agui(
        2,
        AgUiEvent::ReasoningMessageContent { message_id: "r1".into(), delta: "think ".into() },
    ));
    m.apply(&agui(
        3,
        AgUiEvent::ReasoningMessageContent { message_id: "r1".into(), delta: "hard".into() },
    ));
    assert_eq!(m.reasoning, "think hard");
    assert!(!m.reasoning_done);
    m.apply(&agui(4, AgUiEvent::ReasoningEnd { message_id: "r1".into() }));
    assert!(m.reasoning_done);

    // A second reasoning block replaces the first (one screen, tail only).
    m.apply(&agui(5, AgUiEvent::ReasoningStart { message_id: "r2".into() }));
    m.apply(&agui(6, AgUiEvent::ReasoningMessageChunk { message_id: "r2".into(), delta: Some("new".into()) }));
    assert_eq!(m.reasoning, "new");
    assert_eq!(m.reasoning_id.as_deref(), Some("r2"));
}

#[test]
fn tool_call_full_lifecycle() {
    let mut m = TimelineModel::default();
    m.apply(&agui(1, AgUiEvent::ToolCallStart { tool_call_id: "t1".into(), tool_call_name: "navigate_map".into(), parent_message_id: None }));
    assert_eq!(m.tools.len(), 1);
    assert_eq!(m.tools[0].state, ToolState::Running);

    m.apply(&agui(2, AgUiEvent::ToolCallArgs { tool_call_id: "t1".into(), delta: "{\"to\":".into() }));
    m.apply(&agui(3, AgUiEvent::ToolCallArgs { tool_call_id: "t1".into(), delta: "\"home\"}".into() }));
    assert_eq!(m.tools[0].args, "{\"to\":\"home\"}");

    m.apply(&agui(4, AgUiEvent::ToolCallEnd { tool_call_id: "t1".into() }));
    assert_eq!(m.tools[0].state, ToolState::Ended);

    m.apply(&agui(
        5,
        AgUiEvent::ToolCallResult {
            message_id: "m1".into(),
            tool_call_id: "t1".into(),
            content: serde_json::json!({"eta_min": 12}),
            role: None,
        },
    ));
    assert_eq!(m.tools[0].state, ToolState::Result);
    assert_eq!(m.tools[0].result.as_deref(), Some("{\"eta_min\":12}"));
}

#[test]
fn tool_args_for_unknown_id_are_ignored_and_tools_prune() {
    let mut m = TimelineModel::default();
    m.apply(&agui(1, AgUiEvent::ToolCallArgs { tool_call_id: "ghost".into(), delta: "x".into() }));
    assert!(m.tools.is_empty());

    // Fill past the cap with finished tools; the oldest finished one is pruned.
    for i in 0..(MAX_TOOLS + 2) {
        let id = format!("t{i}");
        m.apply(&agui(10 + i as u64, AgUiEvent::ToolCallStart { tool_call_id: id.clone(), tool_call_name: format!("tool{i}"), parent_message_id: None }));
        m.apply(&agui(20 + i as u64, AgUiEvent::ToolCallEnd { tool_call_id: id.clone() }));
        m.apply(&agui(
            30 + i as u64,
            AgUiEvent::ToolCallResult { message_id: "m".into(), tool_call_id: id, content: serde_json::json!("ok"), role: None },
        ));
    }
    assert_eq!(m.tools.len(), MAX_TOOLS);
    assert_eq!(m.tools.first().unwrap().id, "t2", "oldest finished rows drop first");
}

#[test]
fn services_start_stop_and_prune() {
    let mut m = TimelineModel::default();
    m.apply(&op6(1, Op6Event::ServiceStarted { service: "maps".into(), generation: 1, reason: "nav".into() }));
    m.apply(&op6(2, Op6Event::ServiceStarted { service: "stt".into(), generation: 1, reason: "voice".into() }));
    assert_eq!(m.services.len(), 2);
    assert!(m.services.iter().all(|s| s.running));

    m.apply(&op6(3, Op6Event::ServiceStopped { service: "stt".into(), generation: 1, reason: "utterance done".into() }));
    let stt = m.services.iter().find(|s| s.name == "stt").unwrap();
    assert!(!stt.running);
    assert_eq!(stt.reason, "utterance done");

    // Stop for a never-started service still shows (honest, not silent).
    m.apply(&op6(4, Op6Event::ServiceStopped { service: "mystery".into(), generation: 7, reason: "crashed".into() }));
    assert!(m.services.iter().any(|s| s.name == "mystery" && !s.running));

    for i in 0..(MAX_SERVICES + 3) {
        m.apply(&op6(100 + i as u64, Op6Event::ServiceStarted { service: format!("s{i}"), generation: 1, reason: "r".into() }));
    }
    assert_eq!(m.services.len(), MAX_SERVICES);
    assert_eq!(m.services.last().unwrap().name, format!("s{}", MAX_SERVICES + 2));
}

#[test]
fn power_changes_replace_by_component_with_reason() {
    let mut m = TimelineModel::default();
    m.apply(&op6(1, Op6Event::PowerChange { component: "gps".into(), state: PowerState::On, reason: "navigation active".into() }));
    assert_eq!(m.power.len(), 1);
    assert!(m.power[0].on);
    assert_eq!(m.power[0].reason, "navigation active");

    m.apply(&op6(2, Op6Event::PowerChange { component: "gps".into(), state: PowerState::Off, reason: "arrived".into() }));
    assert_eq!(m.power.len(), 1, "one row per component");
    assert!(!m.power[0].on);
    assert_eq!(m.power[0].reason, "arrived");
}

#[test]
fn wake_locks_join_the_power_list_and_release_removes_them() {
    let mut m = TimelineModel::default();
    m.apply(&op6(1, Op6Event::WakeLockAcquired { holder: "maps".into(), deadline_us: Some(999), max_us: None }));
    assert_eq!(m.power.len(), 1);
    assert!(m.power[0].on);
    assert!(m.power[0].reason.contains("wake lock"));
    assert!(m.power[0].reason.contains("999"));

    m.apply(&op6(2, Op6Event::WakeLockReleased { holder: "maps".into(), reason: None }));
    assert!(m.power.is_empty());

    // Power cap still holds.
    for i in 0..(MAX_POWER + 2) {
        m.apply(&op6(10 + i as u64, Op6Event::PowerChange { component: format!("c{i}"), state: PowerState::On, reason: "r".into() }));
    }
    assert_eq!(m.power.len(), MAX_POWER);
}

#[test]
fn handoff_visible_until_replaced() {
    let mut m = TimelineModel::default();
    m.apply(&op6(1, Op6Event::Handoff { destination: "homelab:stt".into(), reason: "big model".into(), status: HandoffStatus::Started }));
    let h = m.handoff.clone().unwrap();
    assert_eq!(h.destination, "homelab:stt");
    assert_eq!(h.status, HandoffStatus::Started);

    m.apply(&op6(2, Op6Event::Handoff { destination: "homelab:stt".into(), reason: "done".into(), status: HandoffStatus::Completed }));
    assert_eq!(m.handoff.unwrap().status, HandoffStatus::Completed);
}

#[test]
fn run_error_shows_and_next_run_clears_it() {
    let mut m = TimelineModel::default();
    m.apply(&agui(1, AgUiEvent::RunError { message: "model lane down".into(), code: None, usage: vec![] }));
    assert_eq!(m.error.as_deref(), Some("model lane down"));
    m.apply(&agui(2, AgUiEvent::RunStarted { thread_id: "th".into(), run_id: "r".into(), protocol_version: None, parent_run_id: None, input: None }));
    assert_eq!(m.error, None);
}

#[test]
fn genai_span_pair_drives_the_header() {
    let mut m = TimelineModel::default();
    let mut e = ev(
        1,
        Payload::GenAi(GenAiEvent::SpanStart { name: "chat qwen3.8".into(), attributes: Default::default() }),
    );
    e.trace_id = Some(TraceId(0x7a1b));
    e.span_id = Some(SpanId(0x0a));
    m.apply(&e);
    assert_eq!(m.model_span.as_deref(), Some("chat qwen3.8"));

    m.apply(&ev(
        2,
        Payload::GenAi(GenAiEvent::SpanEnd { name: "chat qwen3.8".into(), status: trace_events::SpanStatus::Ok, attributes: Default::default() }),
    ));
    assert_eq!(m.model_span, None);
}

#[test]
fn unmodeled_payloads_are_ignored_not_fatal() {
    let mut m = TimelineModel::default();
    m.apply(&op6(1, Op6Event::GrantGranted { service: "maps".into(), capability: "gps".into(), scope: None, expires_us: None }));
    m.apply(&op6(2, Op6Event::GrantRevoked { service: "maps".into(), capability: "gps".into(), reason: "nav done".into() }));
    m.apply(&agui(3, AgUiEvent::StateSnapshot { snapshot: serde_json::json!({"a":1}) }));
    m.apply(&agui(4, AgUiEvent::TextMessageContent { message_id: "m".into(), delta: "answer".into() }));
    assert_eq!(m.events_applied, 4, "they still count as applied");
    assert!(m.services.is_empty() && m.power.is_empty() && m.tools.is_empty());
}

#[test]
fn journal_drops_accumulate_and_are_never_hidden() {
    let mut m = TimelineModel::default();
    assert_eq!(m.dropped, 0);
    assert_eq!(m.dropped_reason, None);

    m.apply(&op6(1, Op6Event::JournalDropped { count: 3, reason: "ring overflow".into() }));
    assert_eq!(m.dropped, 3);
    assert_eq!(m.dropped_reason.as_deref(), Some("ring overflow"));

    // Notices accumulate — the screen shows the history has a gap, not a fake-complete view.
    m.apply(&op6(2, Op6Event::JournalDropped { count: 12, reason: "producer too fast".into() }));
    assert_eq!(m.dropped, 15);
    assert_eq!(m.dropped_reason.as_deref(), Some("producer too fast"));
}

#[test]
fn journal_position_tracks_events() {
    let mut m = TimelineModel::default();
    m.apply(&op6(7, Op6Event::MicStarted { device: None }));
    m.apply(&op6(8, Op6Event::MicFrame { level: 0.1 }));
    assert_eq!(m.events_applied, 2);
    assert_eq!(m.last_seq, Some(8));
}

#[test]
fn dropped_journal_events_are_visible() {
    let mut m = TimelineModel::default();
    m.apply(&op6(9, Op6Event::JournalDropped { count: 6, reason: "slow subscriber".into() }));
    // main's semantics (2d4b6e1): a drop is its own honesty field, not an error —
    // it accumulates and paints its own DROPPED row, so it cannot be cleared by a
    // later RunStarted the way an error would be.
    assert_eq!(m.dropped, 6);
    assert_eq!(m.dropped_reason.as_deref(), Some("slow subscriber"));
    assert_eq!(m.error, None, "a journal gap is not a run error");
}
