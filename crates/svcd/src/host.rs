//! The two seams to the outside world. svcd never touches hardware itself:
//!
//! - [`ActionHost`] performs effects — speak a dialog, run a Wasm `logic`
//!   action (the `crates/sandbox` adapter plugs in here), undo a compensated
//!   action. The simulator uses [`SimHost`], a recorder with scripted replies.
//! - [`CapsBroker`] decides whether a capability request is granted (the
//!   `caps:` scheme later; [`GrantAll`] / [`DenyCaps`] for tests).

use std::collections::{BTreeMap, VecDeque};

use serde::Serialize;
use serde_json::Value;

use crate::ir::{CapabilityRequest, DialogPriority};

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ActionKind {
    /// Speak (Wyoming TTS) and show text. Always `committed_irreversible`.
    Dialog { speech: String, text: String, priority: DialogPriority },
    /// Invoke a function on a sandboxed Wasm component.
    Logic { component_ref: String, wit_interface: String, function: String, args: Value, fuel_budget: u64 },
    /// Roll back a `reversible_compensated` action's external effect.
    Compensate { action_id: String, compensating_action: String },
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ActionRequest {
    pub service_id: String,
    pub action_id: String,
    pub execution_id: String,
    /// `blake3:<hex>` key the ledger records once the effect succeeds.
    pub idempotency_key: Option<String>,
    pub now_ms: u64,
    pub kind: ActionKind,
}

/// What an effect cost; svcd enforces the service's power budget from it.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct ActionOutcome {
    pub output: Value,
    pub fuel_used: u64,
    pub network_bytes: u64,
    /// Virtual time the effect took (checked against `timeout.total_ms`).
    pub elapsed_ms: u64,
    pub wakelock_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostError(pub String);

pub trait ActionHost {
    fn invoke(&mut self, req: &ActionRequest) -> Result<ActionOutcome, HostError>;
}

pub trait CapsBroker {
    /// `Ok(())` grants; `Err(reason)` refuses (the reason is journaled).
    fn grant(&mut self, service_id: &str, req: &CapabilityRequest, now_ms: u64) -> Result<(), String>;
}

/// Grants every request (simulator default: the IR's tests run with the
/// capabilities the IR asks for).
#[derive(Clone, Copy, Debug, Default)]
pub struct GrantAll;

impl CapsBroker for GrantAll {
    fn grant(&mut self, _: &str, _: &CapabilityRequest, _: u64) -> Result<(), String> {
        Ok(())
    }
}

/// Refuses the listed capability names, grants the rest.
#[derive(Clone, Debug, Default)]
pub struct DenyCaps(pub Vec<String>);

impl CapsBroker for DenyCaps {
    fn grant(&mut self, _: &str, req: &CapabilityRequest, _: u64) -> Result<(), String> {
        if self.0.contains(&req.capability) {
            Err(format!("user denied `{}`", req.capability))
        } else {
            Ok(())
        }
    }
}

/// Records every request; replies from a per-action script, else success.
#[derive(Clone, Debug, Default)]
pub struct SimHost {
    /// Successful effects, in order.
    pub calls: Vec<ActionRequest>,
    /// Every invocation, including ones the script failed.
    pub attempts: Vec<ActionRequest>,
    script: BTreeMap<String, VecDeque<Result<ActionOutcome, HostError>>>,
}

impl SimHost {
    pub fn new() -> Self {
        Self::default()
    }

    /// Queue the next reply for `action_id` (consumed in order).
    pub fn script(mut self, action_id: &str, reply: Result<ActionOutcome, HostError>) -> Self {
        self.script.entry(action_id.to_string()).or_default().push_back(reply);
        self
    }

    /// Effects actually performed (successful calls) for an action.
    pub fn performed(&self, action_id: &str) -> usize {
        self.calls.iter().filter(|c| c.action_id == action_id && !matches!(c.kind, ActionKind::Compensate { .. })).count()
    }

    pub fn spoken(&self) -> Vec<String> {
        self.calls
            .iter()
            .filter_map(|c| match &c.kind {
                ActionKind::Dialog { speech, .. } => Some(speech.clone()),
                _ => None,
            })
            .collect()
    }
}

impl ActionHost for SimHost {
    fn invoke(&mut self, req: &ActionRequest) -> Result<ActionOutcome, HostError> {
        let reply = self.script.get_mut(&req.action_id).and_then(VecDeque::pop_front);
        let reply = reply.unwrap_or_else(|| Ok(ActionOutcome::default()));
        self.attempts.push(req.clone());
        if reply.is_ok() {
            self.calls.push(req.clone());
        }
        reply
    }
}
