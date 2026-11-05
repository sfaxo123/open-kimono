//! Borrowed input-BATCH body codec for the accepted control-envelope v1.
//!
//! This module encodes and decodes ONE input-body version 1 payload carried
//! by an already-validated [`Frame`] with `op == Op::Input`. It defines
//! bounded wire bytes ONLY: it performs no injection and makes no claim that
//! any live device accepts these events. Key codes are proposed raw
//! identifiers, not proven OS keymap mappings.
//!
//! Exact input-body v1 layout (little-endian), fixed 5-byte header plus
//! exactly `count` fixed-size 16-byte records — the byte layout of every
//! record kind is documented in [`layout`]:
//! - bytes 0..2    u16 event count, must be 1..=64 (`MAX_BATCH`);
//! - bytes 2..5    three fixed zero padding bytes (must decode as zero);
//! - then exactly `count` records of 16 bytes each.
//!
//! Event kinds: tap, swipe, press, move, release, key, text — each with a
//! fixed 16-byte record (see [`layout`] for the byte-by-byte layout).
//!
//! Coordinate bounds are SDM845-enough: x in 0..=1080 (`MAX_X`), y in
//! 0..=2280 (`MAX_Y`). Out-of-range coordinates, an oversize batch count,
//! unknown tags/phases, non-UTF-8 text and any length mismatch are rejected
//! BEFORE any allocation — decode borrows the caller's buffer and encode
//! writes only into caller-provided output after a complete preflight, so a
//! rejected encode leaves the output byte-for-byte unchanged.
//!
//! The decoder additionally exposes [`InputBatch::held_set`]: the set of
//! currently-pressed inputs implied by the event sequence alone (press adds,
//! release removes, tap is press+release → net empty, swipe is press+move+
//! release → net empty). The held set NEVER depends on a session id,
//! correlation id or any other envelope value — only on the ordered events.
//! It is shaped exactly like [`crate::session::ReleasePlan`] so the session
//! policy can consume it via `register_held_inputs`.
//!
//! Module-local [`InputError`] keeps body rejections in the input
//! namespace; envelope failures map through `Envelope(crate::Error)`.

use crate::session::{MAX_KEY_WORDS, POINTER_SLOTS};
use crate::{Error, Frame, Op, Policy};
use core::fmt;

mod error;
mod layout;
mod scan;

pub use error::{InputError, InputReject};
pub use layout::{
    InputEvent, KeyEvent, KeyPhase, PointerAction, PointerEvent, SwipeEvent, TapEvent, TextEvent,
    INPUT_BODY_VERSION, INPUT_HEADER_LEN, MAX_BATCH, MAX_KEY_CODE, MAX_POINTER_SLOT,
    MAX_TEXT_BYTES, MAX_X, MAX_Y, RECORD_EVENT_LEN, SWIPE_Y1_QUANTUM,
};
pub use scan::{EventCursor, HeldSet, WireCursor};

/// A fully validated input batch bound to the envelope it arrived in: the
/// borrowed batch plus the validated correlation id and boot id of that
/// envelope. A bare [`InputBatch`] from [`InputBatch::decode`] carries no
/// envelope binding at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FramedInput<'a> {
    batch: InputBatch<'a>,
    correlation_id: u64,
    boot_id: [u8; 16],
}

impl<'a> FramedInput<'a> {
    pub fn batch(&self) -> &InputBatch<'a> {
        &self.batch
    }
    /// The envelope's validated nonzero correlation id, distinct from every
    /// body field. It NEVER influences the batch contents or held set.
    pub fn correlation_id(&self) -> u64 {
        self.correlation_id
    }
    /// The envelope's validated nonzero 16-byte boot id.
    pub fn boot_id(&self) -> &[u8; 16] {
        &self.boot_id
    }
}

/// One fully validated input body. Header fields are private and only
/// reachable as already-validated values; events are exposed ONLY through
/// the borrowing [`InputBatch::events`] iterator. Validation of every event
/// completes in [`InputBatch::decode`] before this type can exist — an
/// invalid batch is never constructible — and no allocation ever happens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputBatch<'a> {
    count: u16,
    body: &'a [u8],
}

impl<'a> InputBatch<'a> {
    /// Decode an input body from an already-parsed envelope, keeping the
    /// envelope's correlation id and boot id bound to the batch. The frame
    /// must carry `op == Op::Input`.
    pub fn from_frame(frame: &Frame<'a>) -> Result<FramedInput<'a>, InputError> {
        if frame.op() != Op::Input {
            return Err(InputError::Body(InputReject::WrongOp));
        }
        let batch = Self::decode(frame.payload())?;
        Ok(FramedInput {
            batch,
            correlation_id: frame.correlation_id(),
            boot_id: *frame.boot_id(),
        })
    }

    /// Decode a bare input body (the payload bytes of an `Input` frame).
    /// The batch count is checked against `MAX_BATCH` and the exact body
    /// length against the count BEFORE any per-event work; every record's
    /// framing, tag, phase, coordinate and text bytes are validated in the
    /// same single scan. Nothing is allocated; text borrows the input.
    pub fn decode(body: &'a [u8]) -> Result<InputBatch<'a>, InputError> {
        if body.len() < INPUT_HEADER_LEN {
            return Err(InputError::Body(InputReject::BodyTooShortForHeader));
        }
        let count = u16::from_le_bytes(body[0..2].try_into().unwrap());
        if count < 1 || count > MAX_BATCH as u16 {
            return Err(InputError::Body(InputReject::BatchCountOutOfRange));
        }
        if body[2] != 0 || body[3] != 0 || body[4] != 0 {
            return Err(InputError::Body(InputReject::ReservedNonZero));
        }
        // Exact expected length from the count, checked BEFORE any event
        // scan: truncation and trailing bytes are both rejections.
        let expected = match usize::from(count).checked_mul(RECORD_EVENT_LEN) {
            Some(n) => match INPUT_HEADER_LEN.checked_add(n) {
                Some(t) => t,
                None => return Err(InputError::Body(InputReject::BodyLengthMismatch)),
            },
            None => return Err(InputError::Body(InputReject::BodyLengthMismatch)),
        };
        if body.len() != expected {
            return Err(InputError::Body(InputReject::BodyLengthMismatch));
        }
        // Validate EVERY record before constructing the batch.
        for i in 0..usize::from(count) {
            scan::check_record(body, INPUT_HEADER_LEN + i * RECORD_EVENT_LEN)
                .map_err(InputError::Body)?;
        }
        Ok(InputBatch { count, body })
    }

    /// The exact event count, validated at decode time.
    pub fn event_count(&self) -> usize {
        usize::from(self.count)
    }

    /// Borrowing iterator over the batch's events. Decode already validated
    /// every record, so the iterator never fails and never allocates. A
    /// decoded swipe record expands to its injection sequence (press,
    /// axis-aligned moves, release), so the iterator may yield MORE events
    /// than the wire record count for swipe records — [`InputBatch::
    /// event_count`] reports the WIRE record count.
    pub fn events(&self) -> EventCursor<'a> {
        EventCursor::new(self.body, self.count)
    }

    /// Borrowing iterator over the WIRE records, one per record: a swipe
    /// yields its compact [`SwipeEvent`] value, unexpanded. Use this when
    /// the exact record sequence matters; [`InputBatch::events`] yields the
    /// injection sequence instead.
    pub fn wire_events(&self) -> WireCursor<'a> {
        WireCursor::new(self.body, self.count)
    }

    /// The HELD-INPUT SET implied by this batch's event sequence alone:
    /// press adds, release removes, tap is press+release (net empty),
    /// swipe is press+move+release (net empty). The result depends ONLY on
    /// the ordered events — never on a session id, correlation id, boot id
    /// or any other envelope value — and is shaped exactly like
    /// [`crate::session::ReleasePlan`] for `register_held_inputs`.
    /// The fold runs over the WIRE records; expansion never changes the
    /// held-set result, and no id is ever consulted.
    pub fn held_set(&self) -> HeldSet {
        let mut held = HeldSet {
            keys: [0u64; MAX_KEY_WORDS],
            pointers: [false; POINTER_SLOTS],
        };
        for f in scan::wire_fields(self.body, self.count) {
            scan::apply_fields_to_held_set(&mut held, f);
        }
        held
    }

    /// Encode a caller slice of typed events into the input-body layout in
    /// caller-provided `output`. The COMPLETE body is preflighted first —
    /// batch count, every coordinate, slot, phase, key code, text length
    /// and the output capacity — so a rejected encode leaves `output`
    /// byte-for-byte unchanged. A [`SwipeEvent`] encodes as exactly ONE
    /// compact record. Returns the number of bytes written.
    pub fn encode_body(events: &[InputEvent<'_>], output: &mut [u8]) -> Result<usize, InputError> {
        Self::encode_wire(events, output)
    }

    /// Encode the WIRE sequence (one record per event, swipe kept compact)
    /// — identical to [`InputBatch::encode_body`]; kept as a named alias so
    /// callers can state intent when pairing with [`InputBatch::wire_events`].
    pub fn encode_wire(events: &[InputEvent<'_>], output: &mut [u8]) -> Result<usize, InputError> {
        let count = match u16::try_from(events.len()) {
            Ok(n) if (1..=MAX_BATCH as u16).contains(&n) => n,
            _ => return Err(InputError::Body(InputReject::BatchCountOutOfRange)),
        };
        for ev in events {
            scan::preflight_event(ev)?;
        }
        let total = match INPUT_HEADER_LEN.checked_add(usize::from(count) * RECORD_EVENT_LEN) {
            Some(t) => t,
            None => return Err(InputError::Body(InputReject::BodyLengthMismatch)),
        };
        if total > MAX_BODY_BYTES {
            return Err(InputError::Body(InputReject::BodyLengthMismatch));
        }
        if output.len() < total {
            return Err(InputError::Envelope(Error::OutputTooSmall {
                required: total,
                capacity: output.len(),
            }));
        }
        // All validation passed; only now write.
        output[0..2].copy_from_slice(&count.to_le_bytes());
        output[2] = 0;
        output[3] = 0;
        output[4] = 0;
        let mut pos = INPUT_HEADER_LEN;
        for ev in events {
            scan::write_event(ev, output, &mut pos);
        }
        Ok(total)
    }

    /// Re-encode this already-decoded batch into `output`. The batch was
    /// fully validated at decode, so the re-encode is exact and byte-for-byte
    /// identical to the source body. Returns bytes written.
    pub fn encode_into(&self, output: &mut [u8]) -> Result<usize, InputError> {
        let total = self.body.len();
        if output.len() < total {
            return Err(InputError::Envelope(Error::OutputTooSmall {
                required: total,
                capacity: output.len(),
            }));
        }
        output[..total].copy_from_slice(&self.body[..total]);
        Ok(total)
    }

    /// Build the full envelope (36-byte frame header + this body) into
    /// caller-provided `output`, correlated with `correlation_id` and bound
    /// to the envelope `boot_id`. All validation happens before any write.
    /// Returns total bytes written.
    pub fn encode_envelope(
        &self,
        correlation_id: u64,
        boot_id: &[u8; 16],
        output: &mut [u8],
        policy: &Policy,
    ) -> Result<usize, InputError> {
        let mut body = [0u8; MAX_BODY_BYTES];
        let body_len = self.encode_into(&mut body)?;
        Frame::encode(
            Op::Input,
            correlation_id,
            boot_id,
            &body[..body_len],
            output,
            policy,
        )
        .map_err(InputError::Envelope)
    }
}

/// Hard cap on one whole input body: header + `MAX_BATCH` fixed records.
const MAX_BODY_BYTES: usize = INPUT_HEADER_LEN + MAX_BATCH * RECORD_EVENT_LEN;

impl fmt::Display for InputBatch<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "input v{} events {}", INPUT_BODY_VERSION, self.count)
    }
}
