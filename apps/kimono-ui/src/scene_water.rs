//! Scene: the PLAN.md demo sentence, end to end.
//!
//! "remind me every 2 minutes to drink water" → the model calls
//! `service_create` with a ServiceIR body → the service starts and a grant is
//! issued; then "stop the water reminder" → `service_stop` → the service
//! stops and the grant is revoked. This is the trace the final acceptance
//! checks, so the kimono view must render every line of it.

use serde_json::json;
use trace_events::{AgUiEvent, GenAiOperation, Op6Event, RunFinishedOutcome, SpanStatus};

use crate::ev::*;
use crate::journal::Journal;

const T0: u64 = 1_762_100_000_000_000; // 2025-11-02T14:00:00Z, fixed for byte-stable fixtures

pub fn build() -> Journal {
    let mut j = Journal::new(T0);
    j.service("agentd").trace("4bf92f3577b34da6a3ce929d0e0e4736");

    // — run 1: create the water reminder —
    j.span("0000000000000001");
    j.push(agui(AgUiEvent::RunStarted {
        thread_id: "thread-1".into(),
        run_id: "run-1".into(),
        protocol_version: Some("1.0".into()),
        parent_run_id: None,
        input: Some(json!({ "text": "remind me every 2 minutes to drink water" })),
    }));
    for p in text_user("msg-user-1", "remind me every 2 minutes to drink water") {
        j.push(p);
    }

    j.span("0000000000000002");
    j.tick(40_000);
    j.push(span_start(
        GenAiOperation::Chat,
        "auto",
        attrs(vec![
            ("gen_ai.provider.name", s("litellm")),
            ("gen_ai.request.model", s("auto")),
            ("gen_ai.conversation.id", s("thread-1")),
            ("gen_ai.input.messages", s(r#"[{"role":"user","content":"remind me every 2 minutes to drink water"}]"#)),
        ]),
    ));
    j.tick(1_250_000);
    j.push(span_end(
        GenAiOperation::Chat,
        "auto",
        SpanStatus::Ok,
        attrs(vec![("gen_ai.response.model", s("5090-nothink")), ("gen_ai.usage.input_tokens", n(412)), ("gen_ai.usage.output_tokens", n(188))]),
    ));
    j.close_span();

    for p in reasoning("reason-1", &["A recurring reminder is a service, not a one-shot answer. ", "Every 120 s, notify, forever until stopped. ", "Build a ServiceIR with a timer trigger and a notify action."]) {
        j.push(p);
    }

    for p in tool_call(
        "call-1",
        "service_create",
        &[
            r#"{"name":"water-reminder","ir":{"trigger":{"type":"interval","#,
            r#""every_us":120000000},"actions":[{"type":"notify","text":"drink water"}]"#,
            r#","period":"forever"}}}"#,
        ],
        json!({ "ok": true, "service": "water-reminder", "generation": 1, "admitted": true, "simulated": true }),
    ) {
        j.push(p);
    }

    j.span("0000000000000003");
    j.tick(15_000);
    j.push(span_start(GenAiOperation::ExecuteTool, "service_create", attrs(vec![("gen_ai.tool.call.id", s("call-1")), ("gen_ai.tool.call.arguments", s(r#"{"name":"water-reminder","ir":{...}}"#))])));
    j.tick(90_000);
    j.push(span_end(GenAiOperation::ExecuteTool, "service_create", SpanStatus::Ok, attrs(vec![("gen_ai.tool.call.result", s(r#"{"ok":true,"service":"water-reminder","generation":1}"#))])));
    j.close_span();

    j.tick(5_000);
    j.push(op6(Op6Event::ServiceStarted {
        service: "water-reminder".into(),
        generation: 1,
        reason: "user asked for a reminder every 2 minutes; svcd simulated the IR and admitted it".into(),
    }));
    j.push(op6(Op6Event::GrantGranted {
        service: "water-reminder".into(),
        capability: "notify".into(),
        scope: Some("text on the phone screen".into()),
        expires_us: None,
    }));
    j.push(agui(AgUiEvent::StateSnapshot {
        snapshot: json!({
            "services": [{ "name": "water-reminder", "generation": 1, "state": "running", "trigger": "interval 120 s" }],
            "power": { "mic": "off", "local-model": "off", "screen": "on" },
        }),
    }));
    for p in text_assistant("msg-asst-1", &["Water reminder is running: ", "it will ping every 2 minutes. ", "Say \"stop the water reminder\" to remove it."]) {
        j.push(p);
    }
    j.push(agui(AgUiEvent::RunFinished {
        thread_id: "thread-1".into(),
        run_id: "run-1".into(),
        result: Some(json!({ "answer": "Water reminder is running." })),
        outcome: Some(RunFinishedOutcome::Success { pending_tool_call_ids: vec![] }),
        usage: usage("5090-nothink", 412, 188),
    }));
    j.tick(2_000_000);
    j.push(op6(Op6Event::PowerChange {
        component: "model-homelab".into(),
        state: trace_events::PowerState::Off,
        reason: "run finished; no running service needs a model call".into(),
    }));

    // — run 2: stop it —
    j.span("0000000000000004");
    j.tick(41_000_000);
    j.push(agui(AgUiEvent::RunStarted {
        thread_id: "thread-1".into(),
        run_id: "run-2".into(),
        protocol_version: Some("1.0".into()),
        parent_run_id: Some("run-1".into()),
        input: Some(json!({ "text": "stop the water reminder" })),
    }));
    for p in text_user("msg-user-2", "stop the water reminder") {
        j.push(p);
    }
    j.span("0000000000000005");
    j.tick(30_000);
    j.push(span_start(GenAiOperation::Chat, "auto", attrs(vec![("gen_ai.request.model", s("auto")), ("gen_ai.conversation.id", s("thread-1"))])));
    j.tick(980_000);
    j.push(span_end(GenAiOperation::Chat, "auto", SpanStatus::Ok, attrs(vec![("gen_ai.response.model", s("5090-nothink"))])));
    j.close_span();

    for p in tool_call("call-2", "service_stop", &[r#"{"name":"water-reminder"}"#], json!({ "ok": true, "stopped": "water-reminder", "generation": 1 })) {
        j.push(p);
    }
    j.span("0000000000000006");
    j.push(span_start(GenAiOperation::ExecuteTool, "service_stop", attrs(vec![("gen_ai.tool.call.id", s("call-2"))])));
    j.tick(60_000);
    j.push(span_end(GenAiOperation::ExecuteTool, "service_stop", SpanStatus::Ok, attrs(vec![("gen_ai.tool.call.result", s(r#"{"ok":true}"#))])));
    j.close_span();

    j.tick(4_000);
    j.push(op6(Op6Event::ServiceStopped {
        service: "water-reminder".into(),
        generation: 1,
        reason: "user said \"stop the water reminder\"".into(),
    }));
    j.push(op6(Op6Event::GrantRevoked {
        service: "water-reminder".into(),
        capability: "notify".into(),
        reason: "service stopped; its grants go with it".into(),
    }));
    j.push(agui(AgUiEvent::StateDelta {
        delta: json!([{ "op": "replace", "path": "/services", "value": [] }]),
    }));
    for p in text_assistant("msg-asst-2", &["Stopped the water reminder and revoked its notify grant."]) {
        j.push(p);
    }
    j.push(agui(AgUiEvent::RunFinished {
        thread_id: "thread-1".into(),
        run_id: "run-2".into(),
        result: Some(json!({ "answer": "Stopped." })),
        outcome: Some(RunFinishedOutcome::Success { pending_tool_call_ids: vec![] }),
        usage: usage("5090-nothink", 260, 41),
    }));
    j
}
