//! What the host log store is willing to treat as a file-system name.
//!
//! The store never invents a directory or file name, and it never follows one
//! it was handed. A name is rejected before any directory or file is touched
//! when it is empty, `.`, `..`, absolute, contains a separator or a NUL, or
//! spells a traversal (`.`/`..`) anywhere in its components.
//!
//! That is a naming rule, not a jail. `parent` is a precondition supplied by
//! the caller: if the caller hands over a path that is itself a symlink or a
//! directory it did not mean to expose, nothing here detects that, and no
//! claim is made that a rejected name would have been contained anyway.

#[cfg(test)]
use std::ffi::OsStr;
#[cfg(unix)]
use std::os::unix::fs::DirBuilderExt;
#[cfg(test)]
use std::path::Component;
use std::path::Path;
use std::path::PathBuf;

/// A name that could only have been meant as an escape or as a mistake.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameReject {
    Empty,
    /// `.`, `..`, or any component that repeats an absolute root.
    Traversal,
    /// Contains `/` or `\`, or was written as an absolute path.
    Separator,
    /// Contains a NUL byte, which cannot be part of a POSIX path anyway.
    InteriorNul,
}

/// True when `name` is a single path component that stays where it is put.
pub fn is_usable_name(name: &str) -> bool {
    if name.is_empty() || name.contains('\0') {
        return false;
    }
    // A backslash is a separator on the platforms this file is written for
    // and on any client that later browses the file, so it is never allowed
    // to reach a path either.
    if name.contains('/') || name.contains('\\') {
        return false;
    }
    if name == "." || name == ".." {
        return false;
    }
    true
}

/// Report why `name` is unusable, for the message that refuses it.
pub fn name_reject(name: &str) -> NameReject {
    if name.is_empty() {
        NameReject::Empty
    } else if name.contains('\0') {
        NameReject::InteriorNul
    } else if name.starts_with('/') || name.contains('/') || name.contains('\\') {
        NameReject::Separator
    } else if name == "." || name == ".." {
        NameReject::Traversal
    } else {
        NameReject::Traversal
    }
}

/// Join a single-component name onto a caller-supplied directory, refusing
/// anything that would not stay inside it. The parent is not investigated:
/// it is a precondition, so `parent/../x` is refused as a *name* problem, not
/// unwrapped as a path one.
pub fn child(parent: &Path, name: &str) -> Result<PathBuf, NameReject> {
    if !is_usable_name(name) {
        return Err(name_reject(name));
    }
    Ok(parent.join(name))
}

/// True when `name` survives being split into path components without any
/// component being `.` or `..`. Kept as the narrow, independently testable
/// half of the naming rule: a name with no separator can still be `..`.
#[cfg(test)]
pub fn components_are_plain(name: &str) -> bool {
    let candidate = Path::new(OsStr::new(name));
    candidate
        .components()
        .all(|c| matches!(c, Component::Normal(_)))
}

/// Create `dir` with mode 0o700, private at the moment of creation, and
/// never through a permissive interim state. `create_dir` reports `Already
/// Exists` without following anything, so an existing directory — including a
/// symlink planted at that name — fails the call instead of being entered.
/// A failed create may leave an explicitly named partial directory behind:
/// deleting what a caller might have created is not ours to decide.
#[cfg(unix)]
pub fn create_private_dir(path: &Path) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(false).mode(0o700);
    builder.create(path)
}

#[cfg(not(unix))]
pub fn create_private_dir(path: &Path) -> std::io::Result<()> {
    std::fs::DirBuilder::new().create(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escaping_and_multipart_names_are_refused() {
        for bad in [
            "",
            ".",
            "..",
            "/etc",
            "a/b",
            "a\\b",
            "../logs",
            "..\\..\\x",
            "a\0b",
        ] {
            assert!(!is_usable_name(bad), "{bad:?} must not be a name");
            assert!(child(Path::new("/tmp"), bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_plain_name_joins_and_a_separator_never_survives_splitting() {
        assert!(components_are_plain("logs"));
        assert!(components_are_plain("host.log"));
        assert!(!components_are_plain("../logs"));
        assert!(!components_are_plain("./logs"));
        assert!(!components_are_plain("/etc"));
        // A separator-free name still cannot hide a traversal component.
        assert!(child(Path::new("/tmp"), "..").is_err());
    }
}
