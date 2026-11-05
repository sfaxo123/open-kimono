use fdt_diff::Fdt;
const FIXTURE: &[u8] = include_bytes!("fixtures/live-chosen-splash.dtb");
fn changed(offset: usize, value: u32) -> Vec<u8> {
    let mut bytes = FIXTURE.to_vec();
    bytes[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
    bytes
}
#[test]
fn fixture_parses() {
    assert!(Fdt::parse(FIXTURE).is_ok());
}
#[test]
fn rejects_unsupported_version() {
    assert!(Fdt::parse(&changed(20, 0)).is_err(), "accepted version 0");
}
#[test]
fn rejects_empty_declared_structure() {
    assert!(
        Fdt::parse(&changed(36, 0)).is_err(),
        "walked beyond zero-sized structure block"
    );
}
#[test]
fn rejects_empty_declared_strings() {
    assert!(
        Fdt::parse(&changed(32, 0)).is_err(),
        "resolved names outside zero-sized strings block"
    );
}
#[test]
fn rejects_reservation_offset_outside_blob() {
    assert!(
        Fdt::parse(&changed(16, u32::MAX)).is_err(),
        "ignored invalid reservation map offset"
    );
}
#[test]
fn equal_invalid_inputs_are_rejected() {
    assert!(
        fdt_diff::compare(&[], &[], &["/"]).is_err(),
        "empty buffers reported valid byte-exact DTB equality"
    );
}
#[test]
fn full_tree_reports_empty_child() {
    let mut a = fdt_diff::DtbBuilder::new().open_root();
    let mut b = fdt_diff::DtbBuilder::new().open_root();
    {
        let mut root = a.root_props();
        root.prop("compatible", b"test\0");
    }
    {
        let mut root = b.root_props();
        root.prop("compatible", b"test\0");
        let _child = root.node("extra");
    }
    let a = a.finish();
    let b = b.finish();
    Fdt::parse(&a).unwrap();
    Fdt::parse(&b).unwrap();
    assert!(
        matches!(
            fdt_diff::compare(&a, &b, &["/"]).unwrap(),
            fdt_diff::DiffReport::Different { .. }
        ),
        "full-tree comparison missed propertyless extra node"
    );
}
