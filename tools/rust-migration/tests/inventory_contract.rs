#[path = "inventory_contract/scan.rs"]
mod scan;
#[path = "inventory_contract/validate.rs"]
mod validate;

#[test]
fn inventory_matches_baseline_and_current_tracked_paths() {
    validate::validate(include_str!("../inventory.json"), ".").unwrap();
}

#[test]
fn historical_baseline_allows_later_documentation_descendant() {
    let root = std::env::temp_dir().join(format!("inventory-ancestry-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let run = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .current_dir(&root)
            .output()
            .unwrap()
    };
    assert!(run(&["init", "-q"]).status.success());
    std::fs::write(root.join("source.c"), "int source;\n").unwrap();
    let base = scan::commit(root.to_str().unwrap(), "source baseline").unwrap();
    std::fs::write(root.join("README.md"), "doc-only descendant\n").unwrap();
    scan::commit(root.to_str().unwrap(), "documentation only").unwrap();
    assert!(scan::verify_ancestor(root.to_str().unwrap(), &base).is_ok());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn removing_known_inventory_rows_is_rejected() {
    let mut json: serde_json::Value =
        serde_json::from_str(include_str!("../inventory.json")).unwrap();
    json["script_inventory"].as_array_mut().unwrap().remove(0);
    assert!(validate::validate(&json.to_string(), ".").is_err());
    let mut json: serde_json::Value =
        serde_json::from_str(include_str!("../inventory.json")).unwrap();
    json["patch_surfaces"].as_array_mut().unwrap().remove(0);
    assert!(validate::validate(&json.to_string(), ".").is_err());
    let mut json: serde_json::Value =
        serde_json::from_str(include_str!("../inventory.json")).unwrap();
    let duplicate = json["script_inventory"][0].clone();
    json["script_inventory"]
        .as_array_mut()
        .unwrap()
        .push(duplicate);
    assert!(validate::validate(&json.to_string(), ".").is_err());
    let mut json: serde_json::Value =
        serde_json::from_str(include_str!("../inventory.json")).unwrap();
    json["native_patch_targets"] = serde_json::json!(["src/fabricated.c"]);
    assert!(validate::validate(&json.to_string(), ".").is_err());
}

#[test]
fn native_patch_target_scanner_detects_omitted_ledger_row() {
    let root = std::env::temp_dir().join(format!("inventory-target-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("sample.patch"),
        "diff --git a/drivers/demo.c b/drivers/demo.c\n",
    )
    .unwrap();
    let patches = ["sample.patch".to_owned()].into();
    let actual = scan::patch_targets(root.to_str().unwrap(), &patches).unwrap();
    assert_eq!(actual, ["drivers/demo.c".to_owned()].into());
    assert_ne!(actual, std::collections::BTreeSet::new());
    let _ = std::fs::remove_dir_all(root);
}
