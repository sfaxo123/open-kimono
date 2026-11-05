//! Shared constants and error namespace for the proposed COMMAND body v1
//! codec. The request half lives in [`crate::command_record::request`]. The
//! result half is NOT implemented in this slice: the result-only constants,
//! rejections and helpers that were drafted here are removed so nothing in
//! this crate claims a result codec exists. A caller matches one error enum,
//! not two.
//!
//! Nothing here executes anything. These are byte-layout facts plus typed
//! rejections for the body schema documented in the parent module.

use crate::Error;
use core::fmt;

/// Command-body schema version implemented by this module.
pub const COMMAND_BODY_VERSION: u8 = 1;
/// Hard cap on one whole command body (request or, later, result). This is
/// the explicit maximum checked BEFORE any allocation or size arithmetic.
pub const MAX_COMMAND_BODY: usize = 65_536;
/// Minimum and maximum accepted timeout, in milliseconds.
pub const MIN_TIMEOUT_MS: u32 = 1;
pub const MAX_TIMEOUT_MS: u32 = 60_000;
/// Minimum and maximum argv count.
pub const MIN_ARGC: usize = 1;
pub const MAX_ARGC: usize = 32;
/// Maximum byte length of one argument.
pub const MAX_ARG_BYTES: usize = 4_096;
/// Fixed prefix length of a request body (tag + timeout_ms + argc).
pub const REQUEST_HEADER_LEN: usize = 7;

/// Tag byte distinguishing a request body from a result body. The request
/// decoder accepts only `TAG_REQUEST`; anything else is a typed rejection.
pub(crate) const TAG_REQUEST: u8 = 1;
/// Reserved tag of the (unimplemented in this slice) result body, named so
/// the request decoder can state exactly what an unexpected tag was.
pub(crate) const TAG_RESULT: u8 = 2;

/// Body-level rejections for the command codec. Envelope-level problems
/// (bad magic, wrong op at the frame layer, ...) come from [`crate::Frame`]
/// and surface as [`CommandError::Envelope`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandReject {
    /// `Frame` op was not `Op::Command`.
    WrongOp,
    /// Body shorter than the fixed prefix of its section.
    BodyTooShortForHeader,
    /// Actual body length above the 65536-byte hard cap.
    BodyOverHardCap,
    /// First byte was not the request tag 1 (a result body is NOT decoded
    /// by this slice).
    UnexpectedTag,
    /// timeout_ms outside 1..=60000.
    TimeoutOutOfRange,
    /// argc outside 1..=32.
    ArgcOutOfRange,
    /// A declared argument length above 4096.
    ArgLenOverCap,
    /// argv[0] was empty (other args may be empty).
    EmptyArgv0,
    /// An argument contained a NUL byte.
    ArgumentContainsNul,
    /// Declared lengths do not fit the actual body bytes (truncated), or the
    /// body has more bytes than declared (trailing).
    BodyLengthMismatch,
}

/// Reserved constants for the result-body half of command-body v1. The
/// result codec is NOT implemented in this slice — these numbers exist only
/// so the frozen request-side bound (`MAX_COMMAND_BODY`) and the proposed
/// result layout cannot drift apart silently. Nothing here encodes or
/// decodes a result body.
pub mod reserved {
    /// Fixed size of the (proposed, unimplemented) result-body header.
    pub const RESULT_HEADER_LEN: usize = 14;
    /// Proposed maximum byte length of each output stream.
    pub const MAX_STREAM_BYTES: usize = 65_536;
}

/// How a command finished — the PROPOSED status vocabulary for the result
/// half, which is NOT implemented in this slice. It is exported only as a
/// frozen naming contract; no codec here produces or consumes these values.
/// Deliberately NO "acknowledged" variant: an ACK is not a completed result.
/// `Unknown` means the command MAY have executed — it never licenses a safe
/// retry, and this module makes no claim that any command actually ran.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Exited = 0,
    TimedOut = 1,
    Cancelled = 2,
    Unknown = 3,
    Signaled = 4,
}

/// Body-level rejections reserved for the result half (unimplemented in this
/// slice). They are named so DC-302 can add a result codec without renaming
/// anything; no code path in this slice produces them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultReject {
    /// Status byte outside 0..=4.
    InvalidStatus,
    /// Detail violated its status rule (nonzero where zero required, out of
    /// range for Signaled).
    DetailInvalidForStatus,
    /// A declared stream length above the result stream cap.
    StreamLenOverCap,
}

/// The single error type for the whole command codec: body rejections stay
/// in the command namespace; envelope failures are mapped through
/// `Envelope`. The core [`Error`] type is never modified.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandError {
    Body(CommandReject),
    Envelope(Error),
}

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CommandError::Body(r) => write!(f, "malformed command body: {r:?}"),
            CommandError::Envelope(e) => write!(f, "envelope error: {e}"),
        }
    }
}
