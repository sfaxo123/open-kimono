//! Scene: vocabulary coverage. Every AG-UI event type and every `gen_ai`
//! operation the schema can carry appears exactly once here, so the headless
//! render test can assert the kimono view has a rendering for all of them —
//! and a defined fallback for anything it has never seen.

use serde_json::json;
use trace_events::{
    AgUiEvent, EncryptedValueSubtype, GenAiOperation, RunFinishedOutcome, SpanStatus, SubagentFinishedOutcome,
    TextMessageRole,
};

use crate::ev::*;
use crate::journal::Journal;

const T0: u64 = 1_762_105_200_000_000;

pub fn build() -> Journal {
    let mut j = Journal::new(T0);
    j.service("agentd").trace("11112222333344445555666677778888");

    // — runs, steps, subagents —
    j.span("0000000000000021");
    j.push(agui(AgUiEvent::RunStarted {
        thread_id: "thread-3".into(),
        run_id: "run-9".into(),
        protocol_version: Some("1.0".into()),
        parent_run_id: None,
        input: Some(json!({ "text": "check the wifi and tell me the ssid" })),
    }));
    j.push(agui(AgUiEvent::StepStarted { step_name: "plan".into() }));
    j.push(agui(AgUiEvent::StepFinished { step_name: "plan".into() }));

    j.push(agui(AgUiEvent::SubagentStarted {
        subagent_run_id: "sub-1".into(),
        name: "net-probe".into(),
        description: Some("reads the wifi state read-only".into()),
        parent_subagent_run_id: None,
        parent_tool_call_id: Some("call-9".into()),
        parent_message_id: None,
    }));
    j.push(agui(AgUiEvent::SubagentFinished {
        subagent_run_id: "sub-1".into(),
        result: Some(json!({ "ssid": "homelab-2.4" })),
        outcome: Some(SubagentFinishedOutcome::Success),
    }));
    j.push(agui(AgUiEvent::SubagentStarted {
        subagent_run_id: "sub-2".into(),
        name: "net-probe".into(),
        description: None,
        parent_subagent_run_id: Some("sub-1".into()),
        parent_tool_call_id: None,
        parent_message_id: None,
    }));
    j.push(agui(AgUiEvent::SubagentError {
        subagent_run_id: "sub-2".into(),
        message: "subagent refused: capability wifi.read not granted".into(),
        code: Some("grant_denied".into()),
    }));

    // — chunked forms of message / tool call / reasoning —
    j.push(agui(AgUiEvent::TextMessageChunk {
        message_id: "msg-chunk-1".into(),
        role: Some(TextMessageRole::Assistant),
        name: None,
        delta: Some("Streaming a chunked answer.".into()),
    }));
    j.push(agui(AgUiEvent::ToolCallChunk {
        tool_call_id: Some("call-9".into()),
        tool_call_name: Some("phone_status".into()),
        parent_message_id: None,
        delta: Some("{\"fields\":[\"wifi\"]}".into()),
    }));
    j.push(agui(AgUiEvent::ReasoningStart { message_id: "reason-3".into() }));
    j.push(agui(AgUiEvent::ReasoningMessageChunk { message_id: "reason-3".into(), delta: Some("chunked reasoning thought".into()) }));
    j.push(agui(AgUiEvent::ReasoningEnd { message_id: "reason-3".into() }));
    j.push(agui(AgUiEvent::ReasoningEncryptedValue {
        subtype: EncryptedValueSubtype::Message,
        entity_id: "reason-3".into(),
        encrypted_value: "gcm.v1.Zm9vYmFy".into(),
    }));

    // — state, messages and activity projections —
    j.push(agui(AgUiEvent::StateSnapshot { snapshot: json!({ "services": [], "power": { "wifi": "on" } }) }));
    j.push(agui(AgUiEvent::StateDelta { delta: json!([{ "op": "add", "path": "/power/bt", "value": "off" }]) }));
    j.push(agui(AgUiEvent::MessagesSnapshot {
        messages: json!([
            { "id": "msg-user-9", "role": "user", "content": "check the wifi and tell me the ssid" },
            { "id": "msg-asst-9", "role": "assistant", "content": "SSID is homelab-2.4." },
        ]),
    }));
    j.push(agui(AgUiEvent::ActivitySnapshot {
        message_id: "act-1".into(),
        activity_type: "wifi-scan".into(),
        content: json!({ "networks": ["homelab-2.4", "neighbor-5g"] }),
        replace: Some(true),
    }));
    j.push(agui(AgUiEvent::ActivityDelta {
        message_id: "act-1".into(),
        activity_type: "wifi-scan".into(),
        patch: json!([{ "op": "add", "path": "/networks/2", "value": "homelab-guest" }]),
    }));

    // — escape hatches —
    j.push(agui(AgUiEvent::Raw { event: json!({ "vendor": "litellm", "hidden": true }), source: Some("litellm-stream".into()) }));
    j.push(agui(AgUiEvent::Custom { name: "op6.battery".into(), value: json!({ "percent": 61, "charging": false }) }));
    // W6: the broker gate's refusal, exactly as agentd journals it (kimono.rs
    // ask_to_commit) — the kimono page must turn this one into a confirm card.
    j.push(agui(AgUiEvent::Custom {
        name: "ask_to_commit".into(),
        value: json!({
            "tool": "service_create",
            "label": "service_create svc.demo.water_reminder",
            "reason": "`service_create svc.demo.water_reminder` changes the phone and needs your confirmation first — repeat the request with \"confirm\": \"service_create svc.demo.water_reminder\" to allow exactly this"
        }),
    }));

    // — the remaining gen_ai operations —
    j.span("0000000000000022");
    j.push(span_start(GenAiOperation::InvokeAgent, "agentd", attrs(vec![("gen_ai.agent.id", s("agentd-1")), ("gen_ai.agent.name", s("agentd"))])));
    j.tick(500_000);
    j.push(span_end(GenAiOperation::InvokeAgent, "agentd", SpanStatus::Ok, attrs(vec![])));
    j.close_span();

    j.span("0000000000000023");
    j.push(span_start(GenAiOperation::Embeddings, "bge-small", attrs(vec![("gen_ai.data_source.id", s("memory"))])));
    j.tick(120_000);
    j.push(span_end(GenAiOperation::Embeddings, "bge-small", SpanStatus::Ok, attrs(vec![])));
    j.close_span();

    j.span("0000000000000024");
    j.push(span_start(GenAiOperation::GenerateContent, "gemma3-1b", attrs(vec![("gen_ai.output.type", s("text"))])));
    j.tick(900_000);
    j.push(span_end(GenAiOperation::GenerateContent, "gemma3-1b", SpanStatus::Ok, attrs(vec![])));
    j.close_span();

    j.span("0000000000000025");
    j.push(span_start(GenAiOperation::TextCompletion, "gemma3-1b", attrs(vec![])));
    j.tick(300_000);
    j.push(span_end(GenAiOperation::TextCompletion, "gemma3-1b", SpanStatus::Ok, attrs(vec![])));
    j.close_span();

    j.span("0000000000000026");
    j.push(span_start(GenAiOperation::Retrieval, "memory-index", attrs(vec![("gen_ai.data_source.id", s("memory"))])));
    j.tick(80_000);
    j.push(span_end(GenAiOperation::Retrieval, "memory-index", SpanStatus::Ok, attrs(vec![])));
    j.close_span();

    j.span("0000000000000027");
    j.push(span_start(GenAiOperation::CreateAgent, "agentd", attrs(vec![("gen_ai.agent.name", s("agentd"))])));
    j.tick(40_000);
    j.push(span_end(GenAiOperation::CreateAgent, "agentd", SpanStatus::Ok, attrs(vec![])));
    j.close_span();

    j.span("0000000000000028");
    j.push(span_start(GenAiOperation::InvokeWorkflow, "morning-routine", attrs(vec![])));
    j.tick(1_500_000);
    j.push(span_end(GenAiOperation::InvokeWorkflow, "morning-routine", SpanStatus::Ok, attrs(vec![])));
    j.close_span();

    // — a run that stops and asks the user something —
    j.push(agui(AgUiEvent::RunFinished {
        thread_id: "thread-3".into(),
        run_id: "run-9".into(),
        result: None,
        outcome: Some(RunFinishedOutcome::Interrupt {
            interrupts: vec![trace_events::Interrupt {
                id: "int-1".into(),
                reason: "needs_permission".into(),
                message: Some("Turn Wi-Fi off to save power? Say yes or no.".into()),
                tool_call_id: Some("call-9".into()),
                response_schema: Some(json!({ "type": "boolean" })),
                expires_at: Some("2026-11-02T15:00:00Z".into()),
                metadata: Some(json!({ "component": "wifi" })),
            }],
        }),
        usage: usage("5090-nothink", 512, 96),
    }));
    j
}
