//! Read-only git enumeration against the local QEMU fork at the pin.
//!
//! Only `git ls-tree -r`, `git rev-parse`, and `git diff --name-status`
//! are ever run, always with `-C <fork>`; the fork is read-only by
//! contract and nothing here fetches, writes, or pushes.

use std::process::Command;

/// Pinned ref (proposed at RUST-MIGRATION.md line 90 — the fork HEAD,
/// "Merge op6/m13-ufs-flake"; root confirms at review). Do not bump,
/// narrow, or repoint this.
pub const REF: &str = "c51d86c7c5d6a0e980a3e96050ec68229b5cbe1e";
/// The fork's own import commit — "import: QEMU v11.1.2" — the base of
/// the custom-overlay delta (INV-QEMU-01L: the overlay section is the
/// full `import...pin` diff, not a hand-curated list). Full id resolved
/// from `b5de3fe` in the fork; do not narrow or repoint this.
pub const IMPORT_REF: &str = "b5de3feedd899c35ea5249f95797ffbe52700db6";
/// The local fork root (READ-ONLY: GPL-2.0 external code, rows only).
pub const FORK_PATH: &str = "$HOME/Developer/forks/qemu-oneplus6";

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

/// Resolve the blob hash of `path` at `r#ref` (`git rev-parse <ref>:<path>`).
/// Retained for provenance spot-checks against the fork; the overlay
/// section is the git delta (INV-QEMU-01L) and no longer hashes files.
#[allow(dead_code)]
pub fn blob_hash(fork: &str, r#ref: &str, path: &str) -> Result<String, String> {
    let spec = format!("{ref}:{path}");
    let output = Command::new("git")
        .arg("-C")
        .arg(fork)
        .args(["rev-parse", &spec])
        .output()
        .map_err(|e| format!("git rev-parse {spec}: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "git rev-parse {spec}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let hash = String::from_utf8(output.stdout)
        .map_err(|_| format!("git rev-parse {spec}: non-UTF8 output"))?;
    let hash = hash.trim().to_owned();
    if hash.len() != 40 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(format!("git rev-parse {spec}: not a blob sha1: {hash}"));
    }
    Ok(hash)
}

/// One custom-overlay delta row: a path the local fork changed relative
/// to its own import commit. `change` is the git status letter A/M/D/R;
/// renames (and copies) carry BOTH `old_path` and `path` in one row.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct DeltaRow {
    pub id: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_path: Option<String>,
    pub change: char,
}

/// Enumerate the full custom-overlay delta: `git diff --name-status -M
/// <import>...<pin>` run read-only against the fork. Rename/copy
/// statuses (R/C, optionally with a similarity-score suffix such as
/// `R100`) become ONE row carrying both the old and the new path. Rows
/// are returned in git's own order — deterministic for fixed refs.
pub fn overlay_delta(fork: &str, import: &str, r#ref: &str) -> Result<Vec<DeltaRow>, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(fork)
        .args(["diff", "--name-status", "-M", &format!("{import}...{ref}")])
        .output()
        .map_err(|e| format!("git diff {import}...{ref}: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "git diff {import}...{ref}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let stdout = String::from_utf8(output.stdout)
        .map_err(|_| format!("git diff {import}...{ref}: non-UTF8 path names"))?;
    parse_name_status(&stdout, import, r#ref)
}

/// Parse raw `git diff --name-status -M` output into delta rows. Git's
/// format for a rename/copy line is `status<TAB>old_path<TAB>new_path`
/// (INV-QEMU-01L2: the first path is the OLD one, the second the
/// destination). Split out from `overlay_delta` so the contract test can
/// feed controlled fake status output through the same parser.
pub fn parse_name_status(stdout: &str, import: &str, r#ref: &str) -> Result<Vec<DeltaRow>, String> {
    let mut rows: Vec<DeltaRow> = Vec::new();
    for line in stdout.lines().filter(|l| !l.is_empty()) {
        let mut fields = line.split('\t');
        let (status, path) = match (fields.next(), fields.next()) {
            (Some(s), Some(p)) => (s, p),
            _ => {
                return Err(format!(
                    "git diff {import}...{ref}: unparsable line {line:?}"
                ))
            }
        };
        // Status letters: A, M, D, T, plus R/C with an optional
        // similarity score (R100). Anything else is a format change in
        // git's output — fail loudly rather than mislabel a row.
        let letter = status
            .chars()
            .next()
            .ok_or_else(|| format!("git diff {import}...{ref}: empty status in line {line:?}"))?;
        if !matches!(letter, 'A' | 'M' | 'D' | 'T' | 'R' | 'C') {
            return Err(format!(
                "git diff {import}...{ref}: unknown status {status:?} in line {line:?}"
            ));
        }
        let row = if matches!(letter, 'R' | 'C') {
            // Git emits rename/copy lines as status<TAB>old<TAB>new:
            // the FIRST path is the old path, the SECOND the destination.
            let old = fields.next().ok_or_else(|| {
                format!("git diff {import}...{ref}: rename without new path in line {line:?}")
            })?;
            DeltaRow {
                id: format!("{ref}:{path}=>{old}"),
                path: old.to_owned(),
                old_path: Some(path.to_owned()),
                // Copies are recorded with the rename letter so the
                // row contract stays A|M|D|R.
                change: if letter == 'C' { 'R' } else { letter },
            }
        } else {
            DeltaRow {
                id: format!("{ref}:{path}"),
                path: path.to_owned(),
                old_path: None,
                change: letter,
            }
        };
        if fields.next().is_some() {
            return Err(format!(
                "git diff {import}...{ref}: too many fields in line {line:?}"
            ));
        }
        rows.push(row);
    }
    Ok(rows)
}
