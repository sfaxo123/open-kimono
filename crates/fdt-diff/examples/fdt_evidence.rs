//! HW-08S (Lane H) G3 evidence runner — salvage of HW-08 onto the strict
//! v17 parser, WITH the ranking fix.
//!
//! Usage: fdt_evidence <img-path> <captured-blob-path>
//!
//! Reads a boot image (e.g. op6-embedded-efi-redox.img) and the phone's
//! byte-exact captured fdt blob (from /sys/firmware/fdt), extracts every
//! DTB candidate embedded in the image via `extract_dtbs` (offsets are
//! EVIDENCE, never hardcoded), parses each with the STRICT parser, and
//! compares the FULL tree (subtree "/") of the captured blob against each
//! candidate.
//!
//! Prints per candidate: offset, totalsize, verdict (DiffReport::verdict).
//! The best candidate is chosen by `best_candidate_rank`: BYTE-EQUAL beats
//! structurally-equal beats different (fewest diffs), ties break on lowest
//! offset. (HW-08's defect: it took the FIRST zero-diff structurally-equal
//! candidate even when a later one was byte-equal.)
//! Exit code 0 iff a byte-equal candidate exists, else 1.
//! No hashing inside — the operator pipes sha256sum externally.

use fdt_diff::{best_candidate_rank, compare, extract_dtbs, Diff, DiffReport};
use std::fs;
use std::process;

const FULL_TREE: &[&str] = &["/"];

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("usage: fdt_evidence <img-path> <captured-blob-path>");
        process::exit(2);
    }
    let img = match fs::read(&args[1]) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("cannot read img {}: {}", args[1], e);
            process::exit(2);
        }
    };
    let captured = match fs::read(&args[2]) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("cannot read captured blob {}: {}", args[2], e);
            process::exit(2);
        }
    };

    let candidates = extract_dtbs(&img);
    println!(
        "img: {} ({} B) — {} DTB candidate(s) extracted",
        args[1],
        img.len(),
        candidates.len()
    );
    println!("captured: {} ({} B)", args[2], captured.len());

    // Compare every candidate, then rank. The ranking lives in the library
    // (`best_candidate_rank`) so tests/extract.rs can pin the byte-equal-
    // first rule without re-implementing it here.
    let verdicts: Vec<Result<DiffReport, (usize, fdt_diff::FdtError)>> = candidates
        .iter()
        .map(|cand| {
            let end = cand.offset + cand.totalsize;
            compare(&captured, &img[cand.offset..end], FULL_TREE)
        })
        .collect();

    let mut found_equal = false;
    for (cand, verdict) in candidates.iter().zip(&verdicts) {
        match verdict {
            Ok(report) => {
                println!(
                    "candidate offset {} totalsize {} -> {}",
                    cand.offset,
                    cand.totalsize,
                    report.verdict()
                );
                if let DiffReport::Incomplete { .. } = report {
                    println!("INCOMPLETE (budget exhausted, no diffs seen)");
                }
                if let DiffReport::Different {
                    diffs, truncated, ..
                } = report
                {
                    if *truncated {
                        println!("  TRUNCATED (diff collection stopped at MAX_DIFFS)");
                    }
                    for d in diffs {
                        println!("  diff: {}", fmt_diff(d));
                    }
                }
                if report.is_equal() {
                    found_equal = true;
                }
            }
            Err((side, e)) => {
                println!(
                    "candidate offset {} totalsize {} -> PARSE FAIL side {} {:?}",
                    cand.offset, cand.totalsize, side, e
                );
            }
        }
    }

    match best_candidate_rank(&candidates, &verdicts) {
        Some(i) => {
            let cand = &candidates[i];
            let verdict = match &verdicts[i] {
                Ok(r) => r.verdict().into_owned(),
                Err((side, e)) => format!("PARSE FAIL side {} {:?}", side, e),
            };
            println!(
                "best candidate: offset {} totalsize {} verdict {}",
                cand.offset, cand.totalsize, verdict
            );
        }
        None => {
            println!("best candidate: none (no DTB found in img)");
        }
    }

    process::exit(if found_equal { 0 } else { 1 });
}

fn fmt_diff(d: &Diff) -> String {
    match d {
        Diff::MissingInB { path } => format!("MissingInB {}", path),
        Diff::MissingInA { path } => format!("MissingInA {}", path),
        Diff::PropMissingInB { path, name } => format!("PropMissingInB {} {}", path, name),
        Diff::PropMissingInA { path, name } => format!("PropMissingInA {} {}", path, name),
        Diff::ValueMismatch {
            path,
            name,
            a_off,
            b_off,
            a_len,
            b_len,
        } => format!(
            "ValueMismatch {} {} a_off {:#x} b_off {:#x} a_len {} b_len {}",
            path, name, a_off, b_off, a_len, b_len
        ),
        Diff::NameMismatch {
            parent,
            a_name,
            b_name,
        } => format!("NameMismatch {} {} vs {}", parent, a_name, b_name),
    }
}
