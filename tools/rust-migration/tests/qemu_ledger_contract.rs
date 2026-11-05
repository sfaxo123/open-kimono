//! QEMU ledger contract tests (INV-QEMU-01, EXECUTION-PLAN §6.3;
//! overlay section re-scoped by INV-QEMU-01L).
//!
//! Contracts against the module as the `qemu-ledger` binary wires it:
//! (a) the row count equals the independent `git ls-tree -r` count at
//! the pin; (b) every row carries a category, a tier, and status
//! NOT-STARTED; (c) the custom-overlay section equals the FULL delta
//! `git diff --name-status -M <import>...<pin>` — an independent count,
//! valid A/M/D/R statuses, renames carrying both paths, and the
//! previously-listed T1/PROVENANCE files present in the delta; (d) an
//! injected unmapped path fails and names the offender.

use std::process::Command;

#[path = "../src/qemu_ledger/mod.rs"]
mod qemu_ledger;

/// The fork the pin lives in — read-only by contract (ls-tree/rev-parse/
/// diff only; GPL-2.0 external code, rows only).
const FORK: &str = qemu_ledger::git::FORK_PATH;

/// The 5 files the pre-INV-QEMU-01L overlay section listed (the 4
/// PROVENANCE paths plus `hw/ufs/qcom-ufshc.c`, the T1 UFS surface).
/// Four of them are genuinely custom and must appear in the full delta
/// rows. `ui/input.c` is deliberately NOT required: verified at the pin
/// its blob is byte-identical to the import commit (the fork never
/// touched it; PROVENANCE.md names it spec-only), so it is correctly
/// absent from the `import...pin` delta — and a test asserts that.
const PREVIOUSLY_LISTED_OVERLAY_FILES: [&str; 4] = [
    "hw/input/rmi4-s3706.c",
    "hw/intc/qcom-pdc.c",
    "hw/gpio/qcom-tlmm.c",
    "hw/ufs/qcom-ufshc.c",
];

fn ledger() -> qemu_ledger::ledger::Ledger {
    qemu_ledger::ledger::build(FORK).expect("build ledger")
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

/// Independent `git rev-parse <ref>:<path>` — the oracle for a blob hash.
#[allow(dead_code)]
fn blob_hash(r#ref: &str, path: &str) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(FORK)
        .args(["rev-parse", &format!("{ref}:{path}")])
        .output()
        .expect("git rev-parse");
    assert!(output.status.success(), "git rev-parse {ref}:{path} failed");
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

/// Independent `git diff --name-status -M <import>...<ref>` — the oracle
/// for the overlay delta: raw status lines, rename/copy lines counted as
/// ONE row carrying both paths.
fn diff_name_status(import: &str, r#ref: &str) -> Vec<(String, Vec<String>)> {
    let output = Command::new("git")
        .arg("-C")
        .arg(FORK)
        .args(["diff", "--name-status", "-M", &format!("{import}...{ref}")])
        .output()
        .expect("git diff --name-status");
    assert!(
        output.status.success(),
        "git diff --name-status -M {import}...{ref} failed"
    );
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| {
            let mut fields = l.split('\t');
            let status = fields.next().unwrap_or("").to_owned();
            let paths = fields.map(str::to_owned).collect::<Vec<_>>();
            (status, paths)
        })
        .collect()
}

/// (a) One row per file at the pin: the ledger's rows equal an
/// independent `git ls-tree -r` count at that ref.
#[test]
fn row_count_equals_ls_tree_count() {
    let l = ledger();
    assert!(l.unmapped.is_empty(), "unmapped: {:?}", l.unmapped);
    let oracle = ls_tree_count(qemu_ledger::git::REF);
    assert_eq!(oracle, 11296, "pin is root-frozen; tree size is fixed");
    assert_eq!(l.row_count, oracle, "one row per ls-tree entry");
    assert_eq!(l.rows.len(), oracle);
    for row in &l.rows {
        assert!(
            row.id.starts_with(qemu_ledger::git::REF),
            "row id must carry the pin: {}",
            row.id
        );
    }
}

/// (b) Every row has a non-empty category from the brief's fixed set, a
/// tier in {T1,T2,T3}, and status NOT-STARTED — the ledger's whole
/// purpose is a complete map of untouched surfaces at admission.
#[test]
fn every_row_has_category_tier_and_not_started() {
    let l = ledger();
    assert!(!l.rows.is_empty(), "ledger must not be empty");
    for row in &l.rows {
        assert_eq!(
            row.id,
            format!("{}:{}", qemu_ledger::git::REF, row.upstream_path),
            "id must be ref + path for {}",
            row.id
        );
        assert!(
            matches!(
                row.category.as_str(),
                "machine"
                    | "device"
                    | "core"
                    | "build"
                    | "config"
                    | "doc"
                    | "test"
                    | "qtest-harness"
                    | "other-known"
            ),
            "row {} has bad category {}",
            row.id,
            row.category
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

/// (c) The custom-overlay section is the FULL delta: its row count
/// equals an independent `git diff --name-status -M <import>...<pin>`
/// count (rename = ONE row with both paths), every row carries a valid
/// A/M/D/R status, rename rows carry both paths, and the 5 previously-
/// listed T1/PROVENANCE overlay files appear among the delta rows.
#[test]
fn overlay_section_is_the_full_git_delta() {
    let l = ledger();

    // (a) Count from git, not from the ledger's own bookkeeping.
    let oracle = diff_name_status(qemu_ledger::git::IMPORT_REF, qemu_ledger::git::REF);
    assert_eq!(
        l.overlay.row_count,
        oracle.len(),
        "overlay row count must equal the independent git diff count"
    );
    assert_eq!(
        l.overlay.rows.len(),
        oracle.len(),
        "overlay row_count must match the rows actually present"
    );
    assert_eq!(
        l.overlay.row_count, 95,
        "root-verified: the fork carries a 95-path custom delta at the pin"
    );
    // Full ids recorded in the JSON.
    assert_eq!(l.overlay.import_ref, qemu_ledger::git::IMPORT_REF);
    assert_eq!(l.overlay.ref_sha, qemu_ledger::git::REF);

    // (b) Every row has a valid change status; ids carry the pin.
    for row in &l.overlay.rows {
        assert!(
            matches!(row.change, 'A' | 'M' | 'D' | 'R'),
            "overlay row {} has invalid change {}",
            row.path,
            row.change
        );
        assert!(
            row.id.starts_with(qemu_ledger::git::REF),
            "overlay row id must carry the pin: {}",
            row.id
        );
    }

    // (c) Rename rows carry BOTH paths; non-rename rows carry only one.
    for row in &l.overlay.rows {
        if row.change == 'R' {
            assert!(
                row.old_path.is_some(),
                "rename row {} must carry the old path too",
                row.path
            );
        } else {
            assert!(
                row.old_path.is_none(),
                "non-rename row {} must not carry an old path",
                row.path
            );
        }
    }
    // And the ledger's rename rows agree 1:1 with git's rename lines.
    let oracle_renames = oracle
        .iter()
        .filter(|(s, _)| s.starts_with('R') || s.starts_with('C'))
        .count();
    let ledger_renames = l.overlay.rows.iter().filter(|r| r.change == 'R').count();
    assert_eq!(ledger_renames, oracle_renames, "rename rows match git 1:1");

    // Row-for-row agreement with the oracle (same order, same paths).
    for (row, (status, paths)) in l.overlay.rows.iter().zip(oracle.iter()) {
        assert_eq!(
            row.change,
            status.chars().next().unwrap(),
            "row {} status must match git",
            row.path
        );
        assert_eq!(row.path, paths[0], "row path must match git");
        if paths.len() > 1 {
            assert_eq!(row.old_path.as_deref(), Some(paths[1].as_str()));
        }
    }

    // (d) The previously-listed custom overlay files appear in the
    // delta rows.
    for path in PREVIOUSLY_LISTED_OVERLAY_FILES {
        assert!(
            l.overlay.rows.iter().any(|r| r.path == path),
            "overlay delta must contain the previously-listed {path}"
        );
    }
    // ui/input.c was in the old hand-curated overlay list but is NOT a
    // fork modification: its blob at the pin equals the import commit's
    // (independently re-resolved here), so the honest delta omits it.
    assert_eq!(
        blob_hash(qemu_ledger::git::IMPORT_REF, "ui/input.c"),
        blob_hash(qemu_ledger::git::REF, "ui/input.c"),
        "ui/input.c must be unchanged between import and pin"
    );
    assert!(
        !l.overlay.rows.iter().any(|r| r.path == "ui/input.c"),
        "unchanged ui/input.c must not appear in the delta"
    );
    // Three of the 4 PROVENANCE anchors are in the delta (the fourth is
    // exactly the unchanged ui/input.c above).
    let provenance_in_delta = qemu_ledger::ledger::PROVENANCE_PATHS
        .iter()
        .filter(|p| l.overlay.rows.iter().any(|r| r.path == **p))
        .count();
    assert_eq!(
        provenance_in_delta, 3,
        "3 of 4 PROVENANCE paths are fork changes"
    );
}

/// (e) INV-QEMU-01L2 rename-path contract: git emits rename lines as
/// `R100<TAB>old_path<TAB>new_path` — the FIRST path is the old one, the
/// SECOND the destination. Fed through the parser via controlled fake
/// status output (root's 9:49 AM PT probe proved the bindings were
/// swapped: old_path came back as the new name).
#[test]
fn rename_line_binds_first_path_to_old_and_second_to_destination() {
    let fake = "R100\told-name.c\tnew-name.c\nM\tsingle.c\nA\tadded.c\nD\tgone.c\n";
    let rows = qemu_ledger::git::parse_name_status(fake, "IMPORT", "REF")
        .expect("parse controlled fake --name-status output");
    assert_eq!(rows.len(), 4);

    let r = &rows[0];
    assert_eq!(r.change, 'R');
    assert_eq!(
        r.old_path.as_deref(),
        Some("old-name.c"),
        "first path is the OLD path"
    );
    assert_eq!(r.path, "new-name.c", "second path is the destination");

    // Single-path statuses keep the unchanged handling: one path, no old_path.
    for (row, change, path) in [
        (&rows[1], 'M', "single.c"),
        (&rows[2], 'A', "added.c"),
        (&rows[3], 'D', "gone.c"),
    ] {
        assert_eq!(row.change, change);
        assert_eq!(row.path, path);
        assert_eq!(row.old_path, None);
    }
}

/// (d) An injected path no rule covers makes classification FAIL and
/// names the offender — never a silent drop, never a blank row. The
/// offender string carries UNMAPPED plus ref and path.
#[test]
fn unmapped_path_fails_naming_the_offender() {
    let offender = "zz-qemu-contract/mystery-surface.qzx";
    // The classification rule itself: no rule covers this path.
    assert_eq!(
        qemu_ledger::classify::classify(offender),
        None,
        "no rule may cover the injected path"
    );
    // And the row builder refuses it loudly.
    let result = qemu_ledger::ledger::build_row(qemu_ledger::git::REF, offender);
    match result {
        Ok(row) => panic!("unmapped path produced a row: {row:?}"),
        Err(message) => {
            assert!(message.contains("UNMAPPED"), "must say UNMAPPED: {message}");
            assert!(
                message.contains(offender),
                "must name the offender: {message}"
            );
            assert!(
                message.contains(qemu_ledger::git::REF),
                "must name the ref: {message}"
            );
        }
    }
}
