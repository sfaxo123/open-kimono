//! Ledger document for the QEMU source surfaces at the pin.
//!
//! Deterministic by construction: rows are sorted by (ref, path) in a
//! byte-wise ordering matching `git ls-tree`, keys follow struct field
//! order, and the serialization is compact JSON plus one trailing
//! newline — two runs over the same pinned ref emit identical bytes.

use serde::Serialize;

use super::classify;
use super::git::{self, DeltaRow};
use super::tier;

/// One ledger row per file in the pinned tree. `id` is `ref + path`.
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

/// The custom-overlay section: the FULL delta between the fork's own
/// import commit and the pin — every path the local fork changed, with
/// its git change type (A/M/D/R; renames carry both paths). Counted
/// from git, never hardcoded (INV-QEMU-01L).
#[derive(Serialize)]
pub struct OverlaySection {
    pub import_ref: &'static str,
    pub ref_sha: &'static str,
    pub row_count: usize,
    pub rows: Vec<DeltaRow>,
}

/// The full ledger document.
#[derive(Serialize)]
pub struct Ledger {
    pub schema: &'static str,
    pub generated_by: &'static str,
    pub fork_path: String,
    pub pin: &'static str,
    /// Rows for the full tree at the pin.
    pub row_count: usize,
    /// Paths that matched no category rule (empty on a clean run).
    pub unmapped: Vec<String>,
    pub unmapped_count: usize,
    pub categories: Vec<Tally>,
    pub tiers: Vec<TierTally>,
    pub overlay: OverlaySection,
    pub rows: Vec<LedgerRow>,
}

/// The paths the repo's `docs/PROVENANCE.md` names for the local
/// `qemu-oneplus6` row (verbatim, in the order the row lists them).
/// Kept as the documented provenance anchor; the overlay section is now
/// the full git delta (INV-QEMU-01L), where 3 of these 4 appear — the
/// fourth (`ui/input.c`) is unchanged between import and pin.
#[allow(dead_code)]
pub const PROVENANCE_PATHS: [&str; 4] = [
    "hw/input/rmi4-s3706.c",
    "hw/intc/qcom-pdc.c",
    "hw/gpio/qcom-tlmm.c",
    "ui/input.c",
];

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

/// Build the full ledger from the pinned tree. All git access is
/// read-only against `fork`. On any unmapped path the offenders are
/// collected into `unmapped` (the caller must treat a non-empty list as
/// a failed run) but rows still assemble for everything that mapped, so
/// the report shows the whole picture.
pub fn build(fork: &str) -> Result<Ledger, String> {
    let paths = git::ls_tree(fork, git::REF)?;

    let mut rows: Vec<LedgerRow> = Vec::with_capacity(paths.len());
    let mut unmapped: Vec<String> = Vec::new();
    for path in &paths {
        match build_row(git::REF, path) {
            Ok(row) => rows.push(row),
            Err(offender) => unmapped.push(offender),
        }
    }
    // Deterministic order: byte-wise by (ref, path) — the same ordering
    // `git ls-tree` produces.
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

    // The custom overlay: the full delta from the fork's import commit
    // to the pin, straight from git (rename = one row, both paths).
    let overlay_rows = git::overlay_delta(fork, git::IMPORT_REF, git::REF)?;

    Ok(Ledger {
        schema: "qemu-ledger/v2",
        generated_by: "tools/rust-migration/src/bin/qemu-ledger.rs",
        fork_path: fork.to_owned(),
        pin: git::REF,
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
            import_ref: git::IMPORT_REF,
            ref_sha: git::REF,
            row_count: overlay_rows.len(),
            rows: overlay_rows,
        },
        rows,
    })
}

/// Canonical serialization: serde_json compact bytes plus a trailing
/// newline (the repo's existing ledger format).
pub fn to_json(ledger: &Ledger) -> String {
    let mut json = serde_json::to_string(ledger).expect("serialize qemu ledger");
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
