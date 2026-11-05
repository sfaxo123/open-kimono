//! Scene: the fallback chain and a failure — the two things the kimono view
//! must never hide (PLAN A2: "a clear banner when using the fallback or when
//! something failed").
//!
//! The homelab gateway times out, agentd announces the hand-off and falls
//! back to the on-phone llama.cpp, the local model then emits malformed
//! tool-call arguments, and the run ends in RUN_ERROR. The journal also
//! drops records behind a slow reader, which must surface too.

use serde_json::json;
use trace_events::{AgUiEvent, GenAiOperation, HandoffStatus, Op6Event, SpanStatus};

use crate::ev::*;
use crate::journal::Journal;

const T0: u64 = 1_762_101_800_000_000;

pub fn build() -> Journal {
    let mut j = Journal::new(T0);
    j.service("agentd").trace("6e2f1a9c8b7d4e5f0a1b2c3d4e5f6a7b");

    j.span("0000000000000011");
    j.push(agui(AgUiEvent::RunStarted {
        thread_id: "thread-2".into(),
        run_id: "run-3".into(),
        protocol_version: Some("1.0".into()),
        parent_run_id: None,
        input: Some(json!({ "text": "what's my battery?" })),
    }));
    for p in text_user("msg-user-3", "what's my battery?") {
        j.push(p);
    }

    // — the homelab attempt fails —
    j.span("0000000000000012");
    j.tick(20_000);
    j.push(span_start(
        GenAiOperation::Chat,
        "auto",
        attrs(vec![("gen_ai.provider.name", s("litellm")), ("gen_ai.request.model", s("auto")), ("gen_ai.conversation.id", s("thread-2"))]),
    ));
    j.tick(4_000_000);
    j.push(span_end(
        GenAiOperation::Chat,
        "auto",
        SpanStatus::Error,
        attrs(vec![("error.type", s("connect-timeout")), ("error.message", s("connect timeout after 4000 ms to MESH_HOST:4000"))]),
    ));
    j.close_span();
    j.push(op6(Op6Event::Handoff {
        destination: "homelab-gateway".into(),
        reason: "connect timeout after 4000 ms to MESH_HOST:4000".into(),
        status: HandoffStatus::Failed,
    }));

    // — the fallback, announced —
    j.push(op6(Op6Event::Handoff {
        destination: "on-device llama.cpp (gemma3-1b-it-qat-Q4_0)".into(),
        reason: "homelab unreachable, using local model".into(),
        status: HandoffStatus::Started,
    }));
    j.push(op6(Op6Event::PowerChange {
        component: "model-local".into(),
        state: trace_events::PowerState::On,
        reason: "fallback brain started because the gateway timed out".into(),
    }));
    j.span("0000000000000013");
    j.tick(120_000);
    j.push(span_start(GenAiOperation::Chat, "gemma3-1b-it-qat-Q4_0", attrs(vec![("gen_ai.provider.name", s("llama.cpp")), ("gen_ai.request.model", s("gemma3-1b-it-qat-Q4_0"))])));
    j.tick(2_400_000);
    j.push(span_end(GenAiOperation::Chat, "gemma3-1b-it-qat-Q4_0", SpanStatus::Ok, attrs(vec![("gen_ai.usage.input_tokens", n(380)), ("gen_ai.usage.output_tokens", n(64))])));
    j.close_span();
    j.push(op6(Op6Event::Handoff {
        destination: "on-device llama.cpp (gemma3-1b-it-qat-Q4_0)".into(),
        reason: "local model answered the tool-call turn".into(),
        status: HandoffStatus::Completed,
    }));

    for p in reasoning("reason-2", &["The user wants the battery level. ", "Call phone_status with no arguments."]) {
        j.push(p);
    }

    // — the local model emits malformed tool-call arguments —
    j.push(agui(AgUiEvent::ToolCallStart { tool_call_id: "call-3".into(), tool_call_name: "phone_status".into(), parent_message_id: None }));
    j.push(agui(AgUiEvent::ToolCallArgs { tool_call_id: "call-3".into(), delta: "{\"fields\": [\"battery\"".into() }));
    j.push(agui(AgUiEvent::ToolCallArgs { tool_call_id: "call-3".into(), delta: ",,]}".into() }));
    j.push(agui(AgUiEvent::ToolCallEnd { tool_call_id: "call-3".into() }));

    j.span("0000000000000014");
    j.tick(30_000);
    j.push(span_start(GenAiOperation::ExecuteTool, "phone_status", attrs(vec![("gen_ai.tool.call.id", s("call-3")), ("gen_ai.tool.call.arguments", s("{\"fields\": [\"battery\",,]}"))])));
    j.tick(12_000);
    j.push(span_end(
        GenAiOperation::ExecuteTool,
        "phone_status",
        SpanStatus::Error,
        attrs(vec![("error.type", s("malformed-arguments")), ("error.message", s("tool arguments are not valid JSON: trailing comma at line 1 column 21"))]),
    ));
    j.close_span();
    j.push(agui(AgUiEvent::ToolCallResult {
        message_id: "call-3-result".into(),
        tool_call_id: "call-3".into(),
        content: json!({ "ok": false, "error": "tool arguments are not valid JSON: trailing comma at line 1 column 21" }),
        role: None,
    }));

    // — a slow journal reader, which must not be silent —
    j.push(op6(Op6Event::JournalDropped { count: 3, reason: "kimono SSE reader fell behind by 3 records".into() }));

    // — and the honest stop —
    j.push(agui(AgUiEvent::RunError {
        message: "local model emitted malformed tool-call arguments twice; stopping instead of guessing".into(),
        code: Some("malformed_tool_call".into()),
        usage: usage("gemma3-1b-it-qat-Q4_0", 380, 64),
    }));
    j.push(op6(Op6Event::PowerChange {
        component: "model-local".into(),
        state: trace_events::PowerState::Off,
        reason: "run ended in error; the fallback brain is not needed".into(),
    }));
    j
}
