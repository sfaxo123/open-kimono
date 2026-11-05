//! Record validation, the borrowing event cursor, encode preflight/write,
//! and the held-set fold over a validated input body.
//!
//! Every function here checks bounds BEFORE any slice read and uses checked
//! conversions only. Records are fixed-size at 16 bytes, so record framing
//! is pure arithmetic — a body whose length is not exactly header + count ×
//! 16 is rejected in the header check, never reinterpreted. The same
//! validation backs decode and the [`EventCursor`] iterator, so what decode
//! accepted is exactly what the iterator replays. A swipe record expands
//! into its injection sequence (press, axis-aligned moves, release); the
//! cursor emits that expansion from a fixed-capacity slot queue — no
//! allocation anywhere.

use super::error::{InputError, InputReject};
use super::layout::{
    InputEvent, KeyEvent, KeyPhase, PointerAction, PointerEvent, SwipeEvent, TapEvent, TextEvent,
    MAX_KEY_CODE, MAX_POINTER_SLOT, MAX_TEXT_BYTES, MAX_X, MAX_Y, RECORD_EVENT_LEN,
    SWIPE_Y1_QUANTUM, TAG_KEY, TAG_MOVE, TAG_PRESS, TAG_RELEASE, TAG_SWIPE, TAG_TAP, TAG_TEXT,
};
use crate::session::{MAX_KEY_WORDS, POINTER_SLOTS};

/// The held-input set a batch implies: a key bitset for codes 0..=767 and
/// ten pointer slots — shaped exactly like [`crate::session::ReleasePlan`]
/// so the session policy consumes it directly via `register_held_inputs`.
/// It is derived ONLY from the ordered event sequence, never from any
/// session, correlation or boot id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HeldSet {
    pub keys: [u64; MAX_KEY_WORDS],
    pub pointers: [bool; POINTER_SLOTS],
}

impl HeldSet {
    pub fn is_empty(&self) -> bool {
        self.keys.iter().all(|&w| w == 0) && self.pointers.iter().all(|&p| !p)
    }
    pub fn key_down(&self, code: u16) -> bool {
        let word = usize::from(code) / 64;
        let bit = usize::from(code) % 64;
        if word >= MAX_KEY_WORDS {
            return false;
        }
        self.keys[word] & (1u64 << bit) != 0
    }
    pub fn pointer_down(&self, slot: u8) -> bool {
        usize::from(slot) < POINTER_SLOTS && self.pointers[usize::from(slot)]
    }
}

/// Validate one record at `pos` (which the caller proved is in-bounds).
/// Every field is range-checked; reserved bytes must be zero. No allocation,
/// no slice read outside the record.
pub(crate) fn check_record(body: &[u8], pos: usize) -> Result<(), InputReject> {
    let r = &body[pos..pos + RECORD_EVENT_LEN];
    match r[0] {
        TAG_TAP | TAG_PRESS | TAG_MOVE | TAG_RELEASE => {
            let action = match r[1] {
                0 => TAG_TAP,
                1 => TAG_PRESS,
                2 => TAG_MOVE,
                3 => TAG_RELEASE,
                _ => return Err(InputReject::InvalidPointerAction),
            };
            // The action byte and the tag must agree: a record labeled press
            // with a move action byte is rejected, never reinterpreted.
            if action != r[0] {
                return Err(InputReject::InvalidPointerAction);
            }
            if r[2] > MAX_POINTER_SLOT {
                return Err(InputReject::PointerSlotOutOfRange);
            }
            if u32::from_le_bytes(r[3..7].try_into().unwrap()) > MAX_X {
                return Err(InputReject::PointerXOutOfRange);
            }
            if u32::from_le_bytes(r[7..11].try_into().unwrap()) > MAX_Y {
                return Err(InputReject::PointerYOutOfRange);
            }
            if r[11..16].iter().any(|&b| b != 0) {
                return Err(InputReject::RecordReservedNonZero);
            }
        }
        TAG_SWIPE => {
            // Compact wire form of press + axis-aligned moves + release;
            // endpoints only.
            if r[1] != 0 {
                return Err(InputReject::InvalidPointerAction);
            }
            if r[2] > MAX_POINTER_SLOT {
                return Err(InputReject::PointerSlotOutOfRange);
            }
            if u32::from_le_bytes(r[3..7].try_into().unwrap()) > MAX_X {
                return Err(InputReject::PointerXOutOfRange);
            }
            if u32::from_le_bytes(r[7..11].try_into().unwrap()) > MAX_Y {
                return Err(InputReject::PointerYOutOfRange);
            }
            if u32::from_le_bytes(r[11..15].try_into().unwrap()) > MAX_X {
                return Err(InputReject::PointerXOutOfRange);
            }
            let y1 = u32::from(r[15]) * SWIPE_Y1_QUANTUM;
            if y1 > MAX_Y {
                return Err(InputReject::PointerYOutOfRange);
            }
        }
        TAG_KEY => {
            if KeyPhase::from_u8(r[1]).is_none() {
                return Err(InputReject::InvalidKeyPhase);
            }
            if u16::from_le_bytes(r[3..5].try_into().unwrap()) > MAX_KEY_CODE {
                return Err(InputReject::KeyCodeOutOfRange);
            }
            if r[5..16].iter().any(|&b| b != 0) {
                return Err(InputReject::RecordReservedNonZero);
            }
        }
        TAG_TEXT => {
            if r[1] != 0 {
                return Err(InputReject::InvalidPointerAction);
            }
            let len = usize::from(r[2]);
            if len > MAX_TEXT_BYTES {
                return Err(InputReject::TextLenOverCap);
            }
            // len <= MAX_TEXT_BYTES = 13 guarantees 3 + len <= 16, so the
            // payload always fits the record; the check is kept explicit.
            if 3 + len > RECORD_EVENT_LEN {
                return Err(InputReject::TextLenOverRecord);
            }
            if core::str::from_utf8(&r[3..3 + len]).is_err() {
                return Err(InputReject::TextNotUtf8);
            }
            if r[3 + len..].iter().any(|&b| b != 0) {
                return Err(InputReject::RecordReservedNonZero);
            }
        }
        _ => return Err(InputReject::UnknownEventTag),
    }
    Ok(())
}

/// The record kind and its decoded fields, as a plain struct the cursor
/// turns into typed events at emit time. `text_start`/`text_len` address
/// the payload inside the borrowed body; the cursor re-slices them with the
/// iterator's lifetime — decode already proved this range is valid UTF-8.
#[derive(Clone, Copy)]
pub(crate) struct RecordFields {
    tag: u8,
    action: u8,
    slot: u8,
    x: u32,
    y: u32,
    x1: u32,
    y1: u32,
    text_start: usize,
    text_len: usize,
}

/// Re-read one validated record into its fields. Safe by construction —
/// [`check_record`] accepted this exact bytes.
fn read_fields(body: &[u8], pos: usize) -> Option<RecordFields> {
    let r = body.get(pos..pos + RECORD_EVENT_LEN)?;
    let tag = r[0];
    let mut f = RecordFields {
        tag,
        action: r[1],
        slot: r[2],
        x: 0,
        y: 0,
        x1: 0,
        y1: 0,
        text_start: 0,
        text_len: 0,
    };
    match tag {
        TAG_TAP | TAG_PRESS | TAG_MOVE | TAG_RELEASE => {
            f.x = u32::from_le_bytes(r[3..7].try_into().ok()?);
            f.y = u32::from_le_bytes(r[7..11].try_into().ok()?);
        }
        TAG_SWIPE => {
            f.x = u32::from_le_bytes(r[3..7].try_into().ok()?);
            f.y = u32::from_le_bytes(r[7..11].try_into().ok()?);
            f.x1 = u32::from_le_bytes(r[11..15].try_into().ok()?);
            f.y1 = u32::from(r[15]) * SWIPE_Y1_QUANTUM;
        }
        TAG_KEY => {
            f.x = u32::from(u16::from_le_bytes(r[3..5].try_into().ok()?));
        }
        TAG_TEXT => {
            f.text_len = usize::from(r[2]);
            f.text_start = pos + 3;
        }
        _ => return None,
    }
    Some(f)
}

/// A tiny fixed-capacity queue of pending expanded events. Capacity 4 is
/// the maximum expansion of one wire record (swipe with both axis moves);
/// nothing else expands.
const QUEUE_CAP: usize = 4;

#[derive(Clone, Copy)]
struct Queue {
    items: [Option<RecordFields>; QUEUE_CAP],
    len: usize,
}

impl Queue {
    fn new() -> Self {
        Queue {
            items: [None; QUEUE_CAP],
            len: 0,
        }
    }
    fn is_empty(&self) -> bool {
        self.len == 0
    }
    fn push(&mut self, f: RecordFields) -> bool {
        if self.len >= QUEUE_CAP {
            return false;
        }
        self.items[self.len] = Some(f);
        self.len += 1;
        true
    }
    fn pop(&mut self) -> Option<RecordFields> {
        if self.len == 0 {
            return None;
        }
        let f = self.items[0].take()?;
        for i in 1..self.len {
            self.items[i - 1] = self.items[i].take();
        }
        self.len -= 1;
        Some(f)
    }
}

/// Borrowing cursor over the validated records of an input body. Yields one
/// or more events per wire record (a swipe expands to its injection
/// sequence); every value borrows the original input buffer. Expansion is
/// buffered in a fixed-capacity slot queue — no allocation anywhere.
#[derive(Clone, Copy)]
pub struct EventCursor<'a> {
    body: &'a [u8],
    index: usize,
    count: u16,
    queue: Queue,
}

impl<'a> EventCursor<'a> {
    pub(crate) fn new(body: &'a [u8], count: u16) -> Self {
        EventCursor {
            body,
            index: 0,
            count,
            queue: Queue::new(),
        }
    }

    fn fill_from_record(&mut self) {
        let pos = super::INPUT_HEADER_LEN + self.index * RECORD_EVENT_LEN;
        self.index += 1;
        let f = match read_fields(self.body, pos) {
            Some(f) => f,
            None => return,
        };
        match f.tag {
            TAG_SWIPE => {
                // Expansion: press at (x0,y0), then axis-aligned moves
                // (never a fabricated diagonal path point), then release at
                // (x1,y1).
                let _ = self.queue.push(RecordFields {
                    tag: TAG_PRESS,
                    action: 1,
                    ..f
                });
                if f.x1 != f.x {
                    let _ = self.queue.push(RecordFields {
                        tag: TAG_MOVE,
                        action: 2,
                        x: f.x1,
                        y: f.y,
                        ..f
                    });
                }
                if f.y1 != f.y {
                    let _ = self.queue.push(RecordFields {
                        tag: TAG_MOVE,
                        action: 2,
                        x: f.x1,
                        y: f.y1,
                        ..f
                    });
                }
                let _ = self.queue.push(RecordFields {
                    tag: TAG_RELEASE,
                    action: 3,
                    x: f.x1,
                    y: f.y1,
                    ..f
                });
            }
            _ => {
                let _ = self.queue.push(f);
            }
        }
    }

    fn emit(&self, f: RecordFields) -> Option<InputEvent<'a>> {
        emit_record(self.body, f)
    }
}

/// Turn validated record fields into the typed event, borrowing `body`
/// for text. Shared by both cursors.
fn emit_record<'a>(body: &'a [u8], f: RecordFields) -> Option<InputEvent<'a>> {
    match f.tag {
        TAG_TAP => Some(InputEvent::Tap(TapEvent {
            slot: f.slot,
            x: f.x,
            y: f.y,
        })),
        TAG_PRESS | TAG_MOVE | TAG_RELEASE => Some(InputEvent::Pointer(PointerEvent {
            action: PointerAction::from_u8(f.action)?,
            slot: f.slot,
            x: f.x,
            y: f.y,
        })),
        TAG_SWIPE => Some(InputEvent::Swipe(SwipeEvent {
            slot: f.slot,
            x0: f.x,
            y0: f.y,
            x1: f.x1,
            y1: f.y1,
        })),
        TAG_KEY => Some(InputEvent::Key(KeyEvent {
            code: u16::try_from(f.x).ok()?,
            phase: KeyPhase::from_u8(f.action)?,
        })),
        TAG_TEXT => {
            let bytes = body.get(f.text_start..f.text_start + f.text_len)?;
            let text = core::str::from_utf8(bytes).ok()?;
            Some(InputEvent::Text(TextEvent { text }))
        }
        _ => None,
    }
}

impl<'a> Iterator for EventCursor<'a> {
    type Item = InputEvent<'a>;
    fn next(&mut self) -> Option<InputEvent<'a>> {
        while self.queue.is_empty() {
            if self.index >= usize::from(self.count) {
                return None;
            }
            self.fill_from_record();
        }
        let f = self.queue.pop()?;
        self.emit(f)
    }
}

/// Borrowing cursor over the WIRE records: exactly one event per record,
/// one-to-one with the batch count. A swipe record yields its compact
/// [`SwipeEvent`] value, unexpanded — use this when the exact record
/// sequence matters.
#[derive(Clone, Copy)]
pub struct WireCursor<'a> {
    body: &'a [u8],
    index: usize,
    count: u16,
}

impl<'a> WireCursor<'a> {
    pub(crate) fn new(body: &'a [u8], count: u16) -> Self {
        WireCursor {
            body,
            index: 0,
            count,
        }
    }
}

impl<'a> Iterator for WireCursor<'a> {
    type Item = InputEvent<'a>;
    fn next(&mut self) -> Option<InputEvent<'a>> {
        if self.index >= usize::from(self.count) {
            return None;
        }
        let pos = super::INPUT_HEADER_LEN + self.index * RECORD_EVENT_LEN;
        self.index += 1;
        let f = read_fields(self.body, pos)?;
        emit_record(self.body, f)
    }
}

/// Iterate the validated wire records as raw fields (one per record, no
/// expansion) for the held-set fold. Decode already accepted every record,
/// so `read_fields` cannot fail here; a defensive skip keeps the fold
/// total.
pub(crate) fn wire_fields(body: &[u8], count: u16) -> WireFields<'_> {
    WireFields {
        body,
        index: 0,
        count,
    }
}

pub(crate) struct WireFields<'a> {
    body: &'a [u8],
    index: usize,
    count: u16,
}

impl<'a> Iterator for WireFields<'a> {
    type Item = RecordFields;
    fn next(&mut self) -> Option<RecordFields> {
        while self.index < usize::from(self.count) {
            let pos = super::INPUT_HEADER_LEN + self.index * RECORD_EVENT_LEN;
            self.index += 1;
            if let Some(f) = read_fields(self.body, pos) {
                return Some(f);
            }
        }
        None
    }
}

/// Fold one wire record's fields into the held set. Same rules as
/// [`apply_to_held_set`], expressed directly on validated fields so the
/// fold never depends on event expansion.
pub(crate) fn apply_fields_to_held_set(held: &mut HeldSet, f: RecordFields) {
    match f.tag {
        TAG_TAP => {
            let slot = usize::from(f.slot);
            if slot < POINTER_SLOTS {
                held.pointers[slot] = true;
                held.pointers[slot] = false;
            }
        }
        TAG_PRESS => {
            let slot = usize::from(f.slot);
            if slot < POINTER_SLOTS {
                held.pointers[slot] = true;
            }
        }
        TAG_RELEASE => {
            let slot = usize::from(f.slot);
            if slot < POINTER_SLOTS {
                held.pointers[slot] = false;
            }
        }
        TAG_SWIPE => {
            // press + moves + release: net empty by construction.
        }
        TAG_KEY => {
            let code = usize::try_from(f.x).unwrap_or(usize::MAX);
            let word = code / 64;
            let bit = code % 64;
            if word < MAX_KEY_WORDS {
                match f.action {
                    0 => held.keys[word] |= 1u64 << bit,
                    1 => held.keys[word] &= !(1u64 << bit),
                    _ => {}
                }
            }
        }
        // TAG_MOVE and TAG_TEXT leave the set unchanged.
        _ => {}
    }
}

/// Preflight one caller-supplied event for encode, against the same bounds
/// decode enforces. Rejections are typed; nothing is written.
pub(crate) fn preflight_event(ev: &InputEvent<'_>) -> Result<(), InputError> {
    match ev {
        InputEvent::Tap(t) => {
            if t.slot() > MAX_POINTER_SLOT {
                return Err(InputError::Body(InputReject::PointerSlotOutOfRange));
            }
            if t.x() > MAX_X {
                return Err(InputError::Body(InputReject::PointerXOutOfRange));
            }
            if t.y() > MAX_Y {
                return Err(InputError::Body(InputReject::PointerYOutOfRange));
            }
        }
        InputEvent::Pointer(p) => {
            if p.slot() > MAX_POINTER_SLOT {
                return Err(InputError::Body(InputReject::PointerSlotOutOfRange));
            }
            if p.x() > MAX_X {
                return Err(InputError::Body(InputReject::PointerXOutOfRange));
            }
            if p.y() > MAX_Y {
                return Err(InputError::Body(InputReject::PointerYOutOfRange));
            }
        }
        InputEvent::Swipe(s) => {
            if s.slot() > MAX_POINTER_SLOT {
                return Err(InputError::Body(InputReject::PointerSlotOutOfRange));
            }
            if s.x0() > MAX_X || s.x1() > MAX_X {
                return Err(InputError::Body(InputReject::PointerXOutOfRange));
            }
            if s.y0() > MAX_Y {
                return Err(InputError::Body(InputReject::PointerYOutOfRange));
            }
            // y1 must be exactly representable in the 8-bit high-byte
            // field: a multiple of the quantum within the y bound. Never
            // silently rounded.
            if s.y1() % SWIPE_Y1_QUANTUM != 0 || s.y1() > MAX_Y {
                return Err(InputError::Body(InputReject::PointerYOutOfRange));
            }
        }
        InputEvent::Key(k) => {
            if k.code() > MAX_KEY_CODE {
                return Err(InputError::Body(InputReject::KeyCodeOutOfRange));
            }
        }
        InputEvent::Text(t) => {
            if t.text().len() > MAX_TEXT_BYTES {
                return Err(InputError::Body(InputReject::TextLenOverCap));
            }
        }
    }
    Ok(())
}

/// Write one 16-byte record at `pos`, advancing `pos` by exactly 16. The
/// caller preflighted every field and the output capacity before any write,
/// so this never overruns and never panics.
pub(crate) fn write_event(ev: &InputEvent<'_>, output: &mut [u8], pos: &mut usize) {
    let r = &mut output[*pos..*pos + RECORD_EVENT_LEN];
    for b in r.iter_mut() {
        *b = 0;
    }
    match ev {
        InputEvent::Tap(t) => {
            r[0] = TAG_TAP;
            r[1] = 0; // action byte: Tap
            r[2] = t.slot();
            r[3..7].copy_from_slice(&t.x().to_le_bytes());
            r[7..11].copy_from_slice(&t.y().to_le_bytes());
        }
        InputEvent::Pointer(p) => {
            let (tag, action) = match p.action() {
                PointerAction::Press => (TAG_PRESS, 1),
                PointerAction::Move => (TAG_MOVE, 2),
                PointerAction::Release => (TAG_RELEASE, 3),
                // Tap-as-action cannot occur in a Pointer value; Tap has
                // its own variant. Defensive: encode as the Tap record.
                PointerAction::Tap => (TAG_TAP, 0),
            };
            r[0] = tag;
            r[1] = action;
            r[2] = p.slot();
            r[3..7].copy_from_slice(&p.x().to_le_bytes());
            r[7..11].copy_from_slice(&p.y().to_le_bytes());
        }
        InputEvent::Swipe(s) => {
            // y1 was preflighted: a multiple of the quantum within MAX_Y,
            // so the division is exact and the quotient fits u8.
            r[0] = TAG_SWIPE;
            r[1] = 0;
            r[2] = s.slot();
            r[3..7].copy_from_slice(&s.x0().to_le_bytes());
            r[7..11].copy_from_slice(&s.y0().to_le_bytes());
            r[11..15].copy_from_slice(&s.x1().to_le_bytes());
            r[15] = u8::try_from(s.y1() / SWIPE_Y1_QUANTUM).unwrap_or(u8::MAX);
        }
        InputEvent::Key(k) => {
            r[0] = TAG_KEY;
            r[1] = k.phase() as u8;
            r[3..5].copy_from_slice(&k.code().to_le_bytes());
        }
        InputEvent::Text(t) => {
            r[0] = TAG_TEXT;
            r[1] = 0;
            r[2] = t.text().len() as u8; // preflighted <= 13
            r[3..3 + t.text().len()].copy_from_slice(t.text().as_bytes());
        }
    }
    *pos += RECORD_EVENT_LEN;
}
