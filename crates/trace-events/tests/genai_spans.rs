//! OTel GenAI vocabulary: span-name rule, operation enum, attribute keys,
//! untagged attribute values — checked against the spec spellings.

use std::collections::BTreeMap;
use trace_events::{
    from_json_line, genai_keys, to_json_line, AttrMap, AttrValue, Event, GenAiEvent,
    GenAiOperation, Payload, SpanId, SpanStatus, TraceId,
};

#[test]
fn span_name_rule_is_operation_space_target() {
    // `{gen_ai.operation.name} {target}` per spans-deprecated.yaml.
    assert_eq!(GenAiOperation::Chat.span_name("llama-3.1-8b"), "chat llama-3.1-8b");
    assert_eq!(GenAiOperation::ExecuteTool.span_name("set_timer"), "execute_tool set_timer");
    assert_eq!(GenAiOperation::InvokeAgent.span_name("navigator"), "invoke_agent navigator");
    assert_eq!(GenAiOperation::CreateAgent.span_name("timer-bot"), "create_agent timer-bot");
    assert_eq!(GenAiOperation::Retrieval.span_name("offline-maps"), "retrieval offline-maps");
    assert_eq!(GenAiOperation::Embeddings.span_name("bge-small"), "embeddings bge-small");
    assert_eq!(GenAiOperation::GenerateContent.span_name("gpt-4o"), "generate_content gpt-4o");
    assert_eq!(GenAiOperation::TextCompletion.span_name("gpt-3.5"), "text_completion gpt-3.5");
    assert_eq!(GenAiOperation::InvokeWorkflow.span_name("morning"), "invoke_workflow morning");
}

#[test]
fn operation_enum_matches_spec_values() {
    // All nine gen_ai.operation.name values from registry-deprecated.yaml.
    let all = [
        ("chat", GenAiOperation::Chat),
        ("generate_content", GenAiOperation::GenerateContent),
        ("text_completion", GenAiOperation::TextCompletion),
        ("embeddings", GenAiOperation::Embeddings),
        ("retrieval", GenAiOperation::Retrieval),
        ("create_agent", GenAiOperation::CreateAgent),
        ("invoke_agent", GenAiOperation::InvokeAgent),
        ("execute_tool", GenAiOperation::ExecuteTool),
        ("invoke_workflow", GenAiOperation::InvokeWorkflow),
    ];
    for (s, op) in all {
        assert_eq!(op.as_str(), s);
        assert_eq!(GenAiOperation::parse(s), Some(op));
        assert_eq!(serde_json::to_string(&op).unwrap(), alloc::format!("\"{s}\""));
        assert_eq!(serde_json::from_str::<GenAiOperation>(&alloc::format!("\"{s}\"")).unwrap(), op);
    }
    assert_eq!(GenAiOperation::parse("chat "), None);
    assert!(serde_json::from_str::<GenAiOperation>("\"nonsense\"").is_err());
}

#[test]
fn attribute_keys_are_spec_spellings() {
    // Spot-check the keys against the registry names (they must match exactly).
    assert_eq!(genai_keys::OPERATION, "gen_ai.operation.name");
    assert_eq!(genai_keys::PROVIDER, "gen_ai.provider.name");
    assert_eq!(genai_keys::REQUEST_MODEL, "gen_ai.request.model");
    assert_eq!(genai_keys::TOOL_CALL_ID, "gen_ai.tool.call.id");
    assert_eq!(genai_keys::TOOL_CALL_ARGUMENTS, "gen_ai.tool.call.arguments");
    assert_eq!(genai_keys::TOOL_CALL_RESULT, "gen_ai.tool.call.result");
    assert_eq!(genai_keys::USAGE_INPUT_TOKENS, "gen_ai.usage.input_tokens");
    assert_eq!(genai_keys::USAGE_OUTPUT_TOKENS, "gen_ai.usage.output_tokens");
    assert_eq!(genai_keys::RESPONSE_FINISH_REASONS, "gen_ai.response.finish_reasons");
    assert_eq!(genai_keys::DATA_SOURCE_ID, "gen_ai.data_source.id");
}

fn attrs(pairs: Vec<(&'static str, AttrValue)>) -> AttrMap {
    pairs.into_iter().map(|(k, v)| (k.into(), v)).collect()
}

#[test]
fn span_start_json_shape_and_attr_types() {
    let ev = GenAiEvent::span_start(
        GenAiOperation::Chat,
        "llama-3.1-8b",
        attrs(vec![
            (genai_keys::OPERATION, "chat".into()),
            (genai_keys::PROVIDER, "llama.cpp".into()),
            (genai_keys::REQUEST_MAX_TOKENS, 512i64.into()),
            (genai_keys::REQUEST_STREAM, true.into()),
            (genai_keys::REQUEST_TEMPERATURE, 0.7f64.into()),
        ]),
    );
    let event = Event {
        seq: 3,
        ts_us: 1_730_000_000_000_000,
        service: None,
        trace_id: Some(TraceId(1)),
        span_id: Some(SpanId(2)),
        parent_span_id: None,
        payload: Payload::GenAi(ev.clone()),
    };
    let json = to_json_line(&event).unwrap();
    assert_eq!(
        json,
        r#"{"seq":3,"ts_us":1730000000000000,"trace_id":"00000000000000000000000000000001","span_id":"0000000000000002","payload":{"kind":"gen_ai","event":"span_start","name":"chat llama-3.1-8b","attributes":{"gen_ai.operation.name":"chat","gen_ai.provider.name":"llama.cpp","gen_ai.request.max_tokens":512,"gen_ai.request.stream":true,"gen_ai.request.temperature":0.7}}}"#
    );
    let back = from_json_line(&json).unwrap();
    assert_eq!(back, event);
    // Attribute value types survive the untagged round-trip.
    if let Payload::GenAi(GenAiEvent::SpanStart { attributes, .. }) = back.payload {
        assert_eq!(attributes[genai_keys::REQUEST_MAX_TOKENS], AttrValue::I64(512));
        assert_eq!(attributes[genai_keys::REQUEST_TEMPERATURE], AttrValue::F64(0.7));
        assert_eq!(attributes[genai_keys::REQUEST_STREAM], AttrValue::Bool(true));
        assert_eq!(attributes[genai_keys::PROVIDER], AttrValue::Str("llama.cpp".into()));
    } else {
        panic!("wrong payload");
    }
}

#[test]
fn span_end_carries_status_usage_and_finish_reasons() {
    let mut a = BTreeMap::new();
    a.insert(genai_keys::USAGE_INPUT_TOKENS.into(), AttrValue::I64(120));
    a.insert(genai_keys::USAGE_OUTPUT_TOKENS.into(), AttrValue::I64(48));
    a.insert(
        genai_keys::RESPONSE_FINISH_REASONS.into(),
        AttrValue::Strs(vec!["stop".into()]),
    );
    let ev = GenAiEvent::span_end(GenAiOperation::Chat, "llama-3.1-8b", SpanStatus::Ok, a);
    let json = to_json_line(&Event {
        seq: 4,
        ts_us: 1_730_000_001_000_000,
        service: None,
        trace_id: Some(TraceId(1)),
        span_id: Some(SpanId(2)),
        parent_span_id: None,
        payload: Payload::GenAi(ev),
    })
    .unwrap();
    assert!(json.contains(r#""event":"span_end""#));
    assert!(json.contains(r#""status":"ok""#));
    assert!(json.contains(r#""gen_ai.response.finish_reasons":["stop"]"#));
    assert!(json.contains(r#""gen_ai.usage.input_tokens":120"#));
}

#[test]
fn tool_span_round_trips() {
    let start = GenAiEvent::span_start(
        GenAiOperation::ExecuteTool,
        "set_timer",
        attrs(vec![(genai_keys::TOOL_CALL_ID, "tc-1".into())]),
    );
    let end = GenAiEvent::span_end(
        GenAiOperation::ExecuteTool,
        "set_timer",
        SpanStatus::Error,
        attrs(vec![(genai_keys::TOOL_CALL_RESULT, "timer service not running".into())]),
    );
    for ev in [start, end] {
        let event = Event {
            seq: 0,
            ts_us: 0,
            service: None,
            trace_id: None,
            span_id: None,
            parent_span_id: None,
            payload: Payload::GenAi(ev),
        };
        let json = to_json_line(&event).unwrap();
        assert_eq!(from_json_line(&json).unwrap(), event);
    }
}

extern crate alloc;
