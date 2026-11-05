//! Repo inventory binary (INV-01, EXECUTION-PLAN Packet 9).
//!
//! Run from the repository root. Enumerates `git ls-files`, classifies
//! every tracked file through the inventory module, and writes the
//! deterministic ledger to `workbench/ledger/repo-inventory.json`. A
//! clean run prints the category tallies and exits 0; any unclassified
//! or unreadable file is named on stderr and the run exits nonzero —
//! "zero unclassified" is the tool's whole contract.

use std::collections::BTreeMap;
use std::path::Path;

#[path = "../inventory/mod.rs"]
mod inventory;

fn main() {
    let cwd = std::env::current_dir().expect("current_dir");
    let root = match inventory::scan::repo_root(&cwd) {
        Ok(root) => root,
        Err(e) => {
            eprintln!("inventory: {e}");
            std::process::exit(2);
        }
    };

    let paths = match inventory::scan::tracked_paths(&root) {
        Ok(paths) => paths,
        Err(e) => {
            eprintln!("inventory: {e}");
            std::process::exit(2);
        }
    };

    let baseline = match inventory::scan::git_head(&root) {
        Ok(head) => head,
        Err(e) => {
            eprintln!("inventory: {e}");
            std::process::exit(2);
        }
    };
    let linked = match inventory::scan::is_linked_worktree(&cwd) {
        Ok(linked) => linked,
        Err(e) => {
            eprintln!("inventory: {e}");
            std::process::exit(2);
        }
    };

    let (rows, failures) = inventory::scan::build_rows(&root, &paths);

    // Category tallies (count + total bytes), keyed by category name.
    let mut tallies: BTreeMap<&str, (usize, u64)> = BTreeMap::new();
    for row in &rows {
        let entry = tallies.entry(&row.category).or_insert((0, 0));
        entry.0 += 1;
        entry.1 += row.size_bytes;
    }
    let categories: Vec<inventory::ledger::Category> = tallies
        .iter()
        .map(
            |(category, (count, total_bytes))| inventory::ledger::Category {
                category: (*category).to_owned(),
                count: *count,
                total_bytes: *total_bytes,
            },
        )
        .collect();

    let ledger = inventory::ledger::Ledger {
        baseline_commit: baseline,
        generated_by: "tools/rust-migration/src/bin/inventory.rs",
        schema: "repo-inventory/v1",
        roots: vec![".".to_owned()],
        worktree_mode: if linked { "linked" } else { "main" },
        tracked_count: paths.len(),
        row_count: rows.len(),
        unclassified: failures.clone(),
        unclassified_count: failures.len(),
        categories,
        files: rows,
    };

    // Canonical bytes: compact JSON plus a trailing newline. Written
    // whether or not the run is clean — the failures are part of the
    // record — but a ledger with offenders never counts as delivered.
    let json = inventory::ledger::to_json(&ledger);
    let out_rel = Path::new("workbench/ledger/repo-inventory.json");
    let out_path = root.join(out_rel);
    if let Err(e) = inventory::ledger::write_json(&out_path, &json) {
        eprintln!("inventory: {e}");
        std::process::exit(2);
    }

    if !failures.is_empty() {
        eprintln!(
            "inventory: {} offender(s), no file may be unclassified:",
            failures.len()
        );
        for failure in &failures {
            eprintln!("  {failure}");
        }
        std::process::exit(1);
    }

    println!(
        "inventory: {} tracked files, 0 unclassified",
        ledger.tracked_count
    );
    for category in &ledger.categories {
        println!(
            "  {:<18} {:>5} files  {:>12} bytes",
            category.category, category.count, category.total_bytes
        );
    }
    println!(
        "wrote {} (baseline {})",
        out_rel.display(),
        &ledger.baseline_commit[..12.min(ledger.baseline_commit.len())]
    );
}
