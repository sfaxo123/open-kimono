//! AG-UI vocabulary conformance: the 31 EventType discriminators, exact
//! camelCase/SCREAMING_SNAKE JSON (so stock AG-UI consumers can read our
//! lines), required-field enforcement, and the event_type() mapping.

use serde_json::{json, Value};
use trace_events::{
    AgUiEvent, EncryptedValueSubtype, EventType, Interrupt, RunFinishedOutcome,
    SubagentFinishedOutcome, TextMessageRole, TokenUsage,
};

/// The exact 31 `EventType` enum values from op6-refs/ag-ui
/// spec/1.0/schema.json @ 73b04fd (MIT, spec-only).
const SPEC_EVENT_TYPES: [&str; 31] = [
    "TEXT_MESSAGE_START",
    "TEXT_MESSAGE_CONTENT",
    "TEXT_MESSAGE_END",
    "TEXT_MESSAGE_CHUNK",
    "TOOL_CALL_START",
    "TOOL_CALL_ARGS",
    "TOOL_CALL_END",
    "TOOL_CALL_CHUNK",
    "TOOL_CALL_RESULT",
    "STATE_SNAPSHOT",
    "STATE_DELTA",
    "MESSAGES_SNAPSHOT",
    "ACTIVITY_SNAPSHOT",
    "ACTIVITY_DELTA",
    "RAW",
    "CUSTOM",
    "RUN_STARTED",
    "RUN_FINISHED",
    "RUN_ERROR",
    "STEP_STARTED",
    "STEP_FINISHED",
    "REASONING_START",
    "REASONING_MESSAGE_START",
    "REASONING_MESSAGE_CONTENT",
    "REASONING_MESSAGE_END",
    "REASONING_MESSAGE_CHUNK",
    "REASONING_END",
    "REASONING_ENCRYPTED_VALUE",
    "SUBAGENT_STARTED",
    "SUBAGENT_FINISHED",
    "SUBAGENT_ERROR",
];

#[test]
fn event_type_vocab_matches_spec_exactly() {
    for s in SPEC_EVENT_TYPES {
        let parsed: EventType = serde_json::from_str(&format!("\"{s}\"")).unwrap_or_else(|e| panic!("{s}: {e}"));
        assert_eq!(serde_json::to_string(&parsed).unwrap(), format!("\"{s}\""));
    }
    // The vocabulary is closed: nothing else parses.
    assert!(serde_json::from_str::<EventType>("\"THINKING_START\"").is_err());
    assert!(serde_json::from_str::<EventType>("\"text_message_start\"").is_err());
}

#[test]
fn text_message_start_json_is_ag_ui_compatible() {
    let ev = AgUiEvent::TextMessageStart {
        message_id: "m1".into(),
        role: Some(TextMessageRole::Assistant),
        name: None,
    };
    assert_eq!(
        serde_json::to_string(&ev).unwrap(),
        r#"{"type":"TEXT_MESSAGE_START","messageId":"m1","role":"assistant"}"#
    );
}

#[test]
fn tool_call_lifecycle_round_trips() {
    let start = AgUiEvent::ToolCallStart {
        tool_call_id: "tc1".into(),
        tool_call_name: "set_timer".into(),
        parent_message_id: Some("m1".into()),
    };
    assert_eq!(
        serde_json::to_string(&start).unwrap(),
        r#"{"type":"TOOL_CALL_START","toolCallId":"tc1","toolCallName":"set_timer","parentMessageId":"m1"}"#
    );
    let args = AgUiEvent::ToolCallArgs { tool_call_id: "tc1".into(), delta: "{\"secs\":60}".into() };
    let end = AgUiEvent::ToolCallEnd { tool_call_id: "tc1".into() };
    let result = AgUiEvent::ToolCallResult {
        message_id: "m2".into(),
        tool_call_id: "tc1".into(),
        content: json!({"ok": true}),
        role: Some("tool".into()),
    };
    for ev in [start, args, end, result] {
        let s = serde_json::to_string(&ev).unwrap();
        assert_eq!(serde_json::from_str::<AgUiEvent>(&s).unwrap(), ev, "{s}");
    }
}

#[test]
fn run_finished_carries_outcome_and_usage() {
    let ev = AgUiEvent::RunFinished {
        thread_id: "t1".into(),
        run_id: "r1".into(),
        result: None,
        outcome: Some(RunFinishedOutcome::Success { pending_tool_call_ids: vec!["tc9".into()] }),
        usage: vec![TokenUsage {
            provider: Some("llama.cpp".into()),
            model: Some("llama-3.1-8b".into()),
            input_tokens: Some(120),
            output_tokens: Some(48),
            ..Default::default()
        }],
    };
    let s = serde_json::to_string(&ev).unwrap();
    assert_eq!(
        s,
        r#"{"type":"RUN_FINISHED","threadId":"t1","runId":"r1","outcome":{"type":"success","pendingToolCallIds":["tc9"]},"usage":[{"provider":"llama.cpp","model":"llama-3.1-8b","inputTokens":120,"outputTokens":48}]}"#
    );
    assert_eq!(serde_json::from_str::<AgUiEvent>(&s).unwrap(), ev);
}

#[test]
fn run_error_with_interrupt_outcome_round_trips() {
    let ev = AgUiEvent::RunFinished {
        thread_id: "t1".into(),
        run_id: "r2".into(),
        result: None,
        outcome: Some(RunFinishedOutcome::Interrupt {
            interrupts: vec![Interrupt {
                id: "i1".into(),
                reason: "needs-approval".into(),
                message: Some("Turn on the charging? (power.write)".into()),
                tool_call_id: Some("tc3".into()),
                response_schema: None,
                expires_at: None,
                metadata: None,
            }],
        }),
        usage: vec![],
    };
    let s = serde_json::to_string(&ev).unwrap();
    assert!(s.contains(r#""outcome":{"type":"interrupt","interrupts":[{"id":"i1","reason":"needs-approval""#));
    assert_eq!(serde_json::from_str::<AgUiEvent>(&s).unwrap(), ev);
}

#[test]
fn state_delta_is_json_patch_passthrough() {
    let ev = AgUiEvent::StateDelta {
        delta: json!([
            {"op": "replace", "path": "/battery", "value": 18},
            {"op": "add", "path": "/services/-", "value": "maps"}
        ]),
    };
    let s = serde_json::to_string(&ev).unwrap();
    let back: AgUiEvent = serde_json::from_str(&s).unwrap();
    assert_eq!(back, ev);
    let v: Value = serde_json::from_str(&s).unwrap();
    assert_eq!(v["delta"].as_array().unwrap().len(), 2);
}

#[test]
fn reasoning_encrypted_value_subtype_is_kebab_case() {
    let ev = AgUiEvent::ReasoningEncryptedValue {
        subtype: EncryptedValueSubtype::ToolCall,
        entity_id: "tc1".into(),
        encrypted_value: "Zm9v".into(),
    };
    assert_eq!(
        serde_json::to_string(&ev).unwrap(),
        r#"{"type":"REASONING_ENCRYPTED_VALUE","subtype":"tool-call","entityId":"tc1","encryptedValue":"Zm9v"}"#
    );
}

#[test]
fn subagent_suspended_outcome_round_trips() {
    let ev = AgUiEvent::SubagentFinished {
        subagent_run_id: "sa1".into(),
        result: None,
        outcome: Some(SubagentFinishedOutcome::Suspended { interrupt_ids: vec!["i7".into()] }),
    };
    let s = serde_json::to_string(&ev).unwrap();
    assert!(s.contains(r#""outcome":{"type":"suspended","interruptIds":["i7"]}"#));
    assert_eq!(serde_json::from_str::<AgUiEvent>(&s).unwrap(), ev);
}

#[test]
fn required_fields_are_required() {
    // TEXT_MESSAGE_CONTENT without delta, TOOL_CALL_START without name,
    // RUN_ERROR without message — all must fail.
    for bad in [
        r#"{"type":"TEXT_MESSAGE_CONTENT","messageId":"m1"}"#,
        r#"{"type":"TOOL_CALL_START","toolCallId":"tc1"}"#,
        r#"{"type":"RUN_ERROR"}"#,
        r#"{"type":"CUSTOM","name":"x"}"#,
    ] {
        assert!(serde_json::from_str::<AgUiEvent>(bad).is_err(), "accepted: {bad}");
    }
}

/// One instance of every variant, for the sweep below.
fn all_variants() -> Vec<AgUiEvent> {
    vec![
        AgUiEvent::TextMessageStart { message_id: "m".into(), role: Some(TextMessageRole::User), name: Some("jordan".into()) },
        AgUiEvent::TextMessageContent { message_id: "m".into(), delta: "he".into() },
        AgUiEvent::TextMessageEnd { message_id: "m".into() },
        AgUiEvent::TextMessageChunk { message_id: "m".into(), role: None, name: None, delta: Some("llo".into()) },
        AgUiEvent::ToolCallStart { tool_call_id: "t".into(), tool_call_name: "n".into(), parent_message_id: None },
        AgUiEvent::ToolCallArgs { tool_call_id: "t".into(), delta: "{}".into() },
        AgUiEvent::ToolCallEnd { tool_call_id: "t".into() },
        AgUiEvent::ToolCallChunk { tool_call_id: Some("t".into()), tool_call_name: Some("n".into()), parent_message_id: None, delta: None },
        AgUiEvent::ToolCallResult { message_id: "m".into(), tool_call_id: "t".into(), content: json!("done"), role: None },
        AgUiEvent::StateSnapshot { snapshot: json!({"battery": 18}) },
        AgUiEvent::StateDelta { delta: json!([{"op": "add", "path": "/x", "value": 1}]) },
        AgUiEvent::MessagesSnapshot { messages: json!([]) },
        AgUiEvent::ActivitySnapshot { message_id: "m".into(), activity_type: "nav".into(), content: json!({"step": "turn left"}), replace: Some(true) },
        AgUiEvent::ActivityDelta { message_id: "m".into(), activity_type: "nav".into(), patch: json!([]) },
        AgUiEvent::Raw { event: json!({"anything": 1}), source: Some("goose".into()) },
        AgUiEvent::Custom { name: "battery-low".into(), value: json!(18) },
        AgUiEvent::RunStarted { thread_id: "t".into(), run_id: "r".into(), protocol_version: Some("1.0".into()), parent_run_id: None, input: None },
        AgUiEvent::RunFinished { thread_id: "t".into(), run_id: "r".into(), result: Some(json!({"answer": 42})), outcome: Some(RunFinishedOutcome::Cancelled), usage: vec![] },
        AgUiEvent::RunError { message: "model lane down".into(), code: Some("E_LANE".into()), usage: vec![] },
        AgUiEvent::StepStarted { step_name: "plan".into() },
        AgUiEvent::StepFinished { step_name: "plan".into() },
        AgUiEvent::ReasoningStart { message_id: "rm".into() },
        AgUiEvent::ReasoningMessageStart { message_id: "rm".into(), role: "reasoning".into() },
        AgUiEvent::ReasoningMessageContent { message_id: "rm".into(), delta: "the user wants…".into() },
        AgUiEvent::ReasoningMessageEnd { message_id: "rm".into() },
        AgUiEvent::ReasoningMessageChunk { message_id: "rm".into(), delta: Some("…a timer".into()) },
        AgUiEvent::ReasoningEnd { message_id: "rm".into() },
        AgUiEvent::ReasoningEncryptedValue { subtype: EncryptedValueSubtype::Message, entity_id: "rm".into(), encrypted_value: "e".into() },
        AgUiEvent::SubagentStarted { subagent_run_id: "sa".into(), name: "navigator".into(), description: None, parent_subagent_run_id: None, parent_tool_call_id: None, parent_message_id: None },
        AgUiEvent::SubagentFinished { subagent_run_id: "sa".into(), result: None, outcome: Some(SubagentFinishedOutcome::Success) },
        AgUiEvent::SubagentError { subagent_run_id: "sa".into(), message: "boom".into(), code: None },
    ]
}

#[test]
fn every_variant_round_trips_and_event_type_agrees_with_json() {
    assert_eq!(all_variants().len(), 31, "one instance per spec EventType");
    for ev in all_variants() {
        let s = serde_json::to_string(&ev).unwrap();
        let back: AgUiEvent = serde_json::from_str(&s).unwrap_or_else(|e| panic!("{s}: {e}"));
        assert_eq!(back, ev);
        // The event_type() mapping must agree with the serialized discriminator.
        let v: Value = serde_json::from_str(&s).unwrap();
        let tag = v["type"].as_str().unwrap().to_string();
        assert_eq!(serde_json::to_string(&ev.event_type()).unwrap(), format!("\"{tag}\""));
    }
    // And the 31 discriminators are exactly the spec's set.
    let mut tags: Vec<String> = all_variants().iter().map(|e| format!("{:?}", e.event_type())).collect();
    tags.sort();
    tags.dedup();
    assert_eq!(tags.len(), 31);
}
