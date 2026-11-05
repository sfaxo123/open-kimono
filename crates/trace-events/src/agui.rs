//! AG-UI (Agent User Interaction Protocol) event vocabulary — the agent↔UI
//! half of the journal (docs/PLAN.md B1).
//!
//! The vocabulary is copied from the AG-UI 1.0 spec vendored at
//! `op6-refs/ag-ui` @ 73b04fd (`spec/1.0/schema.json`, MIT): every event
//! carries `"type"` with a SCREAMING_SNAKE discriminator and camelCase
//! fields, so an AG-UI consumer can read our journal lines unmodified.
//! Required-vs-optional fields follow the spec's `required` lists.

use alloc::string::String;
use alloc::vec::Vec;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The discriminator vocabulary — all 31 `EventType` values from
/// `spec/1.0/schema.json`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EventType {
    TextMessageStart,
    TextMessageContent,
    TextMessageEnd,
    TextMessageChunk,
    ToolCallStart,
    ToolCallArgs,
    ToolCallEnd,
    ToolCallChunk,
    ToolCallResult,
    StateSnapshot,
    StateDelta,
    MessagesSnapshot,
    ActivitySnapshot,
    ActivityDelta,
    Raw,
    Custom,
    RunStarted,
    RunFinished,
    RunError,
    StepStarted,
    StepFinished,
    ReasoningStart,
    ReasoningMessageStart,
    ReasoningMessageContent,
    ReasoningMessageEnd,
    ReasoningMessageChunk,
    ReasoningEnd,
    ReasoningEncryptedValue,
    SubagentStarted,
    SubagentFinished,
    SubagentError,
}

/// `TextMessageRole` from the spec.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TextMessageRole {
    Developer,
    System,
    Assistant,
    User,
}

/// `ReasoningEncryptedValueSubtype` from the spec (note the hyphen).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EncryptedValueSubtype {
    ToolCall,
    Message,
}

/// `RunFinishedOutcome` (oneOf success / interrupt / cancelled).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum RunFinishedOutcome {
    Success {
        #[serde(default, rename = "pendingToolCallIds")]
        pending_tool_call_ids: Vec<String>,
    },
    Interrupt {
        interrupts: Vec<Interrupt>,
    },
    Cancelled,
}

/// `Interrupt` — only `id` and `reason` are required by the spec.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Interrupt {
    pub id: String,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_schema: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

/// `SubagentFinishedOutcome` (success / suspended).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum SubagentFinishedOutcome {
    Success,
    Suspended {
        #[serde(default, rename = "interruptIds")]
        interrupt_ids: Vec<String>,
    },
}

/// `TokenUsage` — every field optional in the spec.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_input_tokens: Option<u64>,
}

/// One AG-UI protocol event, exactly as the spec serializes it: internally
/// tagged by `"type"`, camelCase fields. Arbitrary JSON (state snapshots,
/// JSON-Patch deltas, tool results, raw events) passes through as
/// [`serde_json::Value`] so nothing protocol-shaped is invented here.
///
/// Note: an explicit JSON `null` in an optional field reads back as `None`
/// (serde's `Option` semantics) — absent and null are equivalent here.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AgUiEvent {
    // — text messages —
    TextMessageStart {
        #[serde(rename = "messageId")]
        message_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        role: Option<TextMessageRole>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
    TextMessageContent {
        #[serde(rename = "messageId")]
        message_id: String,
        delta: String,
    },
    TextMessageEnd {
        #[serde(rename = "messageId")]
        message_id: String,
    },
    TextMessageChunk {
        #[serde(rename = "messageId")]
        message_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        role: Option<TextMessageRole>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        delta: Option<String>,
    },

    // — tool calls —
    ToolCallStart {
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        #[serde(rename = "toolCallName")]
        tool_call_name: String,
        #[serde(default, rename = "parentMessageId", skip_serializing_if = "Option::is_none")]
        parent_message_id: Option<String>,
    },
    ToolCallArgs {
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        delta: String,
    },
    ToolCallEnd {
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
    },
    ToolCallChunk {
        #[serde(default, rename = "toolCallId", skip_serializing_if = "Option::is_none")]
        tool_call_id: Option<String>,
        #[serde(default, rename = "toolCallName", skip_serializing_if = "Option::is_none")]
        tool_call_name: Option<String>,
        #[serde(default, rename = "parentMessageId", skip_serializing_if = "Option::is_none")]
        parent_message_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        delta: Option<String>,
    },
    ToolCallResult {
        #[serde(rename = "messageId")]
        message_id: String,
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        content: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        role: Option<String>,
    },

    // — shared state —
    StateSnapshot {
        snapshot: Value,
    },
    StateDelta {
        /// JSON Patch (RFC 6902) array.
        delta: Value,
    },
    MessagesSnapshot {
        messages: Value,
    },
    ActivitySnapshot {
        #[serde(rename = "messageId")]
        message_id: String,
        #[serde(rename = "activityType")]
        activity_type: String,
        content: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        replace: Option<bool>,
    },
    ActivityDelta {
        #[serde(rename = "messageId")]
        message_id: String,
        #[serde(rename = "activityType")]
        activity_type: String,
        patch: Value,
    },

    // — escape hatches —
    Raw {
        event: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source: Option<String>,
    },
    Custom {
        name: String,
        value: Value,
    },

    // — runs and steps —
    RunStarted {
        #[serde(rename = "threadId")]
        thread_id: String,
        #[serde(rename = "runId")]
        run_id: String,
        #[serde(default, rename = "protocolVersion", skip_serializing_if = "Option::is_none")]
        protocol_version: Option<String>,
        #[serde(default, rename = "parentRunId", skip_serializing_if = "Option::is_none")]
        parent_run_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        input: Option<Value>,
    },
    RunFinished {
        #[serde(rename = "threadId")]
        thread_id: String,
        #[serde(rename = "runId")]
        run_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        result: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        outcome: Option<RunFinishedOutcome>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        usage: Vec<TokenUsage>,
    },
    RunError {
        message: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        code: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        usage: Vec<TokenUsage>,
    },
    StepStarted {
        #[serde(rename = "stepName")]
        step_name: String,
    },
    StepFinished {
        #[serde(rename = "stepName")]
        step_name: String,
    },

    // — reasoning (the open kimono's favourite events) —
    ReasoningStart {
        #[serde(rename = "messageId")]
        message_id: String,
    },
    ReasoningMessageStart {
        #[serde(rename = "messageId")]
        message_id: String,
        role: String,
    },
    ReasoningMessageContent {
        #[serde(rename = "messageId")]
        message_id: String,
        delta: String,
    },
    ReasoningMessageEnd {
        #[serde(rename = "messageId")]
        message_id: String,
    },
    ReasoningMessageChunk {
        #[serde(rename = "messageId")]
        message_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        delta: Option<String>,
    },
    ReasoningEnd {
        #[serde(rename = "messageId")]
        message_id: String,
    },
    ReasoningEncryptedValue {
        subtype: EncryptedValueSubtype,
        #[serde(rename = "entityId")]
        entity_id: String,
        #[serde(rename = "encryptedValue")]
        encrypted_value: String,
    },

    // — subagents —
    SubagentStarted {
        #[serde(rename = "subagentRunId")]
        subagent_run_id: String,
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        #[serde(default, rename = "parentSubagentRunId", skip_serializing_if = "Option::is_none")]
        parent_subagent_run_id: Option<String>,
        #[serde(default, rename = "parentToolCallId", skip_serializing_if = "Option::is_none")]
        parent_tool_call_id: Option<String>,
        #[serde(default, rename = "parentMessageId", skip_serializing_if = "Option::is_none")]
        parent_message_id: Option<String>,
    },
    SubagentFinished {
        #[serde(rename = "subagentRunId")]
        subagent_run_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        result: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        outcome: Option<SubagentFinishedOutcome>,
    },
    SubagentError {
        #[serde(rename = "subagentRunId")]
        subagent_run_id: String,
        message: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        code: Option<String>,
    },
}

impl AgUiEvent {
    /// The spec `EventType` discriminator for this event.
    pub fn event_type(&self) -> EventType {
        match self {
            AgUiEvent::TextMessageStart { .. } => EventType::TextMessageStart,
            AgUiEvent::TextMessageContent { .. } => EventType::TextMessageContent,
            AgUiEvent::TextMessageEnd { .. } => EventType::TextMessageEnd,
            AgUiEvent::TextMessageChunk { .. } => EventType::TextMessageChunk,
            AgUiEvent::ToolCallStart { .. } => EventType::ToolCallStart,
            AgUiEvent::ToolCallArgs { .. } => EventType::ToolCallArgs,
            AgUiEvent::ToolCallEnd { .. } => EventType::ToolCallEnd,
            AgUiEvent::ToolCallChunk { .. } => EventType::ToolCallChunk,
            AgUiEvent::ToolCallResult { .. } => EventType::ToolCallResult,
            AgUiEvent::StateSnapshot { .. } => EventType::StateSnapshot,
            AgUiEvent::StateDelta { .. } => EventType::StateDelta,
            AgUiEvent::MessagesSnapshot { .. } => EventType::MessagesSnapshot,
            AgUiEvent::ActivitySnapshot { .. } => EventType::ActivitySnapshot,
            AgUiEvent::ActivityDelta { .. } => EventType::ActivityDelta,
            AgUiEvent::Raw { .. } => EventType::Raw,
            AgUiEvent::Custom { .. } => EventType::Custom,
            AgUiEvent::RunStarted { .. } => EventType::RunStarted,
            AgUiEvent::RunFinished { .. } => EventType::RunFinished,
            AgUiEvent::RunError { .. } => EventType::RunError,
            AgUiEvent::StepStarted { .. } => EventType::StepStarted,
            AgUiEvent::StepFinished { .. } => EventType::StepFinished,
            AgUiEvent::ReasoningStart { .. } => EventType::ReasoningStart,
            AgUiEvent::ReasoningMessageStart { .. } => EventType::ReasoningMessageStart,
            AgUiEvent::ReasoningMessageContent { .. } => EventType::ReasoningMessageContent,
            AgUiEvent::ReasoningMessageEnd { .. } => EventType::ReasoningMessageEnd,
            AgUiEvent::ReasoningMessageChunk { .. } => EventType::ReasoningMessageChunk,
            AgUiEvent::ReasoningEnd { .. } => EventType::ReasoningEnd,
            AgUiEvent::ReasoningEncryptedValue { .. } => EventType::ReasoningEncryptedValue,
            AgUiEvent::SubagentStarted { .. } => EventType::SubagentStarted,
            AgUiEvent::SubagentFinished { .. } => EventType::SubagentFinished,
            AgUiEvent::SubagentError { .. } => EventType::SubagentError,
        }
    }
}
