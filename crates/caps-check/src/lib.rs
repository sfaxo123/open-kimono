//! Pattern store (A2.1): learned patterns are versioned, synced to the
//! homelab (sync log is the transport record), and reversible.
//!
//! Invariants proven in tests:
//! - Every learn/forget bumps the version and appends to the sync log —
//!   no silent behavior change.
//! - Forget fully reverses a learn (pattern gone, version still advances
//!   so synced peers converge on the reversal).

use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq)]
pub struct Pattern {
    pub key: String,
    pub value: String,
    pub version: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SyncOp {
    Learn { key: String, version: u64 },
    Forget { key: String, version: u64 },
}

pub struct PatternStore {
    patterns: BTreeMap<String, Pattern>,
    version: u64,
    sync_log: Vec<SyncOp>,
}

impl PatternStore {
    pub fn new() -> Self {
        Self { patterns: BTreeMap::new(), version: 0, sync_log: Vec::new() }
    }

    pub fn learn(&mut self, key: &str, value: &str) -> u64 {
        self.version += 1;
        let v = self.version;
        self.patterns.insert(key.into(), Pattern { key: key.into(), value: value.into(), version: v });
        self.sync_log.push(SyncOp::Learn { key: key.into(), version: v });
        v
    }

    /// Forget is a synced, versioned reversal — never a silent delete.
    pub fn forget(&mut self, key: &str) -> Option<u64> {
        if self.patterns.remove(key).is_none() {
            return None;
        }
        self.version += 1;
        let v = self.version;
        self.sync_log.push(SyncOp::Forget { key: key.into(), version: v });
        Some(v)
    }

    pub fn get(&self, key: &str) -> Option<&Pattern> {
        self.patterns.get(key)
    }

    pub fn version(&self) -> u64 {
        self.version
    }

    /// What would be pushed to the homelab on next sync.
    pub fn sync_log(&self) -> &[SyncOp] {
        &self.sync_log
    }

    /// Rebuild from a synced log (survives phone reset/replacement).
    pub fn replay(ops: &[SyncOp], values: &BTreeMap<String, String>) -> Self {
        let mut s = Self::new();
        for op in ops {
            match op {
                SyncOp::Learn { key, version } => {
                    s.version = *version;
                    if let Some(v) = values.get(key) {
                        s.patterns.insert(key.clone(), Pattern { key: key.clone(), value: v.clone(), version: *version });
                    }
                    s.sync_log.push(op.clone());
                }
                SyncOp::Forget { key, version } => {
                    s.version = *version;
                    s.patterns.remove(key);
                    s.sync_log.push(op.clone());
                }
            }
        }
        s
    }
}

impl Default for PatternStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn learn_is_versioned_and_logged() {
        let mut s = PatternStore::new();
        let v = s.learn("sat_destination", "Tabby's house");
        assert_eq!(v, 1);
        assert_eq!(s.get("sat_destination").unwrap().value, "Tabby's house");
        assert_eq!(s.sync_log().len(), 1);
    }

    #[test]
    fn forget_reverses_and_is_synced() {
        let mut s = PatternStore::new();
        s.learn("sat_destination", "Tabby's house");
        assert!(s.forget("sat_destination").is_some());
        assert!(s.get("sat_destination").is_none());
        assert_eq!(s.sync_log().len(), 2);
        assert!(s.forget("never_learned").is_none());
    }

    #[test]
    fn replay_survives_reset() {
        let mut s = PatternStore::new();
        s.learn("car_action", "ask_tabby");
        let ops = s.sync_log().to_vec();
        let mut vals = BTreeMap::new();
        vals.insert("car_action".to_string(), "ask_tabby".to_string());
        let r = PatternStore::replay(&ops, &vals);
        assert_eq!(r.get("car_action").unwrap().value, "ask_tabby");
        assert_eq!(r.version(), s.version());
    }
}
