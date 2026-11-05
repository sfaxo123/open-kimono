//! Bounded host-side framed byte API for the proposed control-envelope v1.
//!
//! This is a partial 0.2.1 slice: parsing/encoding of one fixed-size header
//! plus an exactly-declared payload. It is NOT compatible with tracejournal or
//! any existing device transport, and it makes no authenticity claim.
//!
//! Hard limits (a caller policy may only tighten them, never loosen):
//! - 36-byte header, magic `DCTL`, version 1, op in 1..=4, reserved byte 0,
//!   nonzero request/action correlation id, nonzero 16-byte boot id;
//! - payload <= 16 MiB (`MAX_PAYLOAD`), declared length checked before any
//!   size arithmetic or allocation;
//! - no allocation on parse: the payload is borrowed from the caller's input.
//!
//! Not implemented yet — do not infer them from this module:
//! - no log, frame, command or input payload body schema;
//! - no authentication or authenticity of sender/boot id;
//! - no stream reassembly, duplicate or replay tracking;
//! - no actual transport and no phone proof of any kind.

#![forbid(unsafe_code)]
#![no_std]

use core::fmt;

pub mod act;
pub mod command_record;
pub mod fake_transport;
pub mod image_record;
pub mod input_record;
pub mod log_record;
pub mod session;

pub const MAGIC: &[u8; 4] = b"DCTL";
pub const VERSION: u16 = 1;
pub const HEADER_LEN: usize = 36;
pub const MAX_PAYLOAD: u32 = 16 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Logs = 1,
    Screenshot = 2,
    Command = 3,
    Input = 4,
}

impl Op {
    fn from_u8(b: u8) -> Option<Op> {
        match b {
            1 => Some(Op::Logs),
            2 => Some(Op::Screenshot),
            3 => Some(Op::Command),
            4 => Some(Op::Input),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reject {
    InputTooShortForHeader,
    BadMagic,
    UnsupportedVersion,
    UnsupportedOp,
    ReservedNonZero,
    ZeroCorrelationId,
    ZeroBootId,
    DeclaredLengthOverHardCap,
    DeclaredLengthOverPolicyCap,
    TruncatedPayload,
    TrailingBytes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    InvalidPolicy,
    Malformed(Reject),
    OutputTooSmall { required: usize, capacity: usize },
    UnsupportedTransport(&'static str),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::InvalidPolicy => write!(f, "caller policy exceeds the hard cap"),
            Error::Malformed(r) => write!(f, "malformed control envelope: {r:?}"),
            Error::OutputTooSmall { required, capacity } => {
                write!(f, "output too small: need {required}, have {capacity}")
            }
            Error::UnsupportedTransport(t) => write!(f, "transport unsupported: {t}"),
        }
    }
}

/// Caller-supplied bound. Default is the hard cap; a policy looser than the
/// 16 MiB hard cap is rejected with `Error::InvalidPolicy`, never clamped.
#[derive(Debug, Clone, Copy)]
pub struct Policy {
    pub max_payload: u32,
}

impl Default for Policy {
    fn default() -> Self {
        Policy {
            max_payload: MAX_PAYLOAD,
        }
    }
}

fn is_zero_boot(b: &[u8; 16]) -> bool {
    b.iter().all(|&x| x == 0)
}

/// One fully validated envelope. Fields are private and only reachable as
/// already-validated values; the payload borrows the parsed input buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frame<'a> {
    op: Op,
    correlation_id: u64,
    boot_id: [u8; 16],
    payload: &'a [u8],
}

impl<'a> Frame<'a> {
    pub fn op(&self) -> Op {
        self.op
    }
    pub fn correlation_id(&self) -> u64 {
        self.correlation_id
    }
    pub fn boot_id(&self) -> &[u8; 16] {
        &self.boot_id
    }
    pub fn payload(&self) -> &'a [u8] {
        self.payload
    }

    /// Parse without allocating: every rejected case leaves no output and the
    /// payload is a borrow of `input`, never a copy.
    pub fn parse(input: &'a [u8], policy: &Policy) -> Result<Frame<'a>, Error> {
        if policy.max_payload > MAX_PAYLOAD {
            return Err(Error::InvalidPolicy);
        }
        if input.len() < HEADER_LEN {
            return Err(Error::Malformed(Reject::InputTooShortForHeader));
        }
        if &input[0..4] != MAGIC {
            return Err(Error::Malformed(Reject::BadMagic));
        }
        if u16::from_le_bytes([input[4], input[5]]) != VERSION {
            return Err(Error::Malformed(Reject::UnsupportedVersion));
        }
        let op = match Op::from_u8(input[6]) {
            Some(o) => o,
            None => return Err(Error::Malformed(Reject::UnsupportedOp)),
        };
        if input[7] != 0 {
            return Err(Error::Malformed(Reject::ReservedNonZero));
        }
        let correlation_id = u64::from_le_bytes(input[8..16].try_into().unwrap());
        if correlation_id == 0 {
            return Err(Error::Malformed(Reject::ZeroCorrelationId));
        }
        let boot_id: [u8; 16] = input[16..32].try_into().unwrap();
        if is_zero_boot(&boot_id) {
            return Err(Error::Malformed(Reject::ZeroBootId));
        }
        let declared = u32::from_le_bytes(input[32..36].try_into().unwrap());
        if declared > MAX_PAYLOAD {
            return Err(Error::Malformed(Reject::DeclaredLengthOverHardCap));
        }
        if declared > policy.max_payload {
            return Err(Error::Malformed(Reject::DeclaredLengthOverPolicyCap));
        }
        let payload_len = usize::try_from(declared)
            .map_err(|_| Error::Malformed(Reject::DeclaredLengthOverHardCap))?;
        let total = match HEADER_LEN.checked_add(payload_len) {
            Some(t) => t,
            None => return Err(Error::Malformed(Reject::TruncatedPayload)),
        };
        if input.len() < total {
            return Err(Error::Malformed(Reject::TruncatedPayload));
        }
        if input.len() > total {
            return Err(Error::Malformed(Reject::TrailingBytes));
        }
        Ok(Frame {
            op,
            correlation_id,
            boot_id,
            payload: &input[HEADER_LEN..total],
        })
    }

    /// Encode into caller-provided output. All metadata and the payload length
    /// are validated first, so a rejection leaves `output` byte-for-byte
    /// unchanged. Returns the number of bytes written.
    pub fn encode(
        op: Op,
        correlation_id: u64,
        boot_id: &[u8; 16],
        payload: &[u8],
        output: &mut [u8],
        policy: &Policy,
    ) -> Result<usize, Error> {
        if policy.max_payload > MAX_PAYLOAD {
            return Err(Error::InvalidPolicy);
        }
        if correlation_id == 0 {
            return Err(Error::Malformed(Reject::ZeroCorrelationId));
        }
        if is_zero_boot(boot_id) {
            return Err(Error::Malformed(Reject::ZeroBootId));
        }
        let declared = match u32::try_from(payload.len()) {
            Ok(n) => n,
            Err(_) => return Err(Error::Malformed(Reject::DeclaredLengthOverHardCap)),
        };
        if declared > MAX_PAYLOAD {
            return Err(Error::Malformed(Reject::DeclaredLengthOverHardCap));
        }
        if declared > policy.max_payload {
            return Err(Error::Malformed(Reject::DeclaredLengthOverPolicyCap));
        }
        let total = match HEADER_LEN.checked_add(payload.len()) {
            Some(t) => t,
            None => return Err(Error::Malformed(Reject::DeclaredLengthOverHardCap)),
        };
        if output.len() < total {
            return Err(Error::OutputTooSmall {
                required: total,
                capacity: output.len(),
            });
        }
        output[0..4].copy_from_slice(MAGIC);
        output[4..6].copy_from_slice(&VERSION.to_le_bytes());
        output[6] = op as u8;
        output[7] = 0;
        output[8..16].copy_from_slice(&correlation_id.to_le_bytes());
        output[16..32].copy_from_slice(boot_id);
        output[32..36].copy_from_slice(&declared.to_le_bytes());
        output[HEADER_LEN..total].copy_from_slice(payload);
        Ok(total)
    }
}

/// Honest reporting only: this crate parses/encodes framed bytes and has no
/// live transport. Any attempt to name a transport yields an explicit error;
/// there is no connect, send, ACK or success path here.
pub fn transport_report(name: &str) -> Error {
    Error::UnsupportedTransport(match name {
        "usb" | "adb" | "tcp" | "uart" => "no live transport implemented",
        _ => "unknown transport, none implemented",
    })
}
