//! Module-local error namespace for the input-body v1 codec.
//!
//! Body rejections stay in the input namespace; envelope-level failures
//! (bad magic, wrong version, trailing frame bytes, output capacity) are
//! mapped through [`InputError::Envelope`] from the core crate error type.

use crate::Error;
use core::fmt;

/// Body-level rejections. Envelope-level failures come from [`crate::Frame`]
/// and surface as [`InputError::Envelope`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputReject {
    /// `Frame` op was not `crate::Op::Input`.
    WrongOp,
    /// Body shorter than the 5-byte fixed header.
    BodyTooShortForHeader,
    /// Declared batch count outside 1..=64 (`MAX_BATCH`), checked from the
    /// header BEFORE any per-event scan or allocation.
    BatchCountOutOfRange,
    /// Header padding bytes (2..5) were not zero.
    ReservedNonZero,
    /// Body length is not exactly header + count x 16 (truncation or
    /// trailing bytes), checked from the count BEFORE any event scan.
    BodyLengthMismatch,
    /// Unknown event tag byte in a record.
    UnknownEventTag,
    /// Pointer action byte outside Tap/Press/Release/Cancel.
    InvalidPointerAction,
    /// Pointer slot outside 0..=9.
    PointerSlotOutOfRange,
    /// X coordinate outside 0..=1080 (`MAX_X`).
    PointerXOutOfRange,
    /// Y coordinate outside 0..=2280 (`MAX_Y`).
    PointerYOutOfRange,
    /// Key phase byte outside Press/Release.
    InvalidKeyPhase,
    /// Key code above 767 (`MAX_KEY_CODE`).
    KeyCodeOutOfRange,
    /// Text byte length declared above 4096 (`MAX_TEXT_BYTES`).
    TextLenOverCap,
    /// Declared text bytes do not fit the record's fixed payload area.
    TextLenOverRecord,
    /// Text bytes are not valid UTF-8.
    TextNotUtf8,
    /// A record field reserved as zero was nonzero (future-proofing: a
    /// decoder never silently ignores a nonzero reserved byte).
    RecordReservedNonZero,
}

/// The single error type for this module: body rejections stay in the input
/// namespace; envelope failures are mapped through `Envelope`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputError {
    Body(InputReject),
    Envelope(Error),
}

impl fmt::Display for InputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InputError::Body(r) => write!(f, "malformed input body: {r:?}"),
            InputError::Envelope(e) => write!(f, "envelope error: {e}"),
        }
    }
}
