//! Ledger document for the U-Boot source surfaces at the frozen pins.
//!
//! Deterministic by construction: rows are sorted by (ref, path) in a
//! byte-wise ordering matching `git ls-tree`, keys follow struct field
//! order, and the serialization is compact JSON plus one trailing
//! newline — two runs over the same pinned refs emit identical bytes.

use serde::Serialize;

use super::classify;
use super::git::{self, OverlayFile};
use super::tier;

/// One ledger row per file in a pinned tree. `id` is `ref + path`, unique
/// across both sections (the two pins differ as strings).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LedgerRow {
    pub id: String,
    /// Path within the tree at this ref (upstream_path == path; kept as
    /// a named field so downstream diff consumers can key on it).
    pub upstream_path: String,
    pub category: String,
    pub tier: &'static str,
    pub status: &'static str,
}

/// Category tallies sorted by name.
#[derive(Serialize)]
pub struct Tally {
    pub category: String,
    pub count: usize,
}

/// Tier tallies sorted by name.
#[derive(Serialize)]
pub struct TierTally {
    pub tier: &'static str,
    pub count: usize,
}

/// The downstream overlay section: the files `git diff` reports between
/// the pins, with per-file line stats, plus totals.
#[derive(Serialize)]
pub struct OverlaySection {
    pub upstream_ref: &'static str,
    pub downstream_ref: &'static str,
    pub file_count: usize,
    pub total_added: u64,
    pub total_removed: u64,
    pub files: Vec<OverlayFile>,
}

/// The full ledger document.
#[derive(Serialize)]
pub struct Ledger {
    pub schema: &'static str,
    pub generated_by: &'static str,
    pub upstream_url: &'static str,
    pub upstream_ref: &'static str,
    pub downstream_ref: &'static str,
    pub fork_path: String,
    /// Rows for the full upstream tree at the upstream pin.
    pub upstream_row_count: usize,
    /// Rows for the full downstream tree at the downstream pin.
    pub downstream_row_count: usize,
    pub row_count: usize,
    /// Paths that matched no category rule (empty on a clean run).
    pub unmapped: Vec<String>,
    pub unmapped_count: usize,
    pub categories: Vec<Tally>,
    pub tiers: Vec<TierTally>,
    pub overlay: OverlaySection,
    pub rows: Vec<LedgerRow>,
}

/// Build one row for `path` at `r#ref`. Returns `None` (and the offender
/// string) when no category rule covers the path — never a blank row.
pub fn build_row(ref_sha: &str, path: &str) -> Result<LedgerRow, String> {
    let Some(category) = classify::classify(path) else {
        return Err(format!("UNMAPPED {ref_sha} {path}"));
    };
    Ok(LedgerRow {
        id: format!("{ref_sha}:{path}"),
        upstream_path: path.to_owned(),
        category: category.to_owned(),
        tier: tier::tier(path),
        status: "NOT-STARTED",
    })
}

/// Build the full ledger from the two pinned trees. All git access is
/// read-only against `fork`. On any unmapped path the offenders are
/// collected into `unmapped` (the caller must treat a non-empty list as
/// a failed run) but rows still assemble for everything that mapped, so
/// the report shows the whole picture.
pub fn build(fork: &str) -> Result<Ledger, String> {
    let upstream_paths = git::ls_tree(fork, git::UPSTREAM_REF)?;
    let downstream_paths = git::ls_tree(fork, git::DOWNSTREAM_REF)?;
    let overlay_files = git::overlay(fork, git::UPSTREAM_REF, git::DOWNSTREAM_REF)?;

    let mut rows: Vec<LedgerRow> =
        Vec::with_capacity(upstream_paths.len() + downstream_paths.len());
    let mut unmapped: Vec<String> = Vec::new();
    for (r#ref, paths) in [
        (git::UPSTREAM_REF, &upstream_paths),
        (git::DOWNSTREAM_REF, &downstream_paths),
    ] {
        for path in paths {
            match build_row(r#ref, path) {
                Ok(row) => rows.push(row),
                Err(offender) => unmapped.push(offender),
            }
        }
    }
    // Deterministic order: byte-wise by (ref, path) — the same ordering
    // `git ls-tree` produces within one tree, with upstream before
    // downstream across refs.
    rows.sort_by(|a, b| a.id.cmp(&b.id));
    unmapped.sort();

    let mut categories: Vec<(String, usize)> = Vec::new();
    for row in &rows {
        match categories.iter_mut().find(|t| t.0 == row.category) {
            Some(t) => t.1 += 1,
            None => categories.push((row.category.clone(), 1)),
        }
    }
    categories.sort_by(|a, b| a.0.cmp(&b.0));

    let mut tiers: Vec<(&'static str, usize)> = Vec::new();
    for row in &rows {
        match tiers.iter_mut().find(|t| t.0 == row.tier) {
            Some(t) => t.1 += 1,
            None => tiers.push((row.tier, 1)),
        }
    }
    tiers.sort_by(|a, b| a.0.cmp(&b.0));

    let total_added: u64 = overlay_files.iter().map(|f| f.added).sum();
    let total_removed: u64 = overlay_files.iter().map(|f| f.removed).sum();

    Ok(Ledger {
        schema: "uboot-ledger/v1",
        generated_by: "tools/rust-migration/src/bin/uboot-ledger.rs",
        upstream_url: git::UPSTREAM_URL,
        upstream_ref: git::UPSTREAM_REF,
        downstream_ref: git::DOWNSTREAM_REF,
        fork_path: fork.to_owned(),
        upstream_row_count: upstream_paths.len(),
        downstream_row_count: downstream_paths.len(),
        row_count: rows.len(),
        unmapped_count: unmapped.len(),
        unmapped,
        categories: categories
            .into_iter()
            .map(|(category, count)| Tally { category, count })
            .collect(),
        tiers: tiers
            .into_iter()
            .map(|(tier, count)| TierTally { tier, count })
            .collect(),
        overlay: OverlaySection {
            upstream_ref: git::UPSTREAM_REF,
            downstream_ref: git::DOWNSTREAM_REF,
            file_count: overlay_files.len(),
            total_added,
            total_removed,
            files: overlay_files,
        },
        rows,
    })
}

/// Canonical serialization: serde_json compact bytes plus a trailing
/// newline (the repo's existing ledger format).
pub fn to_json(ledger: &Ledger) -> String {
    let mut json = serde_json::to_string(ledger).expect("serialize uboot ledger");
    json.push('\n');
    json
}

/// Write the canonical JSON to `path`, creating the parent directory.
pub fn write_json(path: &std::path::Path, json: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, json)
}
