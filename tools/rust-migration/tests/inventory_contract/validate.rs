use super::scan;
use serde_json::Value;
use std::{collections::BTreeSet, path::Path};

fn paths(value: &Value, field: &str) -> Result<BTreeSet<String>, String> {
    let rows = value[field]
        .as_array()
        .ok_or_else(|| format!("missing {field}"))?;
    let mut result = BTreeSet::new();
    for row in rows {
        let path = row["path"]
            .as_str()
            .or_else(|| row.as_str())
            .ok_or_else(|| format!("invalid path row in {field}"))?
            .to_owned();
        if !result.insert(path.clone()) {
            return Err(format!("duplicate {field} row: {path}"));
        }
    }
    Ok(result)
}

fn refs(value: &Value, field: &str) -> Result<BTreeSet<String>, String> {
    let rows = value[field]
        .as_array()
        .ok_or_else(|| format!("missing {field}"))?;
    let mut result = BTreeSet::new();
    for row in rows {
        let item = row
            .as_str()
            .map(str::to_owned)
            .or_else(|| row["source_ref"].as_str().map(str::to_owned))
            .ok_or_else(|| format!("invalid reference in {field}"))?;
        if !result.insert(item.clone()) {
            return Err(format!("duplicate {field} row: {item}"));
        }
    }
    Ok(result)
}

fn check_refs(root: &str, values: &BTreeSet<String>) -> Result<(), String> {
    for item in values {
        let (file, span) = item
            .split_once(':')
            .ok_or_else(|| format!("invalid source ref {item}"))?;
        let content = std::fs::read_to_string(Path::new(root).join(file))
            .map_err(|e| format!("{item}: {e}"))?;
        let (first, last) = span
            .split_once('-')
            .map(|(a, b)| (a, b))
            .unwrap_or((span, span));
        let a: usize = first
            .parse()
            .map_err(|_| format!("invalid source line {item}"))?;
        let b: usize = last
            .parse()
            .map_err(|_| format!("invalid source line {item}"))?;
        if a == 0 || b < a || content.lines().count() < b {
            return Err(format!("source ref out of range: {item}"));
        }
    }
    Ok(())
}

fn rust_signals(root: &str) -> Result<BTreeSet<String>, String> {
    scan::rust_native(root)
}

pub fn validate(text: &str, root: &str) -> Result<(), String> {
    let root = scan::repo_root(root)?;
    let root = root.as_str();
    let data: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let base = data["baseline_commit"]
        .as_str()
        .ok_or("missing baseline_commit")?;
    scan::verify_ancestor(root, base)?;
    let expected = paths(&data, "script_inventory")?;
    let expected_patch = paths(&data, "patch_surfaces")?;
    let scripts = scan::tracked_at(root, base)?
        .into_iter()
        .filter(|p| p.ends_with(".py") || p.ends_with(".sh"))
        .collect::<BTreeSet<_>>();
    let current = scan::tracked_current(root)?
        .into_iter()
        .filter(|p| p.ends_with(".py") || p.ends_with(".sh"))
        .collect::<BTreeSet<_>>();
    let patches = scan::tracked_at(root, base)?
        .into_iter()
        .filter(|p| p.ends_with(".patch"))
        .collect::<BTreeSet<_>>();
    let current_patches = scan::tracked_current(root)?
        .into_iter()
        .filter(|p| p.ends_with(".patch"))
        .collect::<BTreeSet<_>>();
    if scripts != expected || current != expected {
        return Err(
            "script inventory differs from pinned baseline or current tracked paths".into(),
        );
    }
    if patches != expected_patch || current_patches != expected_patch {
        return Err("patch inventory differs from pinned baseline or current tracked paths".into());
    }
    if paths(&data, "extensionless_scripts")? != scan::script_paths(root)? {
        return Err("extensionless script inventory mismatch".into());
    }
    if refs(&data, "rust_native_signals")? != rust_signals(root)? {
        return Err("Rust FFI/native-boundary signals mismatch".into());
    }
    let assembly = scan::patch_assembly(root, &expected_patch)?;
    if assembly != refs(&data, "patch_inline_assembly_refs")? {
        return Err("patch inline-assembly references mismatch".into());
    }
    if scan::patch_targets(root, &expected_patch)? != paths(&data, "native_patch_targets")? {
        return Err("native patch target inventory mismatch".into());
    }
    let generators = data["generator_signals"]
        .as_array()
        .ok_or("missing generator signals")?;
    for row in generators {
        let reference = row["source_ref"]
            .as_str()
            .ok_or("missing generator source ref")?;
        check_refs(root, &[reference.to_owned()].into())?;
        let (file, line) = reference.split_once(':').unwrap();
        let line: usize = line.parse().map_err(|_| "invalid generator source line")?;
        let source =
            std::fs::read_to_string(Path::new(root).join(file)).map_err(|e| e.to_string())?;
        let needle = row["source_marker"]
            .as_str()
            .ok_or("missing generator marker")?;
        if !source
            .lines()
            .nth(line - 1)
            .is_some_and(|text| text.contains(needle))
        {
            return Err(format!("generator marker missing at {reference}"));
        }
    }
    let delta = paths(&data, "rust_sources_added_after_baseline")?;
    for path in &delta {
        if !Path::new(root).join(path).is_file() {
            return Err(format!("missing Rust delta source {path}"));
        }
    }
    for row in data["external_references"]
        .as_array()
        .ok_or("missing external references")?
    {
        for key in ["upstream_commit", "local_snapshot_commit"] {
            if let Some(hash) = row[key].as_str() {
                if hash.len() == 40 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                    continue;
                }
                return Err(format!("{key} must be a 40-character hexadecimal hash"));
            }
        }
        if row["local_snapshot_commit"].as_str().is_none() {
            return Err("missing local_snapshot_commit".into());
        }
        check_refs(
            root,
            &[row["source_ref"]
                .as_str()
                .ok_or("missing external source ref")?
                .to_owned()]
            .into(),
        )?;
    }
    Ok(())
}
