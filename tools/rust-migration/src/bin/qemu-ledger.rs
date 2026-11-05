//! QEMU source ledger binary (INV-QEMU-01, EXECUTION-PLAN §6.3).
//!
//! Run from the repository root. Walks the git TREE at the pinned ref
//! in the local read-only QEMU fork (never fetches, never writes to the
//! fork), classifies and tiers every path, and writes the deterministic
//! ledger to `workbench/ledger/qemu-ledger.json`. A clean run prints the
//! tallies and exits 0; any unmapped path is named on stderr and the run
//! exits nonzero — "zero unmapped" is the contract. The fork is GPL-2.0
//! external code: the ledger holds rows only, never source.

use std::path::Path;

#[path = "../qemu_ledger/mod.rs"]
mod qemu_ledger;

fn main() {
    let fork = qemu_ledger::git::FORK_PATH;
    let ledger = match qemu_ledger::ledger::build(fork) {
        Ok(ledger) => ledger,
        Err(e) => {
            eprintln!("qemu-ledger: {e}");
            std::process::exit(2);
        }
    };

    // Canonical bytes: compact JSON plus a trailing newline. Written
    // whether or not the run is clean — the offenders are part of the
    // record — but a ledger with offenders never counts as delivered.
    let json = qemu_ledger::ledger::to_json(&ledger);
    let out_rel = Path::new("workbench/ledger/qemu-ledger.json");
    let cwd = std::env::current_dir().expect("current_dir");
    let out_path = cwd.join(out_rel);
    if let Err(e) = qemu_ledger::ledger::write_json(&out_path, &json) {
        eprintln!("qemu-ledger: {e}");
        std::process::exit(2);
    }

    if ledger.unmapped_count > 0 {
        eprintln!(
            "qemu-ledger: {} offender(s), no tree path may be unmapped:",
            ledger.unmapped_count
        );
        for offender in &ledger.unmapped {
            eprintln!("  {offender}");
        }
        std::process::exit(1);
    }

    println!(
        "qemu-ledger: {} rows @ {}, 0 unmapped",
        ledger.row_count,
        &ledger.pin[..12]
    );
    println!(
        "overlay: {} delta rows ({}...{})",
        ledger.overlay.row_count,
        &ledger.overlay.import_ref[..7],
        &ledger.overlay.ref_sha[..12]
    );
    for tally in &ledger.categories {
        println!("  category {:<14} {:>6}", tally.category, tally.count);
    }
    for tally in &ledger.tiers {
        println!("  tier     {:<14} {:>6}", tally.tier, tally.count);
    }
    println!("wrote {}", out_rel.display());
}
