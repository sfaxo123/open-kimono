//! U-Boot source ledger binary (INV-UB-01, EXECUTION-PLAN Packet 10).
//!
//! Run from the repository root. Walks the git TREES at the two
//! root-frozen pins in the local read-only fork (never fetches, never
//! writes to the fork), classifies and tiers every path, and writes the
//! deterministic ledger to `workbench/ledger/uboot-ledger.json`. A clean
//! run prints the tallies and exits 0; any unmapped path is named on
//! stderr and the run exits nonzero — "zero unmapped" is the contract.

use std::path::Path;

#[path = "../uboot_ledger/mod.rs"]
mod uboot_ledger;

fn main() {
    let fork = uboot_ledger::git::FORK_PATH;
    let ledger = match uboot_ledger::ledger::build(fork) {
        Ok(ledger) => ledger,
        Err(e) => {
            eprintln!("uboot-ledger: {e}");
            std::process::exit(2);
        }
    };

    // Canonical bytes: compact JSON plus a trailing newline. Written
    // whether or not the run is clean — the offenders are part of the
    // record — but a ledger with offenders never counts as delivered.
    let json = uboot_ledger::ledger::to_json(&ledger);
    let out_rel = Path::new("workbench/ledger/uboot-ledger.json");
    let cwd = std::env::current_dir().expect("current_dir");
    let out_path = cwd.join(out_rel);
    if let Err(e) = uboot_ledger::ledger::write_json(&out_path, &json) {
        eprintln!("uboot-ledger: {e}");
        std::process::exit(2);
    }

    if ledger.unmapped_count > 0 {
        eprintln!(
            "uboot-ledger: {} offender(s), no tree path may be unmapped:",
            ledger.unmapped_count
        );
        for offender in &ledger.unmapped {
            eprintln!("  {offender}");
        }
        std::process::exit(1);
    }

    println!(
        "uboot-ledger: {} rows (upstream {} @ {}, downstream {} @ {}), 0 unmapped",
        ledger.row_count,
        ledger.upstream_row_count,
        &ledger.upstream_ref[..12],
        ledger.downstream_row_count,
        &ledger.downstream_ref[..12]
    );
    println!(
        "overlay: {} files, +{}/-{} lines",
        ledger.overlay.file_count, ledger.overlay.total_added, ledger.overlay.total_removed
    );
    for tally in &ledger.categories {
        println!("  category {:<8} {:>6}", tally.category, tally.count);
    }
    for tally in &ledger.tiers {
        println!("  tier     {:<8} {:>6}", tally.tier, tally.count);
    }
    println!("wrote {}", out_rel.display());
}
