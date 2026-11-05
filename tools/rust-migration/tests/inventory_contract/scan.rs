use std::{collections::BTreeSet, path::Path, process::Command};

fn git(root: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).into_owned());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

pub fn repo_root(root: &str) -> Result<String, String> {
    Ok(git(root, &["rev-parse", "--show-toplevel"])?
        .trim()
        .to_owned())
}

pub fn verify_ancestor(root: &str, base: &str) -> Result<(), String> {
    git(root, &["cat-file", "-e", &format!("{base}^{{commit}}")])?;
    git(root, &["merge-base", "--is-ancestor", base, "HEAD"]).map(|_| ())
}

pub fn commit(root: &str, message: &str) -> Result<String, String> {
    git(root, &["add", "."])?;
    git(
        root,
        &[
            "-c",
            "user.name=Inventory Test",
            "-c",
            "user.email=inventory@test",
            "commit",
            "-m",
            message,
        ],
    )?;
    Ok(git(root, &["rev-parse", "HEAD"])?.trim().to_owned())
}

fn eligible(path: &str) -> bool {
    path.ends_with(".py") || path.ends_with(".sh") || path.ends_with(".patch")
}

pub fn tracked_current(root: &str) -> Result<BTreeSet<String>, String> {
    let out = git(root, &["ls-files", "-z"])?;
    Ok(out
        .split('\0')
        .filter(|p| eligible(p))
        .map(str::to_owned)
        .collect())
}

pub fn tracked_at(root: &str, revision: &str) -> Result<BTreeSet<String>, String> {
    let out = git(root, &["ls-tree", "-rz", "--name-only", revision])?;
    Ok(out
        .split('\0')
        .filter(|p| eligible(p))
        .map(str::to_owned)
        .collect())
}

pub fn script_paths(root: &str) -> Result<BTreeSet<String>, String> {
    let files = git(root, &["ls-files", "-z"])?;
    let mut result = BTreeSet::new();
    for path in files
        .split('\0')
        .filter(|p| !p.is_empty() && !p.ends_with(".py") && !p.ends_with(".sh"))
    {
        let full = Path::new(root).join(path);
        if let Ok(meta) = full.metadata() {
            if meta.is_file() && meta.permissions().mode() & 0o111 != 0 {
                if std::fs::read(&full)
                    .ok()
                    .is_some_and(|b| b.starts_with(b"#!"))
                {
                    result.insert(path.to_owned());
                }
            }
        }
    }
    Ok(result)
}

use std::os::unix::fs::PermissionsExt;

pub fn rust_native(root: &str) -> Result<BTreeSet<String>, String> {
    let listed = git(root, &["ls-files", "-z"])?;
    let excluded = ["tools/rust-migration/tests/inventory_contract/scan.rs"];
    let mut hits = BTreeSet::new();
    for p in listed
        .split('\0')
        .filter(|p| p.ends_with(".rs") && !excluded.contains(p))
    {
        if let Ok(text) = std::fs::read_to_string(Path::new(root).join(p)) {
            for (n, line) in text.lines().enumerate() {
                let s = line.trim();
                let marker = s.contains("asm!")
                    || s.contains("global_asm!")
                    || s.contains("extern \"C\"")
                    || s.contains("extern {");
                if marker {
                    hits.insert(format!("{p}:{}", n + 1));
                }
            }
        }
    }
    Ok(hits)
}

pub fn patch_assembly(root: &str, patches: &BTreeSet<String>) -> Result<BTreeSet<String>, String> {
    let mut hits = BTreeSet::new();
    for path in patches {
        let text =
            std::fs::read_to_string(Path::new(root).join(path)).map_err(|e| e.to_string())?;
        for (i, line) in text.lines().enumerate() {
            let s = line.trim_start_matches('+').trim();
            if line.starts_with('+') && (s.contains("asm!(") || s.contains("__asm__")) {
                hits.insert(format!("{path}:{}", i + 1));
            }
        }
    }
    Ok(hits)
}

pub fn patch_targets(root: &str, patches: &BTreeSet<String>) -> Result<BTreeSet<String>, String> {
    let mut targets = BTreeSet::new();
    for patch in patches {
        let text =
            std::fs::read_to_string(Path::new(root).join(patch)).map_err(|e| e.to_string())?;
        for line in text
            .lines()
            .filter(|line| line.starts_with("diff --git a/"))
        {
            let Some((_, right)) = line.split_once(" b/") else {
                continue;
            };
            if [".c", ".h", ".S", ".s", ".asm"]
                .iter()
                .any(|ext| right.ends_with(ext))
            {
                targets.insert(right.to_owned());
            }
        }
    }
    Ok(targets)
}
