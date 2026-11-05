//! A2 host tests for the kimono fixtures.
//!
//! Positive: every checked-in fixture is valid trace-events JSONL, its `seq`
//! is gapless, and the union of the fixtures covers the whole vocabulary the
//! kimono view claims to render.
//!
//! Negative: each `fixtures/bad/*.jsonl` must fail to parse with a SPECIFIC
//! error — a test that only asserts "some error" is worthless, because a
//! reader that silently skipped bad lines would pass it.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use trace_events::{from_json_line, from_jsonl, Op6Event, Payload, PowerState};

const GOOD: [&str; 4] = ["water_reminder", "fallback_failure", "voice_mic_power", "vocab_coverage"];

/// Every AG-UI `type` discriminator in the schema (crates/trace-events/src/agui.rs).
const AGUI_TYPES: [&str; 31] = [
    "TEXT_MESSAGE_START", "TEXT_MESSAGE_CONTENT", "TEXT_MESSAGE_END", "TEXT_MESSAGE_CHUNK",
    "TOOL_CALL_START", "TOOL_CALL_ARGS", "TOOL_CALL_END", "TOOL_CALL_CHUNK", "TOOL_CALL_RESULT",
    "STATE_SNAPSHOT", "STATE_DELTA", "MESSAGES_SNAPSHOT", "ACTIVITY_SNAPSHOT", "ACTIVITY_DELTA",
    "RAW", "CUSTOM", "RUN_STARTED", "RUN_FINISHED", "RUN_ERROR", "STEP_STARTED", "STEP_FINISHED",
    "REASONING_START", "REASONING_MESSAGE_START", "REASONING_MESSAGE_CONTENT", "REASONING_MESSAGE_END",
    "REASONING_MESSAGE_CHUNK", "REASONING_END", "REASONING_ENCRYPTED_VALUE", "SUBAGENT_STARTED",
    "SUBAGENT_FINISHED", "SUBAGENT_ERROR",
];

/// Every `op6` `type` discriminator in the schema (crates/trace-events/src/op6.rs).
const OP6_TYPES: [&str; 15] = [
    "journal_dropped", "mic_started", "mic_stopped", "mic_frame", "stt_partial", "stt_final",
    "text_cleaned", "power_change", "wake_lock_acquired", "wake_lock_released", "service_started",
    "service_stopped", "grant_granted", "grant_revoked", "handoff",
];

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures").join(format!("{name}.jsonl"))
}

fn read(name: &str) -> String {
    std::fs::read_to_string(fixture(name)).unwrap_or_else(|e| panic!("read {}: {e}", fixture(name).display()))
}

#[test]
fn every_good_fixture_parses_and_is_gapless() {
    for name in GOOD {
        let events = from_jsonl(&read(name)).unwrap_or_else(|e| panic!("{name}: fixture does not parse: {e}"));
        assert!(!events.is_empty(), "{name}: empty fixture");
        let mut expected = 0;
        for ev in &events {
            assert_eq!(ev.seq, expected, "{name}: seq gap or reorder at seq {}", ev.seq);
            expected += 1;
        }
    }
}

#[test]
fn fixtures_cover_the_whole_vocabulary() {
    let mut kinds = BTreeSet::new();
    let mut agui_types = BTreeSet::new();
    let mut op6_types = BTreeSet::new();
    let mut genai_events = BTreeSet::new();

    for name in GOOD {
        for ev in from_jsonl(&read(name)).expect("fixture parses") {
            match &ev.payload {
                Payload::GenAi(g) => {
                    kinds.insert("gen_ai");
                    match g {
                        trace_events::GenAiEvent::SpanStart { .. } => genai_events.insert("span_start"),
                        trace_events::GenAiEvent::SpanEnd { .. } => genai_events.insert("span_end"),
                    };
                }
                Payload::AgUi(a) => {
                    kinds.insert("ag_ui");
                    agui_types.insert(format!("{:?}", a.event_type()));
                }
                Payload::Op6(o) => {
                    kinds.insert("op6");
                    op6_types.insert(op6_type_name(o));
                }
            }
        }
    }

    assert_eq!(kinds, ["ag_ui", "gen_ai", "op6"].into_iter().collect(), "all three vocabularies must be exercised");
    assert_eq!(genai_events, ["span_end", "span_start"].into_iter().collect(), "gen_ai span pairs must be exercised");

    // The fixture set must exercise every discriminator the UI has a case for.
    for t in AGUI_TYPES {
        let rendered = agui_types.contains(&rust_variant_name(t));
        assert!(rendered, "no fixture exercises AG-UI type {t}");
    }
    for t in OP6_TYPES {
        assert!(op6_types.contains(t), "no fixture exercises op6 type {t}");
    }
}

/// `TEXT_MESSAGE_START` -> `TextMessageStart` (the Debug form of the enum).
fn rust_variant_name(discriminator: &str) -> String {
    discriminator
        .split('_')
        .map(|w| {
            let mut chars = w.chars();
            match chars.next() {
                Some(c) => c.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase(),
                None => String::new(),
            }
        })
        .collect()
}

fn op6_type_name(ev: &Op6Event) -> String {
    match ev {
        Op6Event::JournalDropped { .. } => "journal_dropped",
        Op6Event::MicStarted { .. } => "mic_started",
        Op6Event::MicStopped { .. } => "mic_stopped",
        Op6Event::MicFrame { .. } => "mic_frame",
        Op6Event::SttPartial { .. } => "stt_partial",
        Op6Event::SttFinal { .. } => "stt_final",
        Op6Event::TextCleaned { .. } => "text_cleaned",
        Op6Event::PowerChange { .. } => "power_change",
        Op6Event::WakeLockAcquired { .. } => "wake_lock_acquired",
        Op6Event::WakeLockReleased { .. } => "wake_lock_released",
        Op6Event::ServiceStarted { .. } => "service_started",
        Op6Event::ServiceStopped { .. } => "service_stopped",
        Op6Event::GrantGranted { .. } => "grant_granted",
        Op6Event::GrantRevoked { .. } => "grant_revoked",
        Op6Event::Handoff { .. } => "handoff",
    }
    .to_string()
}

#[test]
fn the_voice_fixture_shows_mic_on_and_mic_off_truthfully() {
    // The kimono view renders voiced's journal, so mic truth on the page
    // is exactly these records: the mic shown live while capturing and
    // shown off once capture closes. voiced owns that state (its mic
    // switch file); this fixture is the only kimono-side copy, and it is
    // data, not a second state machine.
    let events = from_jsonl(&read("voice_mic_power")).expect("fixture parses");
    let mic_states: Vec<PowerState> = events
        .iter()
        .filter_map(|e| match &e.payload {
            Payload::Op6(Op6Event::PowerChange { component, state, .. }) if component == "mic" => Some(*state),
            _ => None,
        })
        .collect();
    assert!(mic_states.contains(&PowerState::On), "the fixture must show the mic live: {mic_states:?}");
    assert!(mic_states.contains(&PowerState::Off), "the fixture must show the mic off: {mic_states:?}");
}

// — negatives: the reader must reject these with a NAMED error —

#[test]
fn missing_payload_is_rejected_by_name() {
    let err = from_jsonl(&read("bad/missing_payload")).expect_err("a record with no payload must not parse");
    let msg = err.to_string();
    assert!(msg.contains("payload"), "expected a complaint about the missing payload, got: {msg}");
}

#[test]
fn malformed_span_id_is_rejected_by_name() {
    let err = from_json_line(&read("bad/bad_hex_span")).expect_err("a non-hex span id must not parse");
    let msg = err.to_string();
    assert!(msg.contains("hex"), "expected a complaint about the hex span id, got: {msg}");
}

#[test]
fn truncated_line_is_rejected_as_incomplete() {
    let err = from_json_line(&read("bad/truncated")).expect_err("a truncated line must not parse");
    let msg = err.to_string();
    assert!(msg.contains("EOF while parsing"), "expected an EOF/parse error, got: {msg}");
}

#[test]
fn unknown_op6_type_is_rejected_not_forgiven() {
    let err = from_json_line(&read("bad/unknown_op6_type")).expect_err("an unknown op6 type must not parse");
    let msg = err.to_string();
    assert!(msg.contains("teleported"), "expected the unknown discriminator to be named, got: {msg}");
}
