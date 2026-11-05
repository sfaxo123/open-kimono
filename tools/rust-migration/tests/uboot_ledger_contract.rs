//! U-Boot ledger contract tests (INV-UB-01, EXECUTION-PLAN Packet 10).
//!
//! Four focused contracts against the module as the `uboot-ledger`
//! binary wires it: (a) the row count equals the independent
//! `git ls-tree -r` count at the upstream pin; (b) every row carries a
//! category, a tier, and status NOT-STARTED; (c) the downstream overlay
//! section has exactly 3 files; (d) an injected unmapped path fails and
//! names the offender.

use std::process::Command;

#[path = "../src/uboot_ledger/mod.rs"]
mod uboot_ledger;

/// The fork the pins live in — read-only by contract (ls-tree/diff only).
const FORK: &str = uboot_ledger::git::FORK_PATH;

fn ledger() -> uboot_ledger::ledger::Ledger {
    uboot_ledger::ledger::build(FORK).expect("build ledger")
}

/// The 40-hex ref prefix of a row id (ids are `<sha40>:<path>`).
fn row_id_ref(id: &str) -> &str {
    &id[..40]
}

/// Independent re-run of the enumeration (an external oracle for the
/// row count, not the module's own bookkeeping).
fn ls_tree_count(r#ref: &str) -> usize {
    let output = Command::new("git")
        .arg("-C")
        .arg(FORK)
        .args(["ls-tree", "-r", "--name-only", r#ref])
        .output()
        .expect("git ls-tree");
    assert!(output.status.success(), "git ls-tree {ref} failed");
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|l| !l.is_empty())
        .count()
}

/// (a) One row per file at the upstream pin: the ledger's upstream rows
/// equal an independent `git ls-tree -r` count at that ref.
#[test]
fn upstream_row_count_equals_ls_tree_count() {
    let l = ledger();
    assert!(l.unmapped.is_empty(), "unmapped: {:?}", l.unmapped);
    let oracle = ls_tree_count(uboot_ledger::git::UPSTREAM_REF);
    assert_eq!(oracle, 39192, "pin is root-frozen; tree size is fixed");
    let upstream_rows = l
        .rows
        .iter()
        .filter(|r| r.id.starts_with(uboot_ledger::git::UPSTREAM_REF))
        .count();
    assert_eq!(upstream_rows, oracle, "one row per ls-tree entry");
    assert_eq!(l.upstream_row_count, oracle);
}

/// (b) Every row has a non-empty category, a tier in {T1,T2,T3}, and
/// status NOT-STARTED — the ledger's whole purpose is a complete map of
/// untouched surfaces at admission.
#[test]
fn every_row_has_category_tier_and_not_started() {
    let l = ledger();
    assert!(!l.rows.is_empty(), "ledger must not be empty");
    for row in &l.rows {
        assert!(
            row.id.starts_with(uboot_ledger::git::UPSTREAM_REF)
                || row.id.starts_with(uboot_ledger::git::DOWNSTREAM_REF),
            "row id must be ref+path: {}",
            row.id
        );
        assert_eq!(
            row.id,
            format!("{}:{}", row_id_ref(&row.id), row.upstream_path),
            "id must be ref + path for {}",
            row.id
        );
        assert!(
            !row.category.is_empty(),
            "row {} has empty category",
            row.id
        );
        assert!(
            matches!(row.tier, "T1" | "T2" | "T3"),
            "row {} has bad tier {}",
            row.id,
            row.tier
        );
        assert_eq!(row.status, "NOT-STARTED", "row {}", row.id);
    }
}

/// (c) The downstream overlay section lists exactly the 3 custom files
/// at the downstream pin, with their diff stat.
#[test]
fn overlay_section_has_exactly_three_files() {
    let l = ledger();
    assert_eq!(l.overlay.file_count, 3);
    assert_eq!(l.overlay.files.len(), 3);
    assert_eq!(l.overlay.total_added, 195, "root-verified diff stat");
    assert_eq!(l.overlay.total_removed, 0);
    let paths: Vec<&str> = l.overlay.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(
        paths,
        [
            "board/qualcomm/qcom-phone-headless.config",
            "board/qualcomm/qcom-phone-headless.env",
            "doc/board/qualcomm/oneplus6-headless.rst",
        ]
    );
    for f in &l.overlay.files {
        assert_eq!(f.change_type, "added");
    }
}

/// (d) An injected path no rule covers makes classification FAIL and
/// names the offender — never a silent drop, never a blank row. The
/// offender string carries UNMAPPED plus ref and path.
#[test]
fn unmapped_path_fails_naming_the_offender() {
    let offender = "zz-uboot-contract/mystery-surface.qzx";
    // The classification rule itself: no rule covers this path.
    assert_eq!(
        uboot_ledger::classify::classify(offender),
        None,
        "no rule may cover the injected path"
    );
    // And the row builder refuses it loudly.
    let result = uboot_ledger::ledger::build_row(uboot_ledger::git::UPSTREAM_REF, offender);
    match result {
        Ok(row) => panic!("unmapped path produced a row: {row:?}"),
        Err(message) => {
            assert!(message.contains("UNMAPPED"), "must say UNMAPPED: {message}");
            assert!(
                message.contains(offender),
                "must name the offender: {message}"
            );
            assert!(
                message.contains(uboot_ledger::git::UPSTREAM_REF),
                "must name the ref: {message}"
            );
        }
    }
}
