//! Borrowed `LogRecord` body codec for the accepted control-envelope v1.
//!
//! This module decodes and encodes ONE log-body version 1 payload carried by
//! an already-validated [`Frame`] with `op == Op::Logs`. It is a proposed,
//! frozen body schema for this partial 0.2.1 slice — it is NOT compatible
//! with tracejournal or any existing device transport, and it makes no claim
//! that any live device emits these records.
//!
//! Exact log-body v1 layout (little-endian), fixed 38-byte header:
//! - bytes 0..16   build id, must be nonzero;
//! - bytes 16..24  u64 sequence, must be nonzero;
//! - bytes 24..32  u64 monotonic microseconds (0 is valid);
//! - byte  32      severity, must be in 0..=5;
//! - byte  33      subsystem byte length, must be 1..=64;
//! - bytes 34..38  u32 message byte length, must be 0..=4096;
//! - then exactly `subsystem_bytes` bytes of UTF-8 (no embedded NUL),
//!   then exactly `message_bytes` bytes of UTF-8.
//!
//! A full envelope is the accepted 36-byte frame header plus this body, so
//! the body itself is 39..=4198 bytes (`MAX_LOG_BODY`). Message bytes may
//! contain newlines and NULs and must roundtrip losslessly — this is a
//! binary record codec, not a line protocol. Subsystem must be valid UTF-8
//! with no embedded NUL.
//!
//! Parsing never allocates: subsystem and message borrow the caller's
//! buffer. Encoding writes only into caller-provided output and validates
//! every metadata, UTF-8 and length bound plus output capacity BEFORE any
//! write, so a rejected encode leaves the output byte-for-byte unchanged.
//!
//! This module is deliberately decoupled from the core envelope error type:
//! [`LogError`] carries body rejections as `Body(LogReject)` and maps any
//! envelope-level failure through `Envelope(crate::Error)`.

use crate::{Error, Frame, Op, Policy};
use core::fmt;

/// Log-body schema version implemented by this module.
pub const LOG_BODY_VERSION: u8 = 1;
/// Fixed size of the log-body v1 header.
pub const LOG_HEADER_LEN: usize = 38;
/// Maximum subsystem byte length (the minimum is 1).
pub const MAX_SUBSYSTEM_BYTES: usize = 64;
/// Maximum message byte length.
pub const MAX_MESSAGE_BYTES: usize = 4096;
/// Hard cap on the whole log body: header + max subsystem + max message.
pub const MAX_LOG_BODY: usize = LOG_HEADER_LEN + MAX_SUBSYSTEM_BYTES + MAX_MESSAGE_BYTES;

/// Application log levels (not syslog numbering): Trace0, Debug1, Info2,
/// Warning3, Error4, Fatal5. The wire value is validated before construction,
/// so an invalid severity can never become a `Severity`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Trace = 0,
    Debug = 1,
    Info = 2,
    Warning = 3,
    Error = 4,
    Fatal = 5,
}

impl Severity {
    fn from_u8(b: u8) -> Option<Severity> {
        match b {
            0 => Some(Severity::Trace),
            1 => Some(Severity::Debug),
            2 => Some(Severity::Info),
            3 => Some(Severity::Warning),
            4 => Some(Severity::Error),
            5 => Some(Severity::Fatal),
            _ => None,
        }
    }

    fn to_u8(self) -> u8 {
        self as u8
    }
}

/// Body-level rejections. Envelope-level rejections (bad magic, wrong
/// version, trailing bytes, ...) come from [`Frame::parse`] /
/// [`Frame::encode`] and are surfaced as [`LogError::Envelope`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogReject {
    /// `Frame` op was not `Op::Logs`.
    WrongOp,
    /// Body shorter than the 38-byte fixed header.
    BodyTooShortForHeader,
    /// Actual body length above the 4198-byte hard cap.
    BodyOverHardCap,
    /// Declared subsystem length outside 1..=64.
    SubsystemLenOutOfRange,
    /// Declared message length above 4096.
    MessageLenOverCap,
    /// Declared lengths do not fit the actual body bytes (truncated), or
    /// the body has more bytes than header + declared strings (trailing).
    BodyLengthMismatch,
    ZeroBuildId,
    ZeroSequence,
    InvalidSeverity,
    SubsystemNotUtf8,
    /// Valid UTF-8 subsystem containing an embedded NUL codepoint.
    SubsystemContainsNul,
    /// Message bytes are not valid UTF-8 (distinct from NUL, which is
    /// legal in a message and must roundtrip).
    MessageNotUtf8,
}

/// The single error type for this module: body rejections stay in the log
/// namespace; envelope failures are mapped through `Envelope`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogError {
    Body(LogReject),
    Envelope(Error),
}

impl fmt::Display for LogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LogError::Body(r) => write!(f, "malformed log body: {r:?}"),
            LogError::Envelope(e) => write!(f, "envelope error: {e}"),
        }
    }
}

/// One fully validated log body. Every field is private and only reachable
/// through already-validated accessors; the strings borrow `payload`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogRecord<'a> {
    build_id: [u8; 16],
    sequence: u64,
    monotonic_us: u64,
    severity: Severity,
    subsystem: &'a str,
    message: &'a str,
}

/// A decoded [`LogRecord`] bound to the envelope it arrived in: the record
/// plus the validated correlation id and boot id of that envelope. Use this
/// whenever envelope identity matters; a bare [`LogRecord`] from
/// [`LogRecord::decode`] carries no envelope binding at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FramedLog<'a> {
    record: LogRecord<'a>,
    correlation_id: u64,
    boot_id: [u8; 16],
}

impl<'a> FramedLog<'a> {
    pub fn record(&self) -> &LogRecord<'a> {
        &self.record
    }
    /// The envelope's validated nonzero correlation id (request/response
    /// pairing), not the log body's own build/sequence fields.
    pub fn correlation_id(&self) -> u64 {
        self.correlation_id
    }
    /// The envelope's validated nonzero 16-byte boot id, distinct from the
    /// log body's build id.
    pub fn boot_id(&self) -> &[u8; 16] {
        &self.boot_id
    }
}

impl<'a> LogRecord<'a> {
    /// Construct a producer record without manually manufacturing wire bytes.
    /// Strings remain borrowed; a record is not authenticated or envelope-bound.
    pub fn new(
        build_id: [u8; 16],
        sequence: u64,
        monotonic_us: u64,
        severity: Severity,
        subsystem: &'a str,
        message: &'a str,
    ) -> Result<Self, LogError> {
        if build_id.iter().all(|&x| x == 0) {
            return Err(LogError::Body(LogReject::ZeroBuildId));
        }
        if sequence == 0 {
            return Err(LogError::Body(LogReject::ZeroSequence));
        }
        if subsystem.is_empty() || subsystem.len() > MAX_SUBSYSTEM_BYTES {
            return Err(LogError::Body(LogReject::SubsystemLenOutOfRange));
        }
        if subsystem.as_bytes().contains(&0) {
            return Err(LogError::Body(LogReject::SubsystemContainsNul));
        }
        if message.len() > MAX_MESSAGE_BYTES {
            return Err(LogError::Body(LogReject::MessageLenOverCap));
        }
        Ok(Self {
            build_id,
            sequence,
            monotonic_us,
            severity,
            subsystem,
            message,
        })
    }

    /// Decode a log body from an already-parsed envelope, keeping the
    /// envelope's correlation id and boot id bound to the record. The frame
    /// must carry `op == Op::Logs`; payload bounds are checked BEFORE any
    /// string decoding and every length conversion is checked. Subsystem and
    /// message borrow the original input buffer — no allocation, no copy.
    pub fn from_frame(frame: &Frame<'a>) -> Result<FramedLog<'a>, LogError> {
        if frame.op() != Op::Logs {
            return Err(LogError::Body(LogReject::WrongOp));
        }
        let record = Self::decode(frame.payload())?;
        Ok(FramedLog {
            record,
            correlation_id: frame.correlation_id(),
            boot_id: *frame.boot_id(),
        })
    }

    /// Decode a bare log body (the payload bytes of a `Logs` frame). The
    /// result is explicitly NOT bound to any envelope: it carries no
    /// correlation id or boot id. Use [`LogRecord::from_frame`] when
    /// envelope identity matters.
    pub fn decode(body: &'a [u8]) -> Result<LogRecord<'a>, LogError> {
        if body.len() < LOG_HEADER_LEN {
            return Err(LogError::Body(LogReject::BodyTooShortForHeader));
        }
        // Hard cap on the ACTUAL body bytes, before any decoding.
        if body.len() > MAX_LOG_BODY {
            return Err(LogError::Body(LogReject::BodyOverHardCap));
        }
        let build_id: [u8; 16] = body[0..16].try_into().unwrap();
        if build_id.iter().all(|&x| x == 0) {
            return Err(LogError::Body(LogReject::ZeroBuildId));
        }
        let sequence = u64::from_le_bytes(body[16..24].try_into().unwrap());
        if sequence == 0 {
            return Err(LogError::Body(LogReject::ZeroSequence));
        }
        let monotonic_us = u64::from_le_bytes(body[24..32].try_into().unwrap());
        let severity = match Severity::from_u8(body[32]) {
            Some(s) => s,
            None => return Err(LogError::Body(LogReject::InvalidSeverity)),
        };
        let subsystem_len = usize::from(body[33]);
        if subsystem_len < 1 || subsystem_len > MAX_SUBSYSTEM_BYTES {
            return Err(LogError::Body(LogReject::SubsystemLenOutOfRange));
        }
        let message_declared = u32::from_le_bytes(body[34..38].try_into().unwrap());
        // Compare the declared u32 against the cap BEFORE any usize
        // conversion, never via a lossy cast.
        if message_declared > MAX_MESSAGE_BYTES as u32 {
            return Err(LogError::Body(LogReject::MessageLenOverCap));
        }
        let message_len = usize::try_from(message_declared)
            .map_err(|_| LogError::Body(LogReject::MessageLenOverCap))?;
        // Bounds must be checked BEFORE decoding any string bytes.
        let strings_len = match subsystem_len.checked_add(message_len) {
            Some(n) => n,
            None => return Err(LogError::Body(LogReject::BodyLengthMismatch)),
        };
        let total = match LOG_HEADER_LEN.checked_add(strings_len) {
            Some(n) => n,
            None => return Err(LogError::Body(LogReject::BodyLengthMismatch)),
        };
        if body.len() < total {
            return Err(LogError::Body(LogReject::BodyLengthMismatch));
        }
        if body.len() > total {
            return Err(LogError::Body(LogReject::BodyLengthMismatch));
        }
        let sub_bytes = &body[LOG_HEADER_LEN..LOG_HEADER_LEN + subsystem_len];
        let msg_bytes = &body[LOG_HEADER_LEN + subsystem_len..total];
        let subsystem = match core::str::from_utf8(sub_bytes) {
            Ok(s) => s,
            Err(_) => return Err(LogError::Body(LogReject::SubsystemNotUtf8)),
        };
        if subsystem.bytes().any(|b| b == 0) {
            return Err(LogError::Body(LogReject::SubsystemContainsNul));
        }
        let message = match core::str::from_utf8(msg_bytes) {
            Ok(s) => s,
            Err(_) => return Err(LogError::Body(LogReject::MessageNotUtf8)),
        };
        Ok(LogRecord {
            build_id,
            sequence,
            monotonic_us,
            severity,
            subsystem,
            message,
        })
    }

    pub fn build_id(&self) -> &[u8; 16] {
        &self.build_id
    }
    pub fn sequence(&self) -> u64 {
        self.sequence
    }
    pub fn monotonic_us(&self) -> u64 {
        self.monotonic_us
    }
    pub fn severity(&self) -> Severity {
        self.severity
    }
    /// Borrowed subsystem string; guaranteed nonempty, valid UTF-8, no NUL.
    pub fn subsystem(&self) -> &'a str {
        self.subsystem
    }
    /// Borrowed message string; may contain newlines and NULs and is
    /// preserved losslessly (this codec is not a line protocol).
    pub fn message(&self) -> &'a str {
        self.message
    }

    /// Encode the body into caller-provided `output`. Every metadata value,
    /// UTF-8 requirement, length limit and the output capacity are validated
    /// BEFORE any byte is written, so a rejected encode leaves `output`
    /// byte-for-byte unchanged. Returns the number of bytes written.
    pub fn encode_body(&self, output: &mut [u8]) -> Result<usize, LogError> {
        let sub_bytes = self.subsystem.as_bytes();
        let msg_bytes = self.message.as_bytes();
        if sub_bytes.len() < 1 || sub_bytes.len() > MAX_SUBSYSTEM_BYTES {
            return Err(LogError::Body(LogReject::SubsystemLenOutOfRange));
        }
        if self.subsystem.bytes().any(|b| b == 0) {
            return Err(LogError::Body(LogReject::SubsystemContainsNul));
        }
        // Compare the length against the cap before any conversion.
        let msg_declared = match u32::try_from(msg_bytes.len()) {
            Ok(n) => n,
            Err(_) => return Err(LogError::Body(LogReject::MessageLenOverCap)),
        };
        if msg_declared > MAX_MESSAGE_BYTES as u32 {
            return Err(LogError::Body(LogReject::MessageLenOverCap));
        }
        let total = match LOG_HEADER_LEN.checked_add(sub_bytes.len()) {
            Some(n) => n,
            None => return Err(LogError::Body(LogReject::BodyLengthMismatch)),
        };
        let total = match total.checked_add(msg_bytes.len()) {
            Some(n) => n,
            None => return Err(LogError::Body(LogReject::BodyLengthMismatch)),
        };
        if total > MAX_LOG_BODY {
            return Err(LogError::Body(LogReject::BodyOverHardCap));
        }
        if output.len() < total {
            return Err(LogError::Envelope(Error::OutputTooSmall {
                required: total,
                capacity: output.len(),
            }));
        }
        let sub_declared = u8::try_from(sub_bytes.len())
            .map_err(|_| LogError::Body(LogReject::SubsystemLenOutOfRange))?;
        // All validation passed; only now write.
        output[0..16].copy_from_slice(&self.build_id);
        output[16..24].copy_from_slice(&self.sequence.to_le_bytes());
        output[24..32].copy_from_slice(&self.monotonic_us.to_le_bytes());
        output[32] = self.severity.to_u8();
        output[33] = sub_declared;
        output[34..38].copy_from_slice(&msg_declared.to_le_bytes());
        output[LOG_HEADER_LEN..LOG_HEADER_LEN + sub_bytes.len()].copy_from_slice(sub_bytes);
        let msg_start = LOG_HEADER_LEN + sub_bytes.len();
        output[msg_start..total].copy_from_slice(msg_bytes);
        Ok(total)
    }

    /// Build the full envelope (36-byte frame header + this body) into
    /// caller-provided `output`, correlated with `correlation_id` and bound
    /// to the envelope `boot_id`. All validation happens before any write,
    /// so a rejected encode leaves `output` unchanged. Returns total bytes.
    pub fn encode_envelope(
        &self,
        correlation_id: u64,
        boot_id: &[u8; 16],
        output: &mut [u8],
        policy: &Policy,
    ) -> Result<usize, LogError> {
        let mut body = [0u8; MAX_LOG_BODY];
        let body_len = self.encode_body(&mut body)?;
        Frame::encode(
            Op::Logs,
            correlation_id,
            boot_id,
            &body[..body_len],
            output,
            policy,
        )
        .map_err(LogError::Envelope)
    }
}

impl fmt::Display for LogRecord<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "log v{LOG_BODY_VERSION} seq {} sev {:?} sub {} msg {} bytes",
            self.sequence,
            self.severity,
            self.subsystem.len(),
            self.message.len()
        )
    }
}
