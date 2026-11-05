//! Lego generator (A3.1): deterministic reuse before generation.
//!
//! A kept generated block joins the deterministic library; the next
//! identical request reuses it and never regenerates. Every build is
//! recorded in the console log. Generation runs sandboxed: the block
//! is inert until promoted by the user keeping it.

use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Reused(String),
    Generated { block_id: String, sandboxed: bool },
}

pub struct Generator {
    library: HashMap<String, String>, // intent -> block id
    log: Vec<String>,
    seq: u64,
}

impl Generator {
    pub fn new() -> Self {
        Self { library: HashMap::new(), log: Vec::new(), seq: 0 }
    }

    pub fn fulfill(&mut self, intent: &str) -> Outcome {
        if let Some(id) = self.library.get(intent) {
            let id = id.clone();
            self.log.push(format!("reuse {id} for {intent}"));
            return Outcome::Reused(id);
        }
        self.seq += 1;
        let id = format!("gen-{}", self.seq);
        self.log.push(format!("generated {id} for {intent} (sandboxed)"));
        Outcome::Generated { block_id: id, sandboxed: true }
    }

    /// User keeps a generated block -> deterministic from now on.
    pub fn keep(&mut self, intent: &str, block_id: &str) {
        self.library.insert(intent.to_string(), block_id.to_string());
        self.log.push(format!("promoted {block_id} for {intent}"));
    }

    pub fn discard(&mut self, block_id: &str) {
        self.log.push(format!("discarded {block_id}"));
    }

    pub fn console_log(&self) -> &[String] {
        &self.log
    }
}

impl Default for Generator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_then_kept_then_reused() {
        let mut g = Generator::new();
        let first = g.fulfill("pool laps");
        let id = match first { Outcome::Generated { block_id, sandboxed } => { assert!(sandboxed); block_id } _ => panic!("expected generation") };
        g.keep("pool laps", &id);
        assert_eq!(g.fulfill("pool laps"), Outcome::Reused(id));
    }

    #[test]
    fn discarded_block_regenerates() {
        let mut g = Generator::new();
        let _ = g.fulfill("pool laps");
        g.discard("gen-1");
        match g.fulfill("pool laps") {
            Outcome::Generated { block_id, .. } => assert_eq!(block_id, "gen-2"),
            _ => panic!("expected regeneration after discard"),
        }
    }

    #[test]
    fn builds_are_console_logged() {
        let mut g = Generator::new();
        let _ = g.fulfill("x");
        assert!(g.console_log().iter().any(|l| l.contains("sandboxed")));
    }
}
