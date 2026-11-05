//! Borrowed COMMAND request-body v1: tag 1, timeout_ms, argc, length-prefixed
//! opaque argv. Decoding validates every declared length, running sum and
//! NUL before handing out any borrow, and allocates nothing; the argument
//! iterator re-walks the validated body instead of materializing an argv
//! table. Encoding validates everything (or re-uses an already-validated
//! decode) before writing a single byte.

use super::common::{
    CommandError, CommandReject, MAX_ARGC, MAX_ARG_BYTES, MAX_COMMAND_BODY, MAX_TIMEOUT_MS,
    MIN_ARGC, MIN_TIMEOUT_MS, REQUEST_HEADER_LEN, TAG_REQUEST, TAG_RESULT,
};
use crate::{Error, Frame, Op, Policy};
use core::fmt;

/// Where the arguments of a [`CommandRequest`] come from: the caller's own
/// argv table (constructed) or a validated borrowed body to re-walk
/// (decoded). Both paths hand out borrowed slices and allocate nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArgsSrc<'a> {
    Table(&'a [&'a [u8]]),
    Body(&'a [u8]),
}

/// A fully validated borrowed command request. Fields are private and only
/// reachable through validated accessors; every argument borrows the parsed
/// input buffer or the caller's argv table. Constructing one via
/// [`CommandRequest::new`] performs the same validation as decoding, so an
/// invalid request cannot exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandRequest<'a> {
    timeout_ms: u32,
    src: ArgsSrc<'a>,
}

impl<'a> CommandRequest<'a> {
    /// Validate a caller-supplied argv slice (no copying, no shell
    /// semantics). Every rule of the wire layout applies.
    pub fn new(timeout_ms: u32, args: &'a [&'a [u8]]) -> Result<Self, CommandError> {
        if !(MIN_TIMEOUT_MS..=MAX_TIMEOUT_MS).contains(&timeout_ms) {
            return Err(CommandError::Body(CommandReject::TimeoutOutOfRange));
        }
        if args.len() < MIN_ARGC || args.len() > MAX_ARGC {
            return Err(CommandError::Body(CommandReject::ArgcOutOfRange));
        }
        // Hard-cap sum first, mirroring decode(): the explicit maximum is
        // enforced before per-arg caps so an over-total argv table is
        // rejected as BodyOverHardCap, not a per-field error.
        request_total(args)?;
        check_args(args)?;
        Ok(Self {
            timeout_ms,
            src: ArgsSrc::Table(args),
        })
    }

    /// Decode a request body bound to its envelope: keeps the frame's
    /// validated correlation id and boot id. The frame must carry
    /// `op == Op::Command`.
    pub fn from_frame(frame: &Frame<'a>) -> Result<FramedRequest<'a>, CommandError> {
        if frame.op() != Op::Command {
            return Err(CommandError::Body(CommandReject::WrongOp));
        }
        let request = Self::decode(frame.payload())?;
        Ok(FramedRequest {
            request,
            correlation_id: frame.correlation_id(),
            boot_id: *frame.boot_id(),
        })
    }

    /// Decode a bare request body (payload bytes of a `Command` frame). Not
    /// bound to any envelope. Every declared length, running sum, NUL and
    /// the exact body end are checked BEFORE the borrow is handed out, and
    /// nothing is allocated.
    pub fn decode(body: &'a [u8]) -> Result<Self, CommandError> {
        if body.len() > MAX_COMMAND_BODY {
            return Err(CommandError::Body(CommandReject::BodyOverHardCap));
        }
        if body.len() < REQUEST_HEADER_LEN {
            return Err(CommandError::Body(CommandReject::BodyTooShortForHeader));
        }
        if body[0] != TAG_REQUEST {
            // A result tag (2) or any other byte is a typed rejection: this
            // slice has no result codec, and the decoder never guesses which
            // half the caller meant.
            debug_assert!(TAG_RESULT == 2);
            return Err(CommandError::Body(CommandReject::UnexpectedTag));
        }
        let timeout_ms = u32::from_le_bytes(body[1..5].try_into().unwrap());
        if !(MIN_TIMEOUT_MS..=MAX_TIMEOUT_MS).contains(&timeout_ms) {
            return Err(CommandError::Body(CommandReject::TimeoutOutOfRange));
        }
        let argc = usize::from(u16::from_le_bytes(body[5..7].try_into().unwrap()));
        if argc < MIN_ARGC || argc > MAX_ARGC {
            return Err(CommandError::Body(CommandReject::ArgcOutOfRange));
        }
        // Single pass: walk the argument table, checking every declared
        // length, running total and NUL BEFORE borrowing anything. The scan
        // must end exactly at the body end; trailing bytes are rejected.
        let mut pos = REQUEST_HEADER_LEN;
        for i in 0..argc {
            if pos + 2 > body.len() {
                return Err(CommandError::Body(CommandReject::BodyLengthMismatch));
            }
            let len = usize::from(u16::from_le_bytes(body[pos..pos + 2].try_into().unwrap()));
            if len > MAX_ARG_BYTES {
                return Err(CommandError::Body(CommandReject::ArgLenOverCap));
            }
            if i == 0 && len == 0 {
                return Err(CommandError::Body(CommandReject::EmptyArgv0));
            }
            let next = match pos.checked_add(2).and_then(|p| p.checked_add(len)) {
                Some(n) => n,
                None => return Err(CommandError::Body(CommandReject::BodyLengthMismatch)),
            };
            if next > body.len() {
                return Err(CommandError::Body(CommandReject::BodyLengthMismatch));
            }
            if body[pos + 2..next].contains(&0) {
                return Err(CommandError::Body(CommandReject::ArgumentContainsNul));
            }
            pos = next;
        }
        if pos != body.len() {
            return Err(CommandError::Body(CommandReject::BodyLengthMismatch));
        }
        Ok(Self {
            timeout_ms,
            src: ArgsSrc::Body(body),
        })
    }

    pub fn timeout_ms(&self) -> u32 {
        self.timeout_ms
    }

    /// The exact argv count, validated at construction/decode time.
    pub fn argc(&self) -> usize {
        match self.src {
            ArgsSrc::Table(t) => t.len(),
            // The body was fully validated at decode; re-reading the u16 is
            // exact and cannot fail.
            ArgsSrc::Body(b) => usize::from(u16::from_le_bytes(b[5..7].try_into().unwrap())),
        }
    }

    /// Iterate the arguments as borrowed slices. Zero allocation: for a
    /// decoded request this re-walks the validated body; for a constructed
    /// one it walks the caller's own argv table.
    pub fn args(&self) -> Args<'_, 'a> {
        Args {
            src: &self.src,
            pos: REQUEST_HEADER_LEN,
            index: 0,
            argc: self.argc(),
        }
    }

    /// Encode the request body into caller-provided `output`. Every argument
    /// and length is validated BEFORE any write, so a rejected encode leaves
    /// `output` byte-for-byte unchanged. Returns bytes written.
    pub fn encode_body(&self, output: &mut [u8]) -> Result<usize, CommandError> {
        let total: usize = match &self.src {
            ArgsSrc::Table(table) => {
                if !(MIN_TIMEOUT_MS..=MAX_TIMEOUT_MS).contains(&self.timeout_ms) {
                    return Err(CommandError::Body(CommandReject::TimeoutOutOfRange));
                }
                if table.len() < MIN_ARGC || table.len() > MAX_ARGC {
                    return Err(CommandError::Body(CommandReject::ArgcOutOfRange));
                }
                request_total(table)?;
                check_args(table)?;
                self.encoded_body_len()?
            }
            // A decoded body is already fully validated and exactly sized;
            // its length IS the encoded length.
            ArgsSrc::Body(body) => body.len(),
        };
        if output.len() < total {
            return Err(CommandError::Envelope(Error::OutputTooSmall {
                required: total,
                capacity: output.len(),
            }));
        }
        // All validation passed; only now write.
        output[0] = TAG_REQUEST;
        output[1..5].copy_from_slice(&self.timeout_ms.to_le_bytes());
        let argc = u16::try_from(self.argc())
            .map_err(|_| CommandError::Body(CommandReject::ArgcOutOfRange))?;
        output[5..7].copy_from_slice(&argc.to_le_bytes());
        match &self.src {
            ArgsSrc::Table(table) => {
                let mut pos = REQUEST_HEADER_LEN;
                for arg in table.iter() {
                    let len = u16::try_from(arg.len())
                        .map_err(|_| CommandError::Body(CommandReject::ArgLenOverCap))?;
                    output[pos..pos + 2].copy_from_slice(&len.to_le_bytes());
                    output[pos + 2..pos + 2 + arg.len()].copy_from_slice(arg);
                    pos += 2 + arg.len();
                }
            }
            ArgsSrc::Body(body) => {
                output[REQUEST_HEADER_LEN..total].copy_from_slice(&body[REQUEST_HEADER_LEN..]);
            }
        }
        Ok(total)
    }

    /// Build the full envelope (frame header + request body) into
    /// caller-provided `output`. All validation precedes any write.
    pub fn encode_envelope(
        &self,
        correlation_id: u64,
        boot_id: &[u8; 16],
        output: &mut [u8],
        policy: &Policy,
    ) -> Result<usize, CommandError> {
        // The body is staged in a fixed on-stack buffer of exactly the hard
        // cap. Oversize is rejected by `encode_body`'s checked sums BEFORE
        // this staging and before any allocation or write.
        let mut body = [0u8; MAX_COMMAND_BODY];
        let body_len = self.encode_body(&mut body)?;
        Frame::encode(
            Op::Command,
            correlation_id,
            boot_id,
            &body[..body_len],
            output,
            policy,
        )
        .map_err(CommandError::Envelope)
    }

    /// The exact encoded request-body length for this request, computed with
    /// checked sums. Returns `BodyOverHardCap` when the body would exceed the
    /// 65536-byte explicit maximum — the caller learns the bound is exceeded
    /// before any allocation or write is attempted.
    pub fn encoded_body_len(&self) -> Result<usize, CommandError> {
        match &self.src {
            ArgsSrc::Table(table) => request_total(table),
            ArgsSrc::Body(body) => Ok(body.len()),
        }
    }

    /// Decode a command-request frame in one call: parse the envelope under
    /// `policy` (whose declared payload length is checked against the caps
    /// before any size arithmetic or allocation) and decode the request body
    /// from the borrowed payload. Envelope failures surface as
    /// `CommandError::Envelope`; a non-`Command` op or bad body as
    /// `CommandError::Body`.
    pub fn decode_envelope<'b>(
        input: &'b [u8],
        policy: &Policy,
    ) -> Result<FramedRequest<'b>, CommandError> {
        Frame::parse(input, policy)
            .map_err(CommandError::Envelope)
            .and_then(|f| CommandRequest::<'_>::from_frame(&f))
    }
}

/// Validate argv rules shared by `new` and `encode_body`.
fn check_args(args: &[&[u8]]) -> Result<(), CommandError> {
    if args[0].is_empty() {
        return Err(CommandError::Body(CommandReject::EmptyArgv0));
    }
    for arg in args {
        if arg.len() > MAX_ARG_BYTES {
            return Err(CommandError::Body(CommandReject::ArgLenOverCap));
        }
        if arg.contains(&0) {
            return Err(CommandError::Body(CommandReject::ArgumentContainsNul));
        }
    }
    Ok(())
}

/// Exact request-body length for argv, with checked sums; also enforces the
/// 65536 hard cap.
fn request_total(args: &[&[u8]]) -> Result<usize, CommandError> {
    let mut total = REQUEST_HEADER_LEN;
    for arg in args {
        total = match total.checked_add(2).and_then(|t| t.checked_add(arg.len())) {
            Some(n) => n,
            None => return Err(CommandError::Body(CommandReject::BodyLengthMismatch)),
        };
    }
    if total > MAX_COMMAND_BODY {
        return Err(CommandError::Body(CommandReject::BodyOverHardCap));
    }
    Ok(total)
}

/// Borrowed iterator over the arguments of a [`CommandRequest`]. Yields
/// borrowed slices; allocates nothing.
#[derive(Debug, Clone, Copy)]
pub struct Args<'s, 'a> {
    src: &'s ArgsSrc<'a>,
    pos: usize,
    index: usize,
    argc: usize,
}

impl<'s, 'a> Iterator for Args<'s, 'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<&'a [u8]> {
        if self.index >= self.argc {
            return None;
        }
        match self.src {
            ArgsSrc::Table(table) => {
                let item = table[self.index];
                self.index += 1;
                Some(item)
            }
            ArgsSrc::Body(body) => {
                let len = usize::from(u16::from_le_bytes(
                    body[self.pos..self.pos + 2].try_into().unwrap(),
                ));
                let arg = &body[self.pos + 2..self.pos + 2 + len];
                self.pos += 2 + len;
                self.index += 1;
                Some(arg)
            }
        }
    }
}

/// A decoded [`CommandRequest`] bound to the envelope it arrived in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FramedRequest<'a> {
    request: CommandRequest<'a>,
    correlation_id: u64,
    boot_id: [u8; 16],
}

impl<'a> FramedRequest<'a> {
    pub fn request(&self) -> &CommandRequest<'a> {
        &self.request
    }
    /// The envelope's validated nonzero correlation id, distinct from any
    /// body field.
    pub fn correlation_id(&self) -> u64 {
        self.correlation_id
    }
    /// The envelope's validated nonzero 16-byte boot id.
    pub fn boot_id(&self) -> &[u8; 16] {
        &self.boot_id
    }
}

impl fmt::Display for CommandRequest<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "command v{} timeout {} ms, {} args",
            super::common::COMMAND_BODY_VERSION,
            self.timeout_ms,
            self.argc()
        )
    }
}
