//! The event→view model: journal events in, one screen's worth of state out.
//!
//! Pure std — no windowing — so `cargo test` proves the whole open-kimono
//! mapping (VISION §3) without SDL. Every field here has exactly one place on
//! the screen; events with no place on the screen (grants, text messages,
//! state snapshots) are deliberately *not* modeled: minimal by design
//! (VISION §8), the journal still carries them.

use trace_events::{HandoffStatus, Payload};

/// Tool rows kept on screen (oldest finished rows are pruned first).
pub const MAX_TOOLS: usize = 6;
/// Service rows kept on screen.
pub const MAX_SERVICES: usize = 8;
/// Power rows kept on screen.
pub const MAX_POWER: usize = 8;
/// Longest tool-args / result preview shown (screen is 60 cols at 480 px).
pub const PREVIEW_CHARS: usize = 48;

/// A tool call as the screen shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToolState {
    /// Started, args still streaming.
    Running,
    /// Args complete (`TOOL_CALL_END`), waiting for the result.
    Ended,
    /// Result arrived (`TOOL_CALL_RESULT`).
    Result,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// Accumulated argument deltas (JSON-Patch-ish stream; shown truncated).
    pub args: String,
    pub state: ToolState,
    /// Result preview, once it arrives.
    pub result: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServiceEntry {
    pub name: String,
    pub generation: u64,
    pub running: bool,
    /// Why it started/stopped — the reason is mandatory in the schema.
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PowerEntry {
    pub component: String,
    pub on: bool,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HandoffEntry {
    pub destination: String,
    pub status: HandoffStatus,
    pub reason: String,
}

/// Everything the one screen shows. `Default` is the idle screen.
#[derive(Clone, Debug, Default)]
pub struct TimelineModel {
    // — mic —
    pub mic_live: bool,
    /// 0.0..=1.0, clamped on ingest; drives the level bar.
    pub mic_level: f32,
    /// Reason of the last `mic_stopped` — closing is never silent.
    pub mic_reason: Option<String>,

    // — STT —
    /// Live transcript (partial or final).
    pub transcript: String,
    pub transcript_final: bool,
    /// Post-cleaned text of the last utterance.
    pub cleaned: Option<String>,

    // — model activity —
    /// Active GenAI span name (`chat qwen3.8`), shown in the header.
    pub model_span: Option<String>,
    /// Streaming reasoning text (current block).
    pub reasoning: String,
    /// AG-UI messageId of the current reasoning block.
    pub reasoning_id: Option<String>,
    pub reasoning_done: bool,

    // — tools —
    pub tools: Vec<ToolCall>,

    // — services & power —
    pub services: Vec<ServiceEntry>,
    pub power: Vec<PowerEntry>,
    /// Homelab/cloud hand-off, visible until replaced (VISION §5).
    pub handoff: Option<HandoffEntry>,

    // — honesty —
    /// Last `RUN_ERROR` message; cleared by the next successful run start.
    pub error: Option<String>,
    /// Records the journal skipped, cumulative (`JournalDropped`). A gap in the
    /// history is shown, never hidden — the B1 backpressure contract.
    pub dropped: u64,
    /// Reason of the most recent drop notice.
    pub dropped_reason: Option<String>,

    // — journal position —
    pub events_applied: u64,
    pub last_seq: Option<u64>,
}

/// Truncate to `PREVIEW_CHARS` chars with an ASCII ellipsis (one line on screen).
fn preview(s: &str) -> String {
    let s = s.replace('\n', " ").replace('\r', "");
    if s.chars().count() <= PREVIEW_CHARS {
        s
    } else {
        let mut out: String = s.chars().take(PREVIEW_CHARS).collect();
        out.push_str("...");
        out
    }
}

impl TimelineModel {
    /// Apply one journal event. Unknown/unmodeled payloads are ignored on
    /// purpose (the journal keeps them; the screen does not show everything).
    pub fn apply(&mut self, event: &trace_events::Event) {
        self.events_applied += 1;
        self.last_seq = Some(event.seq);
        match &event.payload {
            Payload::Op6(e) => self.apply_op6(e),
            Payload::AgUi(e) => self.apply_agui(e),
            Payload::GenAi(e) => self.apply_genai(e),
        }
    }

    fn apply_op6(&mut self, e: &trace_events::Op6Event) {
        use trace_events::Op6Event::*;
        match e {
            MicStarted { .. } => {
                self.mic_live = true;
                self.mic_reason = None;
            }
            MicStopped { reason } => {
                self.mic_live = false;
                self.mic_level = 0.0;
                self.mic_reason = Some(reason.clone());
            }
            MicFrame { level } => {
                self.mic_level = level.clamp(0.0, 1.0);
            }
            SttPartial { text, .. } => {
                self.transcript = text.clone();
                self.transcript_final = false;
            }
            SttFinal { text, .. } => {
                self.transcript = text.clone();
                self.transcript_final = true;
            }
            TextCleaned { cleaned, .. } => {
                self.cleaned = Some(cleaned.clone());
            }
            PowerChange { component, state, reason } => {
                let on = matches!(state, trace_events::PowerState::On);
                self.power.retain(|p| p.component != *component);
                self.power.push(PowerEntry { component: component.clone(), on, reason: reason.clone() });
                trim(&mut self.power, MAX_POWER);
            }
            WakeLockAcquired { holder, deadline_us, .. } => {
                let reason = match deadline_us {
                    Some(d) => format!("wake lock, deadline {d}"),
                    None => "wake lock".to_string(),
                };
                self.power.retain(|p| p.component != *holder);
                self.power.push(PowerEntry { component: holder.clone(), on: true, reason });
                trim(&mut self.power, MAX_POWER);
            }
            WakeLockReleased { holder, .. } => {
                self.power.retain(|p| p.component != *holder);
            }
            ServiceStarted { service, generation, reason } => {
                self.services.retain(|s| !(s.name == *service && s.generation == *generation));
                self.services.push(ServiceEntry {
                    name: service.clone(),
                    generation: *generation,
                    running: true,
                    reason: reason.clone(),
                });
                trim(&mut self.services, MAX_SERVICES);
            }
            ServiceStopped { service, generation, reason } => {
                if let Some(s) = self
                    .services
                    .iter_mut()
                    .find(|s| s.name == *service && s.generation == *generation)
                {
                    s.running = false;
                    s.reason = reason.clone();
                } else {
                    self.services.push(ServiceEntry {
                        name: service.clone(),
                        generation: *generation,
                        running: false,
                        reason: reason.clone(),
                    });
                }
                trim(&mut self.services, MAX_SERVICES);
            }
            Handoff { destination, reason, status } => {
                self.handoff =
                    Some(HandoffEntry { destination: destination.clone(), status: *status, reason: reason.clone() });
            }
            // Grants are journal-only for this screen (minimal by design).
            GrantGranted { .. } | GrantRevoked { .. } => {}
            // A skipped record is an honesty event: the screen says the history
            // has a gap rather than pretending it is complete (B1 contract).
            JournalDropped { count, reason } => {
                self.dropped += count;
                self.dropped_reason = Some(reason.clone());
            }
        }
    }

    fn apply_agui(&mut self, e: &trace_events::AgUiEvent) {
        use trace_events::AgUiEvent::*;
        match e {
            ReasoningStart { message_id } | ReasoningMessageStart { message_id, .. } => {
                if self.reasoning_id.as_deref() != Some(message_id.as_str()) {
                    self.reasoning.clear();
                    self.reasoning_id = Some(message_id.clone());
                }
                self.reasoning_done = false;
            }
            ReasoningMessageContent { message_id, delta } => {
                self.reasoning_block(message_id);
                self.reasoning.push_str(delta);
            }
            ReasoningMessageChunk { message_id, delta } => {
                self.reasoning_block(message_id);
                if let Some(delta) = delta {
                    self.reasoning.push_str(delta);
                }
            }
            ReasoningMessageEnd { .. } | ReasoningEnd { .. } => {
                self.reasoning_done = true;
            }
            ToolCallStart { tool_call_id, tool_call_name, .. } => {
                self.tools.retain(|t| t.id != *tool_call_id);
                self.tools.push(ToolCall {
                    id: tool_call_id.clone(),
                    name: tool_call_name.clone(),
                    args: String::new(),
                    state: ToolState::Running,
                    result: None,
                });
                trim_tools(&mut self.tools);
            }
            ToolCallArgs { tool_call_id, delta } => {
                if let Some(t) = self.tools.iter_mut().find(|t| t.id == *tool_call_id) {
                    t.args.push_str(delta);
                }
            }
            ToolCallChunk { tool_call_id, tool_call_name, delta, .. } => {
                if let (Some(id), Some(name)) = (tool_call_id, tool_call_name) {
                    if !self.tools.iter().any(|t| t.id == *id) {
                        self.tools.push(ToolCall {
                            id: id.clone(),
                            name: name.clone(),
                            args: String::new(),
                            state: ToolState::Running,
                            result: None,
                        });
                        trim_tools(&mut self.tools);
                    }
                }
                if let (Some(id), Some(delta)) = (tool_call_id, delta) {
                    if let Some(t) = self.tools.iter_mut().find(|t| t.id == *id) {
                        t.args.push_str(delta);
                    }
                }
            }
            ToolCallEnd { tool_call_id } => {
                if let Some(t) = self.tools.iter_mut().find(|t| t.id == *tool_call_id) {
                    t.state = ToolState::Ended;
                }
            }
            ToolCallResult { tool_call_id, content, .. } => {
                if let Some(t) = self.tools.iter_mut().find(|t| t.id == *tool_call_id) {
                    t.state = ToolState::Result;
                    t.result = Some(preview(&content.to_string()));
                }
            }
            RunStarted { .. } => self.error = None,
            RunError { message, .. } => self.error = Some(message.clone()),
            // Answer text, state snapshots, subagents: journal-only here.
            _ => {}
        }
    }

    fn apply_genai(&mut self, e: &trace_events::GenAiEvent) {
        match e {
            trace_events::GenAiEvent::SpanStart { name, .. } => self.model_span = Some(name.clone()),
            trace_events::GenAiEvent::SpanEnd { .. } => self.model_span = None,
        }
    }

    /// Start a fresh reasoning block if `message_id` is not the current one.
    fn reasoning_block(&mut self, message_id: &str) {
        if self.reasoning_id.as_deref() != Some(message_id) {
            self.reasoning.clear();
            self.reasoning_id = Some(message_id.to_string());
            self.reasoning_done = false;
        }
    }
}

/// Keep the last `max` rows (the screen shows the newest activity).
fn trim<T>(v: &mut Vec<T>, max: usize) {
    if v.len() > max {
        v.drain(0..v.len() - max);
    }
}

/// Tool rows: finished rows are pruned before running ones.
fn trim_tools(v: &mut Vec<ToolCall>) {
    while v.len() > MAX_TOOLS {
        match v.iter().position(|t| t.state == ToolState::Result) {
            Some(i) => {
                v.remove(i);
            }
            None => {
                v.remove(0);
            }
        }
    }
}
