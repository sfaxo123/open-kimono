//! Shorthand constructors so the scene files read like the traces they model.

use serde_json::Value;
use trace_events::{AgUiEvent, AttrMap, AttrValue, GenAiEvent, GenAiOperation, Op6Event, Payload, SpanStatus};

pub fn agui(event: AgUiEvent) -> Payload {
    Payload::AgUi(event)
}
pub fn op6(event: Op6Event) -> Payload {
    Payload::Op6(event)
}
pub fn genai(event: GenAiEvent) -> Payload {
    Payload::GenAi(event)
}

/// `gen_ai` span pair with the spec-shaped name `{operation} {target}`.
pub fn span_start(op: GenAiOperation, target: &str, attributes: AttrMap) -> Payload {
    genai(GenAiEvent::span_start(op, target, attributes))
}
pub fn span_end(op: GenAiOperation, target: &str, status: SpanStatus, attributes: AttrMap) -> Payload {
    genai(GenAiEvent::span_end(op, target, status, attributes))
}

/// Attribute map from `(&str, AttrValue)` pairs.
pub fn attrs(pairs: Vec<(&str, AttrValue)>) -> AttrMap {
    pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
}

pub fn s(v: &str) -> AttrValue {
    AttrValue::Str(v.into())
}
pub fn n(v: i64) -> AttrValue {
    AttrValue::I64(v)
}
pub fn text_user(id: &str, text: &str) -> Vec<Payload> {
    vec![
        agui(AgUiEvent::TextMessageStart { message_id: id.into(), role: Some(trace_events::TextMessageRole::User), name: None }),
        agui(AgUiEvent::TextMessageContent { message_id: id.into(), delta: text.into() }),
        agui(AgUiEvent::TextMessageEnd { message_id: id.into() }),
    ]
}

pub fn text_assistant(id: &str, chunks: &[&str]) -> Vec<Payload> {
    let mut out = vec![agui(AgUiEvent::TextMessageStart {
        message_id: id.into(),
        role: Some(trace_events::TextMessageRole::Assistant),
        name: None,
    })];
    out.extend(chunks.iter().map(|c| agui(AgUiEvent::TextMessageContent { message_id: id.into(), delta: (*c).into() })));
    out.push(agui(AgUiEvent::TextMessageEnd { message_id: id.into() }));
    out
}

pub fn reasoning(id: &str, chunks: &[&str]) -> Vec<Payload> {
    let mut out = vec![
        agui(AgUiEvent::ReasoningStart { message_id: id.into() }),
        agui(AgUiEvent::ReasoningMessageStart { message_id: id.into(), role: "assistant".into() }),
    ];
    out.extend(chunks.iter().map(|c| agui(AgUiEvent::ReasoningMessageContent { message_id: id.into(), delta: (*c).into() })));
    out.push(agui(AgUiEvent::ReasoningMessageEnd { message_id: id.into() }));
    out.push(agui(AgUiEvent::ReasoningEnd { message_id: id.into() }));
    out
}

/// A complete tool call: start, streamed args, end, result.
pub fn tool_call(call_id: &str, name: &str, arg_chunks: &[&str], result: Value) -> Vec<Payload> {
    let mut out = vec![agui(AgUiEvent::ToolCallStart {
        tool_call_id: call_id.into(),
        tool_call_name: name.into(),
        parent_message_id: None,
    })];
    out.extend(arg_chunks.iter().map(|a| agui(AgUiEvent::ToolCallArgs { tool_call_id: call_id.into(), delta: (*a).into() })));
    out.push(agui(AgUiEvent::ToolCallEnd { tool_call_id: call_id.into() }));
    out.push(agui(AgUiEvent::ToolCallResult {
        message_id: format!("{call_id}-result"),
        tool_call_id: call_id.into(),
        content: result,
        role: None,
    }));
    out
}

pub fn usage(model: &str, input: u64, output: u64) -> Vec<trace_events::TokenUsage> {
    vec![trace_events::TokenUsage {
        provider: Some("litellm".into()),
        model: Some(model.into()),
        input_tokens: Some(input),
        output_tokens: Some(output),
        total_tokens: Some(input + output),
        ..Default::default()
    }]
}

pub fn json_str(v: Value) -> String {
    v.to_string()
}
