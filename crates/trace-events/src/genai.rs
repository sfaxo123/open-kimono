//! OpenTelemetry GenAI vocabulary for model calls (docs/PLAN.md B1).
//!
//! Names are copied, not invented: the span-name rule
//! `{gen_ai.operation.name} {target}` and the `gen_ai.*` attribute keys come
//! from the OTel GenAI semantic conventions as vendored in
//! `op6-refs/otel-semconv` @ 3e25804
//! (`model/gen-ai/deprecated/registry-deprecated.yaml` +
//! `spans-deprecated.yaml` — the conventions moved to the
//! `semantic-conventions-genai` repo with the names unchanged).
//!
//! A model call is two journal events — span start and span end — sharing a
//! `span_id` in the envelope, so the UI can draw the call's lifetime and the
//! end event carries the token usage.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use serde::{Deserialize, Serialize};

/// `gen_ai.*` attribute keys, exactly as the spec spells them.
pub mod keys {
    pub const OPERATION: &str = "gen_ai.operation.name";
    pub const PROVIDER: &str = "gen_ai.provider.name";
    pub const REQUEST_MODEL: &str = "gen_ai.request.model";
    pub const REQUEST_MAX_TOKENS: &str = "gen_ai.request.max_tokens";
    pub const REQUEST_TEMPERATURE: &str = "gen_ai.request.temperature";
    pub const REQUEST_STREAM: &str = "gen_ai.request.stream";
    pub const RESPONSE_MODEL: &str = "gen_ai.response.model";
    pub const RESPONSE_FINISH_REASONS: &str = "gen_ai.response.finish_reasons";
    pub const RESPONSE_TIME_TO_FIRST_CHUNK: &str = "gen_ai.response.time_to_first_chunk";
    pub const CONVERSATION_ID: &str = "gen_ai.conversation.id";
    pub const AGENT_ID: &str = "gen_ai.agent.id";
    pub const AGENT_NAME: &str = "gen_ai.agent.name";
    pub const AGENT_DESCRIPTION: &str = "gen_ai.agent.description";
    pub const TOOL_NAME: &str = "gen_ai.tool.name";
    pub const TOOL_DESCRIPTION: &str = "gen_ai.tool.description";
    pub const TOOL_CALL_ID: &str = "gen_ai.tool.call.id";
    pub const TOOL_CALL_ARGUMENTS: &str = "gen_ai.tool.call.arguments";
    pub const TOOL_CALL_RESULT: &str = "gen_ai.tool.call.result";
    pub const TOOL_DEFINITIONS: &str = "gen_ai.tool.definitions";
    pub const USAGE_INPUT_TOKENS: &str = "gen_ai.usage.input_tokens";
    pub const USAGE_OUTPUT_TOKENS: &str = "gen_ai.usage.output_tokens";
    pub const DATA_SOURCE_ID: &str = "gen_ai.data_source.id";
    pub const INPUT_MESSAGES: &str = "gen_ai.input.messages";
    pub const OUTPUT_MESSAGES: &str = "gen_ai.output.messages";
    pub const SYSTEM_INSTRUCTIONS: &str = "gen_ai.system_instructions";
    pub const OUTPUT_TYPE: &str = "gen_ai.output.type";
}

/// The `gen_ai.operation.name` enum (all nine spec values).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GenAiOperation {
    Chat,
    GenerateContent,
    TextCompletion,
    Embeddings,
    Retrieval,
    CreateAgent,
    InvokeAgent,
    ExecuteTool,
    InvokeWorkflow,
}

impl GenAiOperation {
    pub fn as_str(self) -> &'static str {
        match self {
            GenAiOperation::Chat => "chat",
            GenAiOperation::GenerateContent => "generate_content",
            GenAiOperation::TextCompletion => "text_completion",
            GenAiOperation::Embeddings => "embeddings",
            GenAiOperation::Retrieval => "retrieval",
            GenAiOperation::CreateAgent => "create_agent",
            GenAiOperation::InvokeAgent => "invoke_agent",
            GenAiOperation::ExecuteTool => "execute_tool",
            GenAiOperation::InvokeWorkflow => "invoke_workflow",
        }
    }

    /// Spec span name: `{gen_ai.operation.name} {target}`. The target is the
    /// request model (chat / text_completion / generate_content / embeddings),
    /// the tool name (execute_tool), the agent name (create_agent /
    /// invoke_agent / invoke_workflow) or the data-source id (retrieval).
    pub fn span_name(self, target: &str) -> String {
        alloc::format!("{} {}", self.as_str(), target)
    }

    pub fn parse(s: &str) -> Option<GenAiOperation> {
        match s {
            "chat" => Some(GenAiOperation::Chat),
            "generate_content" => Some(GenAiOperation::GenerateContent),
            "text_completion" => Some(GenAiOperation::TextCompletion),
            "embeddings" => Some(GenAiOperation::Embeddings),
            "retrieval" => Some(GenAiOperation::Retrieval),
            "create_agent" => Some(GenAiOperation::CreateAgent),
            "invoke_agent" => Some(GenAiOperation::InvokeAgent),
            "execute_tool" => Some(GenAiOperation::ExecuteTool),
            "invoke_workflow" => Some(GenAiOperation::InvokeWorkflow),
            _ => None,
        }
    }
}

/// Span attributes. `BTreeMap` so the JSON is deterministic (workbench
/// evidence and tests must be byte-stable).
pub type AttrMap = BTreeMap<String, AttrValue>;

/// OTel attribute value, untagged in JSON (string/number/bool/array of
/// strings — the types the GenAI conventions actually use).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AttrValue {
    Str(String),
    I64(i64),
    F64(f64),
    Bool(bool),
    Strs(Vec<String>),
}

impl From<&str> for AttrValue {
    fn from(v: &str) -> Self {
        AttrValue::Str(v.into())
    }
}
impl From<String> for AttrValue {
    fn from(v: String) -> Self {
        AttrValue::Str(v)
    }
}
impl From<i64> for AttrValue {
    fn from(v: i64) -> Self {
        AttrValue::I64(v)
    }
}
impl From<f64> for AttrValue {
    fn from(v: f64) -> Self {
        AttrValue::F64(v)
    }
}
impl From<bool> for AttrValue {
    fn from(v: bool) -> Self {
        AttrValue::Bool(v)
    }
}
impl From<Vec<String>> for AttrValue {
    fn from(v: Vec<String>) -> Self {
        AttrValue::Strs(v)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpanStatus {
    Ok,
    Error,
}

/// A model/tool/agent span as two journal events.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum GenAiEvent {
    SpanStart {
        name: String,
        #[serde(default)]
        attributes: AttrMap,
    },
    SpanEnd {
        name: String,
        status: SpanStatus,
        #[serde(default)]
        attributes: AttrMap,
    },
}

impl GenAiEvent {
    /// Span start with the spec-shaped name `{operation} {target}`.
    pub fn span_start(op: GenAiOperation, target: &str, attributes: AttrMap) -> GenAiEvent {
        GenAiEvent::SpanStart { name: op.span_name(target), attributes }
    }

    /// Span end; pair it with the start via the envelope's `span_id`.
    pub fn span_end(op: GenAiOperation, target: &str, status: SpanStatus, attributes: AttrMap) -> GenAiEvent {
        GenAiEvent::SpanEnd { name: op.span_name(target), status, attributes }
    }
}
