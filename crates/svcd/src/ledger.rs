//! Durable execution ledger (SERVICE-IR.md §4.2, after hermes-agent
//! `cron/executions.py`, spec-only):
//!
//! - `executions`: one row per run, `claimed → running → completed | failed |
//!   unknown`; terminal rows are immutable.
//! - `effects`: `UNIQUE(service_id, idempotency_key)` — once an effect is
//!   recorded, any retry or redelivery skips it and replays the stored output.
//!
//! In-memory with JSON export/import; the SQLite-WAL `state:` backend is a gap.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ir::EffectClass;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecStatus {
    Claimed,
    Running,
    Completed,
    Failed,
    Unknown,
}

impl ExecStatus {
    pub fn is_terminal(self) -> bool {
        matches!(self, ExecStatus::Completed | ExecStatus::Failed | ExecStatus::Unknown)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExecutionRow {
    /// `<execution_id>/<n>`: the n-th run of one trigger occurrence.
    pub run_id: String,
    pub execution_id: String,
    pub service_id: String,
    pub trigger_id: String,
    pub status: ExecStatus,
    pub attempts: u32,
    pub started_ms: u64,
    pub finished_ms: Option<u64>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EffectRow {
    pub service_id: String,
    pub key: String,
    /// The readable key material that was hashed (open kimono).
    pub material: String,
    pub action_id: String,
    pub execution_id: String,
    pub effect_class: EffectClass,
    pub output: Value,
    pub at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LedgerError {
    IllegalTransition { run_id: String, from: ExecStatus, to: ExecStatus },
    NoSuchRun(String),
    DuplicateEffect { service_id: String, key: String },
}

impl std::fmt::Display for LedgerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LedgerError::IllegalTransition { run_id, from, to } => write!(f, "run {run_id}: {from:?} -> {to:?} is not allowed"),
            LedgerError::NoSuchRun(r) => write!(f, "no run {r}"),
            LedgerError::DuplicateEffect { service_id, key } => write!(f, "effect {key} already recorded for {service_id}"),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Ledger {
    executions: BTreeMap<String, ExecutionRow>,
    effects: BTreeMap<String, EffectRow>,
}

/// `blake3:<hex>` over `service_id \0 material` (SERVICE-IR.md §4.2).
pub fn idempotency_key(service_id: &str, material: &str) -> String {
    let mut h = blake3::Hasher::new();
    h.update(service_id.as_bytes());
    h.update(&[0]);
    h.update(material.as_bytes());
    format!("blake3:{}", h.finalize().to_hex())
}

/// Stable id of one trigger occurrence: retries and redeliveries share it.
pub fn execution_id(service_id: &str, trigger_id: &str, occurrence: &str) -> String {
    let h = blake3::hash(format!("{service_id}\0{trigger_id}\0{occurrence}").as_bytes());
    format!("x-{}", &h.to_hex()[..16])
}

fn effect_slot(service_id: &str, key: &str) -> String {
    format!("{service_id}\0{key}")
}

impl Ledger {
    pub fn new() -> Self {
        Self::default()
    }

    /// Open a new run row for an occurrence in `claimed`.
    pub fn claim(&mut self, execution_id: &str, service_id: &str, trigger_id: &str, now_ms: u64) -> String {
        let n = self.executions.values().filter(|r| r.execution_id == execution_id).count() + 1;
        let run_id = format!("{execution_id}/{n}");
        self.executions.insert(
            run_id.clone(),
            ExecutionRow {
                run_id: run_id.clone(),
                execution_id: execution_id.to_string(),
                service_id: service_id.to_string(),
                trigger_id: trigger_id.to_string(),
                status: ExecStatus::Claimed,
                attempts: 0,
                started_ms: now_ms,
                finished_ms: None,
                error: None,
            },
        );
        run_id
    }

    pub fn transition(&mut self, run_id: &str, to: ExecStatus, now_ms: u64, error: Option<String>) -> Result<(), LedgerError> {
        let row = self.executions.get_mut(run_id).ok_or_else(|| LedgerError::NoSuchRun(run_id.to_string()))?;
        let legal = matches!(
            (row.status, to),
            (ExecStatus::Claimed, ExecStatus::Running | ExecStatus::Failed)
                | (ExecStatus::Running, ExecStatus::Completed | ExecStatus::Failed | ExecStatus::Unknown)
        );
        if !legal {
            return Err(LedgerError::IllegalTransition { run_id: run_id.to_string(), from: row.status, to });
        }
        row.status = to;
        if to.is_terminal() {
            row.finished_ms = Some(now_ms);
            row.error = error;
        }
        Ok(())
    }

    pub fn bump_attempts(&mut self, run_id: &str) {
        if let Some(r) = self.executions.get_mut(run_id) {
            r.attempts += 1;
        }
    }

    /// True if any run of this occurrence completed (redelivery is a no-op).
    pub fn completed(&self, execution_id: &str) -> bool {
        self.executions.values().any(|r| r.execution_id == execution_id && r.status == ExecStatus::Completed)
    }

    pub fn run(&self, run_id: &str) -> Option<&ExecutionRow> {
        self.executions.get(run_id)
    }

    pub fn runs(&self) -> impl Iterator<Item = &ExecutionRow> {
        self.executions.values()
    }

    pub fn effect(&self, service_id: &str, key: &str) -> Option<&EffectRow> {
        self.effects.get(&effect_slot(service_id, key))
    }

    pub fn effects(&self) -> impl Iterator<Item = &EffectRow> {
        self.effects.values()
    }

    pub fn record_effect(&mut self, row: EffectRow) -> Result<(), LedgerError> {
        let slot = effect_slot(&row.service_id, &row.key);
        if self.effects.contains_key(&slot) {
            return Err(LedgerError::DuplicateEffect { service_id: row.service_id, key: row.key });
        }
        self.effects.insert(slot, row);
        Ok(())
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("ledger serializes")
    }

    pub fn from_json(text: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(text)
    }
}
