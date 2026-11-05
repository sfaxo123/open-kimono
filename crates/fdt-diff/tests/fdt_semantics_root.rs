use fdt_diff::{compare, Diff, DiffReport, DtbBuilder, Fdt};
fn tree(value: &[u8], extra: bool) -> Vec<u8> {
    let mut b = DtbBuilder::new().open_root();
    {
        let mut r = b.root_props();
        r.prop("compatible", value);
        if extra {
            let _n = r.node("extra");
        }
    }
    b.finish()
}
#[test]
fn root_property_difference_is_reported() {
    let a = tree(b"aaa\0", false);
    let b = tree(b"bbb\0", false);
    assert!(
        matches!(
            compare(&a, &b, &["/"]).unwrap(),
            DiffReport::Different { .. }
        ),
        "root property mismatch was declared structurally equal"
    );
}
#[test]
fn extra_node_in_b_is_missing_from_a() {
    let a = tree(b"aaa\0", false);
    let b = tree(b"aaa\0", true);
    let DiffReport::Different { diffs, .. } = compare(&a, &b, &["/"]).unwrap() else {
        panic!("missing node unreported")
    };
    assert!(
        diffs
            .iter()
            .any(|d| matches!(d,Diff::MissingInA{path} if path=="/extra")),
        "wrong missing side: {diffs:?}"
    );
}
#[test]
fn reservation_map_cannot_start_at_header_magic() {
    let mut a = include_bytes!("fixtures/live-chosen-splash.dtb").to_vec();
    a[16..20].copy_from_slice(&0u32.to_be_bytes());
    assert!(
        Fdt::parse(&a).is_err(),
        "accepted reservation map pointing into header"
    );
}
