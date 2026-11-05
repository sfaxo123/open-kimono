//! Borrowed proposed COMMAND body codec for the control-envelope v1.
//!
//! This module family decodes and encodes ONE command-body version 1 payload
//! carried by an already-validated [`Frame`] with `op == Op::Command`. It is
//! a PROPOSED body schema for this partial 0.2.1 slice. It never executes,
//! schedules, authorizes, replays or cancels anything: it only turns bytes
//! into a validated borrowed description and back. There is no phone, no
//! transport and no proof of execution anywhere in these modules.
//!
//! Exact request-body v1 layout (little-endian), variable length — see the
//! [`request`] submodule:
//! - byte 0        tag, must be 1;
//! - bytes 1..5    u32 timeout_ms, must be 1..=60000;
//! - bytes 5..7    u16 argc, must be 1..=32;
//! - then argc repetitions of: u16 arg_length (must be 0..=4096) followed by
//!   that many argument BYTES. argv[0] must be nonempty; later args may be
//!   empty; no argument may contain a NUL byte;
//! - the whole body must be at most 65536 bytes, with no trailing bytes.
//!
//! Arguments are opaque bytes: they may be non-UTF-8 and are preserved
//! losslessly. This codec performs NO shell splitting, quoting or
//! sanitization — argv is exactly what the wire says it is.
//!
//! The result-body half (tag 2) is NOT implemented in this slice. There is
//! no `result` submodule here: nothing in this crate encodes or decodes a
//! result body. A result tag arriving on the wire is rejected by the request
//! decoder as `CommandReject::UnexpectedTag`. The reserved constants and
//! status names in [`common`] are a frozen naming contract for that future
//! half, not an active codec.
//!
//! Parsing never allocates: arguments and streams borrow the caller's
//! buffer. A decoded [`request::CommandRequest`] stores the whole validated
//! body and hands out arguments through a zero-allocation borrowed iterator
//! that re-walks it; one built via `new` iterates the caller's own argv
//! slice. Encoding writes only into caller-provided output and validates
//! every argument, length, sum and capacity BEFORE any write, so a rejected
//! encode leaves the output byte-for-byte unchanged.
//!
//! Body errors are module-local ([`common::CommandError`]); envelope-level
//! failures map through `Envelope(crate::Error)` and never change the core
//! type.

pub mod common;
pub mod request;
pub mod result_contract;

pub use common::{
    reserved, CommandError, CommandReject, ResultReject, Status, COMMAND_BODY_VERSION, MAX_ARGC,
    MAX_ARG_BYTES, MAX_COMMAND_BODY, MAX_TIMEOUT_MS, MIN_ARGC, MIN_TIMEOUT_MS, REQUEST_HEADER_LEN,
};
pub use request::{Args, CommandRequest, FramedRequest};
