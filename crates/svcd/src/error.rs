//! Precise IR rejection: every error names a JSON Pointer into the submitted
//! document, a stable machine code, and a human message.

use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Code {
    /// Not JSON at all.
    Syntax,
    /// Wrong JSON type for a field.
    Type,
    UnknownField,
    MissingField,
    /// Value outside an enum / const.
    Enum,
    Pattern,
    Range,
    Length,
    Duplicate,
    /// A name that should refer to an action/trigger/param/component does not.
    UnknownReference,
    /// `next` edges form a cycle (the action graph must be a DAG).
    Cycle,
    /// Malformed or unresolvable expression / template / sentence.
    Expression,
    /// An action needs a capability the IR does not request.
    Capability,
    /// An action/trigger needs a scheme the IR does not depend on.
    Dependency,
    /// Initial state or a patch path does not fit `state_schema`.
    StateSchema,
    /// Fields that contradict each other (e.g. `read_only` state mutation).
    Contradiction,
    /// Would stop svcd/caps/trace or loop on its own bus emissions.
    LifecycleGuard,
    /// Valid per the D1 schema but not implemented by this svcd MVP.
    Unsupported,
    /// A simulator test case that cannot be run as written.
    Test,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IrError {
    /// RFC 6901 JSON Pointer into the submitted document (`""` = root).
    pub path: String,
    pub code: Code,
    pub message: String,
}

impl IrError {
    pub fn new(path: impl Into<String>, code: Code, message: impl Into<String>) -> Self {
        Self { path: path.into(), code, message: message.into() }
    }
}

impl fmt::Display for IrError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let path = if self.path.is_empty() { "/" } else { &self.path };
        write!(f, "{path}: {:?}: {}", self.code, self.message)
    }
}

/// All problems found in one IR, in document order of discovery.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IrErrors(pub Vec<IrError>);

impl IrErrors {
    pub fn has(&self, path: &str, code: Code) -> bool {
        self.0.iter().any(|e| e.path == path && e.code == code)
    }
}

impl fmt::Display for IrErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "ServiceIR rejected ({} error{}):", self.0.len(), if self.0.len() == 1 { "" } else { "s" })?;
        for e in &self.0 {
            writeln!(f, "  {e}")?;
        }
        Ok(())
    }
}

impl std::error::Error for IrErrors {}

/// JSON Pointer escaping of one segment (RFC 6901: `~` → `~0`, `/` → `~1`).
pub fn ptr_seg(seg: &str) -> String {
    seg.replace('~', "~0").replace('/', "~1")
}

/// Join a pointer with one more segment.
pub fn ptr(base: &str, seg: impl fmt::Display) -> String {
    format!("{base}/{}", ptr_seg(&seg.to_string()))
}
