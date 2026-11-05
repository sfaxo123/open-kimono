//! Repo-inventory contract tests (INV-01, EXECUTION-PLAN Packet 9).
//!
//! Four focused contracts on the inventory module as the `inventory`
//! binary wires it: every row carries a non-empty category; the row
//! count equals the `git ls-files` count; an unclassifiable path makes
//! the build FAIL and names the offender; and the module's sha256 +
//! size match the real bytes of a sampled tracked file.

use std::path::PathBuf;

#[path = "../src/inventory/mod.rs"]
mod inventory;

fn root() -> PathBuf {
    inventory::scan::repo_root(&std::env::current_dir().expect("current_dir")).expect("repo root")
}

/// (a) Every classification row has a non-empty category — a blank
/// category would be a silent blank on the map the ledger exists to
/// prevent.
#[test]
fn every_row_category_is_non_empty() {
    let root = root();
    let paths = inventory::scan::tracked_paths(&root).expect("tracked paths");
    let (rows, failures) = inventory::scan::build_rows(&root, &paths);
    assert!(failures.is_empty(), "unclassified: {failures:?}");
    assert!(!rows.is_empty(), "ledger must not be empty");
    for row in &rows {
        assert!(
            !row.category.is_empty(),
            "row {} has an empty category",
            row.path
        );
    }
}

/// (b) One row per tracked path: the row count equals the `git
/// ls-files` count, so the ledger is complete, not a map with blank
/// countries.
#[test]
fn row_count_equals_git_ls_files_count() {
    let root = root();
    let paths = inventory::scan::tracked_paths(&root).expect("tracked paths");
    let (rows, failures) = inventory::scan::build_rows(&root, &paths);
    assert!(failures.is_empty(), "unclassified: {failures:?}");
    assert_eq!(rows.len(), paths.len(), "one row per tracked path");
}

/// (c) An injected path no rule covers makes classification FAIL and
/// names the offender — never a silent drop, never a blank row.
#[test]
fn unclassified_path_fails_naming_the_offender() {
    let root = root();
    let offender = "zz-inventory-contract/mystery-thing.qzx";
    let (rows, failures) = inventory::scan::build_rows(&root, &[offender.to_owned()]);
    assert!(rows.is_empty(), "an unclassified file must emit no row");
    assert_eq!(failures.len(), 1, "exactly one failure expected");
    assert!(
        failures[0].starts_with(offender)
            && (failures[0].contains("UNCLASSIFIED") || failures[0].contains("UNREADABLE")),
        "failure must name the offender: {failures:?}"
    );
    // And the classification rule itself: no rule covers this path,
    // so classify returns None — the condition that makes build_rows
    // name it.
    assert_eq!(
        inventory::classify::classify(offender, b"arbitrary bytes"),
        None,
        "no rule may cover the injected path"
    );
}

/// (d) The module's digest and size match the real bytes of a sampled
/// tracked file — cross-checked against the system `sha256sum` so the
/// self-contained implementation is pinned to an external oracle.
#[test]
fn sha256_and_size_match_a_sampled_real_file() {
    let root = root();
    let sample = "tools/rust-migration/src/inventory/sha256.rs";
    let content = inventory::scan::read_file(&root.join(sample)).expect("read sample");

    // The module's own digest path (what build_rows uses).
    let digest = inventory::scan::digest_of(&content);
    assert_eq!(digest.len(), 64, "sha256 hex is 64 lowercase chars");
    assert_eq!(
        digest,
        digest.to_ascii_lowercase(),
        "digest must be lowercase hex"
    );

    // External oracle: system sha256sum on the same file.
    let output = std::process::Command::new("sha256sum")
        .current_dir(&root)
        .arg(sample)
        .output()
        .expect("sha256sum");
    assert!(output.status.success(), "sha256sum failed");
    let oracle = String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .next()
        .expect("sha256sum output")
        .to_owned();
    assert_eq!(digest, oracle, "module digest must match sha256sum");

    // Size: the streamed disk path reports the same byte count as the
    // file's real content.
    let (size, streamed) = inventory::scan::size_and_digest(&root.join(sample)).expect("stat");
    assert_eq!(size, content.len() as u64, "size must match file bytes");
    assert_eq!(streamed, digest, "streamed digest must match one-shot");
}
