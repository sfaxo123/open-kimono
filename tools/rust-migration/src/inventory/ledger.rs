//! Ledger document assembled from the repo inventory's own enumeration.
//!
//! The ledger exists so later inventory and host-tool packets (U-Boot,
//! QEMU, custom-feature and dependency audits) can key off one
//! deterministic file. That only works if every consumer of the JSON can
//! re-derive the same bytes: hence byte-order-sorted rows and one
//! canonical serialization (compact JSON, `\n` after every row — the
//! repo's existing ledger format, `tools/rust-migration/inventory.json`).

use serde::Serialize;

use super::scan::InventoryRow;

#[derive(Serialize)]
pub struct Ledger {
    /// `git rev-parse HEAD` in the repository root the scan walked.
    pub baseline_commit: String,
    pub generated_by: &'static str,
    pub schema: &'static str,
    pub roots: Vec<String>,
    pub worktree_mode: &'static str,
    /// Paths the scan enumerated (one per `git ls-files` entry).
    pub tracked_count: usize,
    /// Rows emitted; one per tracked path unless a read failed.
    pub row_count: usize,
    /// Paths that matched no classification rule, or could not be read.
    pub unclassified: Vec<String>,
    pub unclassified_count: usize,
    pub categories: Vec<Category>,
    pub files: Vec<InventoryRow>,
}

/// Category tallies sorted by name; the binary prints them on a clean
/// run.
#[derive(Serialize)]
pub struct Category {
    pub category: String,
    pub count: usize,
    pub total_bytes: u64,
}

/// Canonical serialization of a ledger: serde_json compact bytes plus a
/// trailing newline. Deterministic by construction — the struct field
/// order fixes key order and the rows arrive pre-sorted by path — so two
/// runs over identical bytes emit identical files.
pub fn to_json(ledger: &Ledger) -> String {
    let mut json = serde_json::to_string(ledger).expect("serialize ledger");
    json.push('\n');
    json
}

/// Write the canonical JSON to `path`, creating the parent directory if
/// needed.
pub fn write_json(path: &std::path::Path, json: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, json)
}
