use fdt_diff::{compare_capped, DiffReport, DtbBuilder};
fn tree(value: u8) -> Vec<u8> {
    let mut b = DtbBuilder::new().open_root();
    {
        let mut root = b.root_props();
        for i in 0..100 {
            root.prop(&format!("p{i}"), &[value]);
        }
    }
    b.finish()
}
#[test]
fn diff_limit_bounds_retained_storage() {
    let a = tree(1);
    let b = tree(2);
    let DiffReport::Different {
        diffs, truncated, ..
    } = compare_capped(&a, &b, &["/"], 4).unwrap()
    else {
        panic!("expected differences")
    };
    assert!(truncated);
    assert_eq!(diffs.len(), 4);
    assert!(
        diffs.capacity() <= 8,
        "cap4 retained capacity {}: collected whole subtree before truncation",
        diffs.capacity()
    );
}
#[test]
fn unexamined_tail_cannot_prove_structural_equality() {
    fn many(last: u8) -> Vec<u8> {
        let mut b = DtbBuilder::new().open_root();
        {
            let mut r = b.root_props();
            r.prop("compatible", b"test\0");
            for i in 0..12 {
                let mut n = r.node(&format!("n{i:02}"));
                n.prop("value", &[if i == 11 { last } else { 0 }]);
            }
        }
        b.finish()
    }
    let a = many(1);
    let b = many(2);
    let report = compare_capped(&a, &b, &["/"], 2).unwrap();
    assert!(
        !matches!(
            report,
            DiffReport::StructurallyEqual { .. } | DiffReport::Equal
        ),
        "unexamined differing tail reported equal: {report:?}"
    );
}
// HW-09D (root probe test 3): a NON-EMPTY partial report must carry the
// walk truncation. Root prop differs AND the LAST of 12 children differs
// deeper down, under cap 4: the walk is budget-blind past its window, so
// `Different.truncated` must be true — a 1-diff report must not imply a
// complete walk. Mirrors $HOME/artifacts/handoff/redox-perplexity/
// FDT-CAP-ROOT-PROBE.rs::partial_difference_report_marks_unexamined_tail
// in semantics (the probe file itself is root's, untouched).
#[test]
fn partial_difference_report_marks_unexamined_tail() {
    fn many(value: u8) -> Vec<u8> {
        let mut b = DtbBuilder::new().open_root();
        {
            let mut r = b.root_props();
            r.prop("compatible", &[value]);
            for i in 0..12 {
                let mut n = r.node(&format!("n{i:02}"));
                n.prop("value", &[if i == 11 { value } else { 0 }]);
            }
        }
        b.finish()
    }
    let a = many(1);
    let b = many(2);
    let report = compare_capped(&a, &b, &["/"], 4).unwrap();
    let DiffReport::Different { truncated, .. } = report else {
        panic!("expected real prefix diff: {report:?}")
    };
    assert!(
        truncated,
        "walk omitted differing tail but report implies complete: {report:?}"
    );
}
// HW-09D counterpart: a walk that COMPLETES fully under the cap must keep
// `truncated == false` — the flag marks blindness, not mere capping.
#[test]
fn full_walk_under_cap_is_not_truncated() {
    fn many(value: u8) -> Vec<u8> {
        let mut b = DtbBuilder::new().open_root();
        {
            let mut r = b.root_props();
            r.prop("compatible", &[value]);
            for i in 0..12 {
                let mut n = r.node(&format!("n{i:02}"));
                n.prop("value", &[if i == 11 { value } else { 0 }]);
            }
        }
        b.finish()
    }
    let a = many(1);
    let b = many(2);
    // Cap 64: the whole tree (13 nodes + 13 props) fits the entry budget,
    // so both differing sites are examined and nothing is blind.
    let report = compare_capped(&a, &b, &["/"], 64).unwrap();
    match report {
        DiffReport::Different {
            diffs, truncated, ..
        } => {
            assert!(!truncated, "complete walk must not claim truncation");
            assert!(
                diffs.iter().any(|d| matches!(
                    d,
                    fdt_diff::Diff::ValueMismatch { path, .. } if path == "/n11"
                )),
                "complete walk must find the tail diff: {diffs:?}"
            );
        }
        other => panic!("expected full Different with both diffs, got {other:?}"),
    }
    // And the uncapped public path never reports truncated (HW-09 contract).
    let report = fdt_diff::compare(&a, &b, &["/"]).unwrap();
    let DiffReport::Different { truncated, .. } = report else {
        panic!("expected Different: {report:?}")
    };
    assert!(!truncated, "uncapped compare must never report truncated");
}
