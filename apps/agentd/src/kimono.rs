//! Journal writers for the shapes agentd produces over and over: a run, a chat
//! span, a tool call, an answer, an error. Keeping them in one place is what
//! makes the open kimono consistent — every request traces the same way, and
//! the tests can assert on the vocabulary instead of on ad-hoc strings.
//!
//! Correlation follows the envelope: one `trace_id` per request, a fresh
//! `span_id` per model call or tool call, and `parent_span_id` pointing back at
//! the chat call that asked for the tool.

use trace_events::{
    AgUiEvent, AttrMap, AttrValue, Event, GenAiEvent, GenAiOperation, HandoffStatus, Payload, SpanId,
    SpanStatus, TextMessageRole, TraceId,
};

use crate::journal::Journal;

/// Deterministic per-request ids: the trace id is derived from the run id, and
/// span ids count up. No RNG, so a recorded run replays byte-for-byte.
pub struct Recorder {
    journal: Journal,
    trace: TraceId,
    next_span: u64,
}

impl Recorder {
    pub fn new(journal: Journal, run_id: &str) -> Recorder {
        Recorder { journal, trace: TraceId(hash128(run_id)), next_span: 1 }
    }

    pub fn trace_id(&self) -> TraceId {
        self.trace
    }

    pub fn next_span(&mut self) -> SpanId {
        let id = SpanId(self.next_span);
        self.next_span += 1;
        id
    }

    /// Push one payload under an explicit span.
    pub fn push(&self, span: Option<SpanId>, parent: Option<SpanId>, payload: Payload) -> u64 {
        self.journal.push_event(Event {
            seq: 0,
            ts_us: crate::smoke::now_us(),
            service: Some("agentd".to_string()),
            trace_id: Some(self.trace),
            span_id: span,
            parent_span_id: parent,
            payload,
        })
    }

    /// A payload with no span of its own (run lifecycle, handoffs, power).
    pub fn push_plain(&self, payload: Payload) -> u64 {
        self.push(None, None, payload)
    }

    pub fn run_started(&self, thread_id: &str, run_id: &str, input: serde_json::Value) -> u64 {
        self.push_plain(Payload::AgUi(AgUiEvent::RunStarted {
            thread_id: thread_id.to_string(),
            run_id: run_id.to_string(),
            protocol_version: None,
            parent_run_id: None,
            input: Some(input),
        }))
    }

    pub fn run_finished(&self, thread_id: &str, run_id: &str, result: serde_json::Value) -> u64 {
        self.push_plain(Payload::AgUi(AgUiEvent::RunFinished {
            thread_id: thread_id.to_string(),
            run_id: run_id.to_string(),
            result: Some(result),
            outcome: None,
            usage: Vec::new(),
        }))
    }

    pub fn run_error(&self, message: &str, code: Option<&str>) -> u64 {
        self.push_plain(Payload::AgUi(AgUiEvent::RunError {
            message: message.to_string(),
            code: code.map(str::to_string),
            usage: Vec::new(),
        }))
    }

    pub fn step(&self, step_name: &str, finished: bool) -> u64 {
        self.push_plain(Payload::AgUi(if finished {
            AgUiEvent::StepFinished { step_name: step_name.to_string() }
        } else {
            AgUiEvent::StepStarted { step_name: step_name.to_string() }
        }))
    }

    /// `chat <model>` span start.
    pub fn chat_start(&mut self, model: &str, provider: &str, conversation_id: &str) -> (SpanId, u64) {
        let span = self.next_span();
        let mut attrs = AttrMap::new();
        attrs.insert(trace_events::genai_keys::OPERATION.to_string(), AttrValue::from("chat"));
        attrs.insert(trace_events::genai_keys::PROVIDER.to_string(), AttrValue::from(provider));
        attrs.insert(trace_events::genai_keys::REQUEST_MODEL.to_string(), AttrValue::from(model));
        attrs.insert(trace_events::genai_keys::CONVERSATION_ID.to_string(), AttrValue::from(conversation_id));
        let seq = self.push(
            Some(span),
            None,
            Payload::GenAi(GenAiEvent::span_start(GenAiOperation::Chat, model, attrs)),
        );
        (span, seq)
    }

    pub fn chat_end(&self, span: SpanId, model: &str, status: SpanStatus, usage: &[(String, i64)], error: Option<&str>) -> u64 {
        let mut attrs = AttrMap::new();
        attrs.insert(trace_events::genai_keys::RESPONSE_MODEL.to_string(), AttrValue::from(model));
        for (key, value) in usage {
            attrs.insert(key.clone(), AttrValue::I64(*value));
        }
        if let Some(reason) = error {
            attrs.insert("error.reason".to_string(), AttrValue::from(reason));
        }
        self.push(
            Some(span),
            None,
            Payload::GenAi(GenAiEvent::span_end(GenAiOperation::Chat, model, status, attrs)),
        )
    }

    /// `execute_tool <name>` span start.
    pub fn tool_start(&mut self, name: &str, call_id: &str, arguments: &str, parent: Option<SpanId>) -> SpanId {
        let span = self.next_span();
        let mut attrs = AttrMap::new();
        attrs.insert(trace_events::genai_keys::TOOL_NAME.to_string(), AttrValue::from(name));
        attrs.insert(trace_events::genai_keys::TOOL_CALL_ID.to_string(), AttrValue::from(call_id));
        attrs.insert(trace_events::genai_keys::TOOL_CALL_ARGUMENTS.to_string(), AttrValue::from(arguments));
        self.push(
            Some(span),
            parent,
            Payload::GenAi(GenAiEvent::span_start(GenAiOperation::ExecuteTool, name, attrs)),
        );
        span
    }

    pub fn tool_end(&self, span: SpanId, name: &str, status: SpanStatus, result: &str, parent: Option<SpanId>) -> u64 {
        let mut attrs = AttrMap::new();
        attrs.insert(trace_events::genai_keys::TOOL_NAME.to_string(), AttrValue::from(name));
        attrs.insert(trace_events::genai_keys::TOOL_CALL_RESULT.to_string(), AttrValue::from(result));
        self.push(
            Some(span),
            parent,
            Payload::GenAi(GenAiEvent::span_end(GenAiOperation::ExecuteTool, name, status, attrs)),
        )
    }

    /// The AG-UI tool-call triple: start, args, end.
    pub fn tool_agui(&self, call_id: &str, name: &str, arguments: &str, parent_message: Option<&str>) {
        self.push_plain(Payload::AgUi(AgUiEvent::ToolCallStart {
            tool_call_id: call_id.to_string(),
            tool_call_name: name.to_string(),
            parent_message_id: parent_message.map(str::to_string),
        }));
        self.push_plain(Payload::AgUi(AgUiEvent::ToolCallArgs {
            tool_call_id: call_id.to_string(),
            delta: arguments.to_string(),
        }));
        self.push_plain(Payload::AgUi(AgUiEvent::ToolCallEnd { tool_call_id: call_id.to_string() }));
    }

    pub fn tool_result(&self, message_id: &str, call_id: &str, content: serde_json::Value) -> u64 {
        self.push_plain(Payload::AgUi(AgUiEvent::ToolCallResult {
            message_id: message_id.to_string(),
            tool_call_id: call_id.to_string(),
            content,
            role: None,
        }))
    }

    /// The answer, as a complete AG-UI text message.
    pub fn answer(&self, message_id: &str, text: &str) {
        self.push_plain(Payload::AgUi(AgUiEvent::TextMessageStart {
            message_id: message_id.to_string(),
            role: Some(TextMessageRole::Assistant),
            name: None,
        }));
        self.push_plain(Payload::AgUi(AgUiEvent::TextMessageContent {
            message_id: message_id.to_string(),
            delta: text.to_string(),
        }));
        self.push_plain(Payload::AgUi(AgUiEvent::TextMessageEnd { message_id: message_id.to_string() }));
    }

    /// CV3: one sentence of the run's answer, journaled the moment
    /// the brain's stream completed it — inside the run, before
    /// `RUN_FINISHED`. This is the record voiced speaks from: it
    /// carries the run id, the sentence's index in the answer, and
    /// the sentence exactly as the brain produced it. The run's full
    /// answer still lands at the end as the classic text message
    /// (`answer`), so a reader who ignores chunks loses nothing, and
    /// the bracket rule is unchanged: chunks are only ever consumed
    /// inside their own run's `RUN_STARTED` … `RUN_FINISHED`.
    pub fn answer_chunk(&self, run_id: &str, index: usize, text: &str) -> u64 {
        self.push_plain(Payload::AgUi(AgUiEvent::Custom {
            name: "answer_chunk".to_string(),
            value: serde_json::json!({"run_id": run_id, "index": index, "text": text}),
        }))
    }

    /// CV3: how a brain call's content arrived — `streamed` (sentence
    /// chunks journaled live), `buffered` (the brain answered without
    /// a stream; the whole answer lands at once), `preamble` (content
    /// was streamed and *then* the round produced tool calls, so the
    /// chunks were a preamble, not the answer), or `aborted` (the
    /// stream broke after at least one chunk; the run fails rather
    /// than mix a second brain's answer into the first one's speech).
    /// Journaled once per brain call that produced content, so the
    /// journal — not the code path — says which shape a run took.
    pub fn answer_stream(&self, run_id: &str, mode: &str, chunks: usize, reason: &str) -> u64 {
        self.push_plain(Payload::AgUi(AgUiEvent::Custom {
            name: "answer_stream".to_string(),
            value: serde_json::json!({"run_id": run_id, "mode": mode, "chunks": chunks, "reason": reason}),
        }))
    }

    /// The broker refused a commit-class call for lack of confirmation.
    /// The tool did not run; this record is the whole of what happened.
    pub fn ask_to_commit(&self, tool: &str, label: &str) -> u64 {
        self.push_plain(Payload::AgUi(AgUiEvent::Custom {
            name: "ask_to_commit".to_string(),
            value: serde_json::json!({
                "tool": tool,
                "label": label,
                "reason": format!("`{label}` changes the phone and needs your confirmation first — repeat the request with \"confirm\": \"{label}\" to allow exactly this"),
            }),
        }))
    }

    /// CV1: the session context a run started with — the record that lets
    /// the kimono (and any later audit) see exactly what conversation the
    /// brain was given: how many earlier turns, which entities, what was
    /// pending, and whether this run opened the session or continued it.
    pub fn session_context(
        &self,
        session: &str,
        prior_turns: usize,
        entities: &std::collections::BTreeMap<String, String>,
        pending: &[String],
        follow_up: bool,
        fresh: bool,
    ) -> u64 {
        let ents: serde_json::Map<String, serde_json::Value> =
            entities.iter().map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone()))).collect();
        self.push_plain(Payload::AgUi(AgUiEvent::Custom {
            name: "session_context".to_string(),
            value: serde_json::json!({
                "session": session,
                "prior_turns": prior_turns,
                "entities": ents,
                "pending": pending,
                "follow_up": follow_up,
                "fresh": fresh,
            }),
        }))
    }

    /// CV1: the follow-up window changed state. The wakeword listener —
    /// the process that owns the microphone — reports its own window
    /// through `POST /follow-up`; this record is what the kimono's
    /// "still listening" indicator renders, so the page shows the mic's
    /// real state, never an inference from run timing.
    pub fn follow_up_window(&self, session: &str, state: &str) -> u64 {
        self.push_plain(Payload::AgUi(AgUiEvent::Custom {
            name: "follow_up_window".to_string(),
            value: serde_json::json!({ "session": session, "state": state }),
        }))
    }

    /// CV2: the repair stratum a turn was classified into, with the
    /// wire's confidence, the classification reason, and the span the
    /// stratum acts on. Every classified turn leaves this record —
    /// including HIGH and no-signal turns — so the journal, not the
    /// code path, is what a reader audits.
    pub fn repair_stratum(&self, stratum: &str, confidence: Option<f32>, reason: &str, span: Option<&str>) -> u64 {
        self.push_plain(Payload::AgUi(AgUiEvent::Custom {
            name: "repair_stratum".to_string(),
            value: serde_json::json!({
                "stratum": stratum,
                "confidence": confidence,
                "reason": reason,
                "span": span,
            }),
        }))
    }

    /// CV2: a repair answer was consumed. The composed request is
    /// recorded verbatim beside the original and the answer, so
    /// anyone auditing the run sees exactly which user words the
    /// brain was given — and that nothing agent-inferred was laundered
    /// into them.
    pub fn repair_resumed(&self, original: &str, heard_span: &str, answer: &str, composed: &str) -> u64 {
        self.push_plain(Payload::AgUi(AgUiEvent::Custom {
            name: "repair_resumed".to_string(),
            value: serde_json::json!({
                "original": original,
                "heard_span": heard_span,
                "answer": answer,
                "composed": composed,
            }),
        }))
    }

    /// An agent-emitted module manifest (MODULE-PROTOCOL v1). The
    /// manifest and its agent-filled inputs ride one CUSTOM record;
    /// kimono is the only renderer and it reads manifests only from
    /// the journal (the follow_up_window pattern) — this record is
    /// the whole emission path, there is no side channel.
    pub fn module_manifest(&self, manifest: serde_json::Value, inputs: serde_json::Value) -> u64 {
        self.push_plain(Payload::AgUi(AgUiEvent::Custom {
            name: "module_manifest".to_string(),
            value: serde_json::json!({ "manifest": manifest, "inputs": inputs }),
        }))
    }

    /// A brain switch, announced with the reason (VISION: hand-offs are visible).
    pub fn handoff(&self, destination: &str, reason: &str, status: HandoffStatus) -> u64 {
        self.push_plain(Payload::Op6(trace_events::Op6Event::Handoff {
            destination: destination.to_string(),
            reason: reason.to_string(),
            status,
        }))
    }
}

/// FNV-1a over the run id: stable, dependency-free, good enough to correlate.
fn hash128(s: &str) -> u128 {
    let mut h: u128 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= u128::from(b);
        h = h.wrapping_mul(0x1000_0000_01b3);
    }
    h
}
