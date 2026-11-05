//! Read-only git enumeration against the local fork at the frozen pins.
//!
//! Only `git ls-tree -r` and `git diff --numstat` are ever run, always
//! with `-C <fork>`; the fork is read-only by contract and nothing here
//! fetches, writes, or pushes.

use std::process::Command;

/// Root-frozen pins (verbatim from the root HANDOFF 2026-10-28 — NOT
/// worker discretion). Do not bump, narrow, or repoint these.
pub const UPSTREAM_URL: &str = "https://source.denx.de/u-boot/u-boot.git";
pub const UPSTREAM_REF: &str = "8649a54d301a4d0ac526324c7a7ad2574939ff27";
pub const DOWNSTREAM_REF: &str = "3e15c2819caa7e14e1e4b2ff758a41cbc58a7311";
/// The local fork root both objects were verified present in by root.
pub const FORK_PATH: &str = "$HOME/Developer/forks/u-boot";

/// Run `git -C <fork> ls-tree -r --name-only <ref>` and return the blob
/// paths, exactly as git lists them (byte order). Errors name the ref so
/// a missing object is loud, never silent.
pub fn ls_tree(fork: &str, r#ref: &str) -> Result<Vec<String>, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(fork)
        .args(["ls-tree", "-r", "--name-only", r#ref])
        .output()
        .map_err(|e| format!("git ls-tree {ref}: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "git ls-tree {ref}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let stdout = String::from_utf8(output.stdout)
        .map_err(|_| format!("git ls-tree {ref}: non-UTF8 path names"))?;
    Ok(stdout
        .lines()
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect())
}

/// One downstream overlay file: the path plus its diff stat against the
/// upstream pin. `added`/`removed` are line counts from
/// `git diff --numstat`; binary files would report `-`, which is mapped
/// to 0 and never occurs for this overlay (verified by contract test).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct OverlayFile {
    pub path: String,
    pub change_type: &'static str,
    pub added: u64,
    pub removed: u64,
}

/// Enumerate the downstream overlay (what `git diff` reports between the
/// two pins) with per-file line stats, sorted by path. The change type is
/// mapped from git's status letter (A/M/D/R — R reported as "modified"
/// under its post-rename path).
pub fn overlay(fork: &str, upstream: &str, downstream: &str) -> Result<Vec<OverlayFile>, String> {
    let name_status = run_git(
        fork,
        &["diff", "--name-status", upstream, downstream],
        "diff --name-status",
    )?;
    let numstat = run_git(
        fork,
        &["diff", "--numstat", upstream, downstream],
        "diff --numstat",
    )?;

    let statuses: Vec<(char, &str)> = name_status
        .lines()
        .filter(|l| !l.is_empty())
        .map(|line| {
            let mut fields = line.split('\t');
            let letter = fields
                .next()
                .and_then(|f| f.chars().next())
                .ok_or_else(|| format!("overlay: malformed status line {line:?}"))?;
            // Rename/copy lines carry two paths; take the post-change one.
            let path = match letter {
                'R' | 'C' => fields.nth(1).unwrap_or(""),
                _ => fields.next().unwrap_or(""),
            };
            Ok((letter, path))
        })
        .collect::<Result<Vec<_>, String>>()?;

    let stats: Vec<(u64, u64)> = numstat
        .lines()
        .filter(|l| !l.is_empty())
        .map(|line| {
            let mut fields = line.split('\t');
            let next_count = |f: Option<&str>| -> Result<u64, String> {
                match f {
                    // Binary diffs report "-"; treat as zero lines and
                    // say so by returning Ok(0) — the overlay contract
                    // test pins that no binary file is present.
                    Some("-") => Ok(0),
                    Some(n) => n
                        .parse::<u64>()
                        .map_err(|_| format!("overlay: malformed numstat field {n:?}")),
                    None => Err("overlay: truncated numstat line".to_owned()),
                }
            };
            Ok((next_count(fields.next())?, next_count(fields.next())?))
        })
        .collect::<Result<Vec<_>, String>>()?;

    if statuses.len() != stats.len() {
        return Err(format!(
            "overlay: name-status ({}) and numstat ({}) disagree",
            statuses.len(),
            stats.len()
        ));
    }

    let mut files: Vec<OverlayFile> = statuses
        .iter()
        .zip(stats.iter())
        .map(|((letter, path), (added, removed))| OverlayFile {
            path: (*path).to_owned(),
            change_type: match letter {
                'A' => "added",
                'D' => "deleted",
                _ => "modified",
            },
            added: *added,
            removed: *removed,
        })
        .collect();
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}

/// Run a read-only git subcommand in the fork and return stdout.
fn run_git(fork: &str, args: &[&str], what: &str) -> Result<String, String> {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(fork);
    cmd.args(args);
    let output = cmd.output().map_err(|e| format!("git {what}: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "git {what}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    String::from_utf8(output.stdout).map_err(|_| format!("git {what}: non-UTF8 output"))
}
