//! Git enumeration and ledger rows for the repository inventory.
//!
//! Enumeration is *structural*: paths are gathered by rule (a `tests/`,
//! `fixtures/` or `evidence/` tree is test or audit data; a script that
//! drives a native toolchain is a dependency of that toolchain), never
//! by a hardcoded file list, and a file that matches no rule is a
//! failure by design — the run must name the offender.

use std::{
    io::Read,
    path::{Path, PathBuf},
    process::Command,
};

/// One inventory row: a tracked file, its class, its size on disk, and
/// its content digest. Serializes in field order and sorts by path — the
/// ledger is deterministic across platforms by construction.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct InventoryRow {
    pub path: String,
    pub category: String,
    pub size_bytes: u64,
    pub sha256: String,
}

impl InventoryRow {
    pub fn new(path: &str, category: String, size_bytes: u64, sha256: String) -> Self {
        Self {
            path: path.to_owned(),
            category,
            size_bytes,
            sha256,
        }
    }
}

/// Read a whole file into memory (files are processed one at a time, so
/// the repository never sits in memory at once).
pub fn read_file(path: &Path) -> std::io::Result<Vec<u8>> {
    let mut file = std::fs::File::open(path)?;
    let mut content = Vec::new();
    file.read_to_end(&mut content)?;
    Ok(content)
}

/// Digest one file from the already-read bytes (streaming in 64 KiB
/// chunks through the hash, so hashing never doubles the peak).
pub fn digest_of(content: &[u8]) -> String {
    let mut hasher = super::sha256::Sha256::new();
    hasher.update(content);
    super::sha256::hex_lower(&hasher.finish())
}

/// Digest one file on disk, streamed in 64 KiB chunks (the same digest
/// `digest_of` would give, for anything that reads from disk).
pub fn size_and_digest(path: &Path) -> std::io::Result<(u64, String)> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = super::sha256::Sha256::new();
    let mut total: u64 = 0;
    let mut buf = [0u8; 65536];
    loop {
        match file.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                total += n as u64;
                hasher.update(&buf[..n]);
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok((total, super::sha256::hex_lower(&hasher.finish())))
}

fn git_out(args: &[&str], cwd: &Path) -> Result<String, String> {
    let output = Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .map_err(|e| format!("git {args:?}: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(format!("git {args:?} failed: {stderr}"));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// The repository root `cwd` runs inside (git `rev-parse --show-toplevel`).
pub fn repo_root(cwd: &Path) -> Result<PathBuf, String> {
    let out = git_out(&["rev-parse", "--show-toplevel"], cwd)?;
    let line = out.trim_end_matches(['\n', '\r']);
    if line.is_empty() {
        return Err("git rev-parse --show-toplevel printed nothing".to_owned());
    }
    Ok(PathBuf::from(line))
}

/// The commit the scan walked (`git rev-parse HEAD`, trimmed).
pub fn git_head(root: &Path) -> Result<String, String> {
    let out = git_out(&["rev-parse", "HEAD"], root)?;
    let line = out.trim_end_matches(['\n', '\r']);
    if line.is_empty() {
        return Err("git rev-parse HEAD printed nothing".to_owned());
    }
    Ok(line.to_owned())
}

/// True when `cwd` runs inside a linked (secondary) worktree: the
/// absolute git dir returned by `--absolute-git-dir` lies OUTSIDE this
/// worktree's toplevel. The main checkout keeps its `.git` (and its
/// `commondir` file, which points back into `.git`) under its own
/// toplevel, so the containment test answers the main checkout, a
/// linked worktree, and this machine's `$GIT_DIR = .git/worktrees/<name>`
/// case alike.
pub fn is_linked_worktree(cwd: &Path) -> Result<bool, String> {
    let common = git_out(&["rev-parse", "--absolute-git-dir"], cwd)?;
    let common = Path::new(common.trim_end_matches(['\n', '\r']));
    let common = common
        .canonicalize()
        .map_err(|e| format!("resolve git dir {}: {e}", common.display()))?;
    let toplevel = repo_root(cwd)?;
    let toplevel = toplevel
        .canonicalize()
        .map_err(|e| format!("resolve toplevel {}: {e}", toplevel.display()))?;
    Ok(!common.starts_with(&toplevel))
}

/// Every tracked path under `root`, in git's own output order (`git
/// ls-files -z`).
pub fn tracked_paths(root: &Path) -> Result<Vec<String>, String> {
    let out = git_out(&["ls-files", "-z"], root)?;
    Ok(out
        .split('\0')
        .filter(|p| !p.is_empty())
        .map(str::to_owned)
        .collect())
}

/// Build the ledger rows for `paths`. A path whose classification fails
/// — no rule matched, or the file could not be read — becomes a named
/// failure line, never a silent row: a ledger that quietly drops files
/// is a map with blank countries, which is the exact failure mode this
/// tool exists to prevent. Rows sort by path in byte order, so two runs
/// over identical bytes emit identical JSON.
pub fn build_rows(root: &Path, paths: &[String]) -> (Vec<InventoryRow>, Vec<String>) {
    let mut rows = Vec::with_capacity(paths.len());
    let mut failures = Vec::new();
    for path in paths {
        let full = root.join(path);
        let content = match read_file(&full) {
            Ok(content) => content,
            Err(e) => {
                failures.push(format!("{path}: UNREADABLE ({e})"));
                continue;
            }
        };
        match super::classify::classify(path, &content) {
            Some(category) => {
                let digest = digest_of(&content);
                rows.push(InventoryRow::new(
                    path,
                    category.to_owned(),
                    content.len() as u64,
                    digest,
                ));
            }
            None => failures.push(format!("{path}: UNCLASSIFIED (no classification rule)")),
        }
    }
    rows.sort_by(|a, b| a.path.cmp(&b.path));
    (rows, failures)
}
