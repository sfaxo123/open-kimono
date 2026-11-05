use alloc::string::String;
use core::fmt;

use crate::caps::{Capability, DenyReason};
use crate::limits::ResourceDenial;

/// Raised inside a host function to stop the guest; mapped to
/// [`SandboxError`] when the entry-point call returns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HostTrap {
    Denied { capability: Capability, reason: DenyReason },
    /// The guest passed a bad pointer/length, oversize data, or exceeded a
    /// host-side quota in a way that cannot be reported as a return code.
    BadCall(String),
}

impl fmt::Display for HostTrap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HostTrap::Denied { capability, reason } => {
                write!(f, "capability `{capability}` refused: {}", reason.as_str())
            }
            HostTrap::BadCall(msg) => write!(f, "bad host call: {msg}"),
        }
    }
}

impl core::error::Error for HostTrap {}
impl wasmi::errors::HostError for HostTrap {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SandboxError {
    /// The bytes are not a valid Wasm module.
    Compile(String),
    /// The module imports something outside the `op6` ABI (or a non-function).
    UnknownImport { module: String, name: String },
    /// Start functions are refused: code runs only via explicit entry points.
    StartFunction,
    /// Instantiation would exceed the memory/table cap.
    ResourceCap(ResourceDenial),
    Instantiate(String),
    MissingExport(String),
    /// A host call needed a capability the service does not hold.
    Denied { capability: Capability, reason: DenyReason },
    /// The entry point ran out of fuel (runaway loop).
    OutOfFuel { fuel_per_call: u64 },
    BadHostCall(String),
    Trap(String),
    /// The service was stopped by an earlier failure and is not reused.
    Stopped,
}

impl fmt::Display for SandboxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SandboxError::Compile(e) => write!(f, "invalid module: {e}"),
            SandboxError::UnknownImport { module, name } => {
                write!(f, "import `{module}.{name}` is not part of the sandbox ABI")
            }
            SandboxError::StartFunction => f.write_str("start functions are not allowed"),
            SandboxError::ResourceCap(d) => write!(f, "resource cap: {d:?}"),
            SandboxError::Instantiate(e) => write!(f, "instantiation failed: {e}"),
            SandboxError::MissingExport(n) => write!(f, "missing export `{n}`"),
            SandboxError::Denied { capability, reason } => {
                write!(f, "capability `{capability}` refused: {}", reason.as_str())
            }
            SandboxError::OutOfFuel { fuel_per_call } => {
                write!(f, "stopped: out of fuel ({fuel_per_call} per call)")
            }
            SandboxError::BadHostCall(e) => write!(f, "bad host call: {e}"),
            SandboxError::Trap(e) => write!(f, "trap: {e}"),
            SandboxError::Stopped => f.write_str("service is stopped"),
        }
    }
}

impl core::error::Error for SandboxError {}
