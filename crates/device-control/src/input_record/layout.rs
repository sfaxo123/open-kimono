//! Wire layout, event types and bounds for the input-body v1 codec.
//!
//! Every event record is FIXED at 16 bytes, so a batch body is exactly
//! `5 + 16 * count` bytes — batch size is bounded by the COUNT alone
//! (`MAX_BATCH`), checked before any event is examined.
//!
//! Record framing (all records, little-endian):
//! - byte 0      event tag, 0..=6;
//! - byte 1      sub-kind byte (action / phase);
//! - bytes 2..16 kind-specific payload, as laid out per kind below.
//!
//! Tag 0 `TapEvent` — press+release at one point, net-empty on the held set:
//! - byte 2      pointer slot, 0..=9;
//! - bytes 3..7  u32 x, 0..=1080;
//! - bytes 7..11 u32 y, 0..=2280;
//! - bytes 11..16 five reserved zero bytes.
//!
//! Tag 1 `SwipeEvent` — compact wire form of press → move → release along
//! a segment; endpoints only, net-empty on the held set. Exact framing:
//! - byte 2      pointer slot, 0..=9;
//! - bytes 3..7  u32 x0, 0..=1080;
//! - bytes 7..11 u32 y0, 0..=2280;
//! - bytes 11..15 u32 x1, 0..=1080;
//! - byte 15     u8 y1_high, with y1 = (y1_high as u32) * 256. Since y1 is
//!   bounded by 2280, this 8-bit quantization to multiples of 256 is exact
//!   for every y1 the wire accepts; a swipe whose y1 is not a multiple of
//!   256 is rejected at construction, never silently rounded.
//! Decoding a swipe expands it to its injection sequence (press at (x0,y0),
//! move to (x1,y0), move to (x1,y1), release at (x1,y1)); the diagonal is
//! decomposed into axis-aligned moves — no intermediate path point on the
//! true diagonal is fabricated, the wire declares only endpoints.
//!
//! Tag 2 `press` — pointer down, adds its slot to the held set:
//! - byte 2      pointer slot, 0..=9;
//! - bytes 3..7  u32 x; bytes 7..11 u32 y;
//! - bytes 11..16 five reserved zero bytes.
//!
//! Tag 3 `move` — pointer move; requires the slot already held, leaves the
//! held set unchanged:
//! - same field layout as press.
//!
//! Tag 4 `release` — pointer up, removes its slot from the held set:
//! - same field layout as press.
//!
//! Tag 5 `KeyEvent` — key down/up; press adds the code, release removes it:
//! - byte 2      key phase, 0=Press 1=Release;
//! - bytes 3..5  u16 key code, 0..=767;
//! - bytes 5..16 eleven reserved zero bytes.
//!
//! Tag 6 `TextEvent` — a run of UTF-8, no held-set effect:
//! - byte 2      u8 text byte length, 0..=13 (the payload area of a fixed
//!   record);
//! - bytes 3..3+len the UTF-8 text bytes;
//! - remaining bytes reserved zero.
//!
//! (Text longer than 13 bytes does not fit a fixed record: this v1 slice
//! bounds it at `MAX_TEXT_BYTES` = 13, the record payload area.)

/// Input-body schema version implemented by this codec.
pub const INPUT_BODY_VERSION: u8 = 1;
/// Fixed size of the input-body v1 header (u16 count + 3 zero bytes).
pub const INPUT_HEADER_LEN: usize = 5;
/// Fixed size of one event record.
pub const RECORD_EVENT_LEN: usize = 16;
/// Inclusive bound on the number of events in one batch. The batch-count
/// check happens on the header, BEFORE any allocation or per-event scan.
pub const MAX_BATCH: usize = 64;
/// Inclusive upper bound for x coordinates (SDM845-enough: 0..=1080).
pub const MAX_X: u32 = 1080;
/// Inclusive upper bound for y coordinates (SDM845-enough: 0..=2280).
pub const MAX_Y: u32 = 2280;
/// Inclusive bound for a pointer slot (matches session POINTER_SLOTS).
pub const MAX_POINTER_SLOT: u8 = 9;
/// Inclusive bound for a raw key code (proposed identifier, not OS-mapped;
/// matches the session key bitset domain 0..=767).
pub const MAX_KEY_CODE: u16 = 767;
/// Inclusive bound for one text event's UTF-8 byte length — the fixed
/// record payload area (16 - 3 framing bytes).
pub const MAX_TEXT_BYTES: usize = 13;
/// y1 of a swipe record is quantized to a multiple of this (the 8-bit
/// high byte in the fixed record). Exact for every y1 <= `MAX_Y` that the
/// wire accepts.
pub const SWIPE_Y1_QUANTUM: u32 = 256;

/// Wire tags for the event kinds. Unknown tag bytes are rejected.
pub const TAG_TAP: u8 = 0;
pub const TAG_SWIPE: u8 = 1;
pub const TAG_PRESS: u8 = 2;
pub const TAG_MOVE: u8 = 3;
pub const TAG_RELEASE: u8 = 4;
pub const TAG_KEY: u8 = 5;
pub const TAG_TEXT: u8 = 6;

/// Pointer actions carried by press/move/release records. Tap is encoded as
/// its own tag, so the action byte of tags 2..4 is checked against this
/// (each tag pins its own action; the byte is explicit on the wire so a
/// mislabeled record is rejected, not reinterpreted).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerAction {
    Tap = 0,
    Press = 1,
    Move = 2,
    Release = 3,
}

impl PointerAction {
    pub(crate) fn from_u8(b: u8) -> Option<PointerAction> {
        match b {
            0 => Some(PointerAction::Tap),
            1 => Some(PointerAction::Press),
            2 => Some(PointerAction::Move),
            3 => Some(PointerAction::Release),
            _ => None,
        }
    }
}

/// Key phases: Press, Release. Wire-validated on decode, so an invalid
/// phase byte can never become a `KeyPhase`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyPhase {
    Press = 0,
    Release = 1,
}

impl KeyPhase {
    pub(crate) fn from_u8(b: u8) -> Option<KeyPhase> {
        match b {
            0 => Some(KeyPhase::Press),
            1 => Some(KeyPhase::Release),
            _ => None,
        }
    }
}

/// One tap event: press+release at (x, y) on `slot`. Net effect on the
/// held set is empty.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TapEvent {
    pub(crate) slot: u8,
    pub(crate) x: u32,
    pub(crate) y: u32,
}

impl TapEvent {
    pub fn new(slot: u8, x: u32, y: u32) -> Self {
        TapEvent { slot, x, y }
    }
    pub fn slot(&self) -> u8 {
        self.slot
    }
    pub fn x(&self) -> u32 {
        self.x
    }
    pub fn y(&self) -> u32 {
        self.y
    }
}

/// One pointer press/move/release event. `action` must match the record
/// tag: only Press, Move and Release are expressible as values (Tap has
/// its own tag; swipe is the press/move/release sequence, see the module
/// docs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PointerEvent {
    pub(crate) action: PointerAction,
    pub(crate) slot: u8,
    pub(crate) x: u32,
    pub(crate) y: u32,
}

impl PointerEvent {
    pub fn press(slot: u8, x: u32, y: u32) -> Self {
        PointerEvent {
            action: PointerAction::Press,
            slot,
            x,
            y,
        }
    }
    pub fn move_to(slot: u8, x: u32, y: u32) -> Self {
        PointerEvent {
            action: PointerAction::Move,
            slot,
            x,
            y,
        }
    }
    pub fn release(slot: u8, x: u32, y: u32) -> Self {
        PointerEvent {
            action: PointerAction::Release,
            slot,
            x,
            y,
        }
    }
    pub fn action(&self) -> PointerAction {
        self.action
    }
    pub fn slot(&self) -> u8 {
        self.slot
    }
    pub fn x(&self) -> u32 {
        self.x
    }
    pub fn y(&self) -> u32 {
        self.y
    }
}

/// One key event. `code` is a proposed raw identifier in 0..=767, NOT a
/// proven OS keymap mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyEvent {
    pub(crate) code: u16,
    pub(crate) phase: KeyPhase,
}

impl KeyEvent {
    pub fn new(code: u16, phase: KeyPhase) -> Self {
        KeyEvent { code, phase }
    }
    pub fn code(&self) -> u16 {
        self.code
    }
    pub fn phase(&self) -> KeyPhase {
        self.phase
    }
}

/// One text event. `text` borrows the caller's buffer; Unicode and newlines
/// are preserved exactly, with no shell semantics. Bounded by
/// `MAX_TEXT_BYTES` (the fixed record payload area).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextEvent<'a> {
    pub(crate) text: &'a str,
}

impl<'a> TextEvent<'a> {
    pub fn new(text: &'a str) -> Self {
        TextEvent { text }
    }
    pub fn text(&self) -> &'a str {
        self.text
    }
}

/// One event in a batch. Every variant is one fixed 16-byte record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputEvent<'a> {
    Tap(TapEvent),
    Pointer(PointerEvent),
    Swipe(SwipeEvent),
    Key(KeyEvent),
    Text(TextEvent<'a>),
}

/// A compact swipe record: endpoints only. It is a wire FORM, not an
/// `InputEvent` value — decoding one expands it to its injection sequence
/// (press at the start, release at the end), and encoding the expansion back
/// produces the equivalent press/release pair rather than the compact form.
/// The full move path is never fabricated: the wire declares only endpoints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SwipeEvent {
    pub(crate) slot: u8,
    pub(crate) x0: u32,
    pub(crate) y0: u32,
    pub(crate) x1: u32,
    pub(crate) y1: u32,
}

impl SwipeEvent {
    pub fn new(slot: u8, x0: u32, y0: u32, x1: u32, y1: u32) -> Self {
        SwipeEvent {
            slot,
            x0,
            y0,
            x1,
            y1,
        }
    }
    pub fn slot(&self) -> u8 {
        self.slot
    }
    pub fn x0(&self) -> u32 {
        self.x0
    }
    pub fn y0(&self) -> u32 {
        self.y0
    }
    pub fn x1(&self) -> u32 {
        self.x1
    }
    pub fn y1(&self) -> u32 {
        self.y1
    }
}
