//! fb-write — minimal framebuffer console core (HW-10 salvage, reduced scope).
//!
//! `Frame` wraps a raw pixel buffer as 32-bit pixels in **X888 little-endian**
//! layout: each pixel occupies 4 consecutive bytes, low byte first, as
//! `[B, G, R, 0]` in memory — numerically the stored `u32` value is
//! `0x00RRGGBB`. On a little-endian framebuffer (the x86/ARM fastboot case)
//! writing the `u32` directly produces exactly those bytes.
//!
//! All drawing primitives **silently clip**: out-of-bounds pixels are dropped,
//! never a panic. Zero-dimension frames are legal; every write is a no-op.

#![no_std]
#![allow(clippy::too_many_arguments)]

extern crate alloc;

pub mod evidence;

/// A glyph: 16 rows of 8 bits each. Bit 7 (the high bit) of each row byte is
/// the LEFT-most pixel of that row; bit 0 is the right-most. Set bit = pixel
/// drawn in the foreground colour.
pub type Glyph8x16 = [[u8; 16]; 1];

/// Hand-authored glyph for 'A'.
const GLYPH_A: Glyph8x16 = [[
    0b00011000, // row 0  (blanked by the mutation gate test)
    0b00011000, // row 1  -> pixel at col 3 is set
    0b00100100, // row 2
    0b00100100, // row 3
    0b01000010, // row 4
    0b01000010, // row 5
    0b01000010, // row 6
    0b01111110, // row 7  -> crossbar, pixel at col 3 is set
    0b01000010, // row 8
    0b10000001, // row 9
    0b10000001, // row 10
    0b10000001, // row 11
    0b10000001, // row 12
    0b10000001, // row 13
    0b10000001, // row 14
    0b00000000, // row 15 (descender space)
]];

/// Hand-authored glyph for 'B'.
const GLYPH_B: Glyph8x16 = [[
    0b01111100, // row 0  -> pixel at col 2 is set
    0b01000010, // row 1
    0b01000010, // row 2
    0b01111100, // row 3
    0b01000010, // row 4
    0b01000010, // row 5
    0b01000010, // row 6
    0b01111100, // row 7
    0b01000010, // row 8
    0b01000010, // row 9
    0b01000010, // row 10
    0b01000010, // row 11
    0b01000010, // row 12
    0b01000010, // row 13
    0b01111100, // row 14
    0b00000000, // row 15
]];

/// Hand-authored glyph for ' ' (space) — entirely clear.
const GLYPH_SPACE: Glyph8x16 = [[0u8; 16]; 1];

/// Fallback glyph for every character without a hand-authored entry: a hollow
/// box, border rows/cols set, interior clear.
const GLYPH_BOX: Glyph8x16 = [[
    0b00111100, // row 0  (top border)
    0b01000010, // row 1
    0b01000010, // row 2
    0b01000010, // row 3
    0b01000010, // row 4
    0b01000010, // row 5
    0b01000010, // row 6
    0b01000010, // row 7
    0b01000010, // row 8
    0b01000010, // row 9
    0b01000010, // row 10
    0b01000010, // row 11
    0b01000010, // row 12
    0b01000010, // row 13
    0b00111100, // row 14 (bottom border)
    0b00000000, // row 15
]];

// ---------------------------------------------------------------------------
// Hand-authored font table, part 1/3 (HW-10F1): A-Z, 0-9, space.
//
// Style: 8 px wide, 16 rows, baseline at row 14 (row 15 is descender space).
// Uppercase letters span rows 2..14 (13 rows); digits span rows 3..14
// (12 rows). 3 px side bearings for round letters, 4 px for narrow ones.
// `static` data only — no runtime generation.
// ---------------------------------------------------------------------------

/// Hand-authored glyph for 'C'.
const GLYPH_C: Glyph8x16 = [[
    0b00011110, // row 0  (blanked by the mutation gate test)
    0b00100001, // row 1
    0b01000000, // row 2
    0b01000000, // row 3
    0b01000000, // row 4
    0b01000000, // row 5
    0b01000000, // row 6
    0b01000000, // row 7
    0b01000000, // row 8
    0b01000000, // row 9
    0b01000000, // row 10
    0b01000000, // row 11
    0b01000000, // row 12
    0b00100001, // row 13
    0b00011110, // row 14 (baseline)
    0b00000000, // row 15
]];

/// Hand-authored glyph for 'D'.
const GLYPH_D: Glyph8x16 = [[
    0b00000000, // row 0
    0b01111100, // row 1
    0b01000010, // row 2
    0b01000010, // row 3
    0b01000010, // row 4
    0b01000010, // row 5
    0b01000010, // row 6
    0b01000010, // row 7
    0b01000010, // row 8
    0b01000010, // row 9
    0b01000010, // row 10
    0b01000010, // row 11
    0b01000010, // row 12
    0b01000010, // row 13
    0b01111100, // row 14 (baseline)
    0b00000000, // row 15
]];

/// Hand-authored glyph for 'E'.
const GLYPH_E: Glyph8x16 = [[
    0b00000000, // row 0
    0b01111110, // row 1
    0b01000000, // row 2
    0b01000000, // row 3
    0b01000000, // row 4
    0b01000000, // row 5
    0b01111100, // row 6
    0b01000000, // row 7
    0b01000000, // row 8
    0b01000000, // row 9
    0b01000000, // row 10
    0b01000000, // row 11
    0b01000000, // row 12
    0b01000000, // row 13
    0b01111110, // row 14 (baseline)
    0b00000000, // row 15
]];

/// Hand-authored glyph for 'F'.
const GLYPH_F: Glyph8x16 = [[
    0b00000000, // row 0
    0b01111110, // row 1
    0b01000000, // row 2
    0b01000000, // row 3
    0b01000000, // row 4
    0b01000000, // row 5
    0b01111100, // row 6
    0b01000000, // row 7
    0b01000000, // row 8
    0b01000000, // row 9
    0b01000000, // row 10
    0b01000000, // row 11
    0b01000000, // row 12
    0b01000000, // row 13
    0b01000000, // row 14 (baseline)
    0b00000000, // row 15
]];

/// Hand-authored glyph for 'G'.
const GLYPH_G: Glyph8x16 = [[
    0b00000000, // row 0
    0b00111100, // row 1
    0b01000010, // row 2
    0b01000000, // row 3
    0b01000000, // row 4
    0b01000000, // row 5
    0b01000000, // row 6
    0b01001110, // row 7
    0b01000010, // row 8
    0b01000010, // row 9
    0b01000010, // row 10
    0b01000010, // row 11
    0b01000010, // row 12
    0b00100010, // row 13
    0b00011100, // row 14 (baseline)
    0b00000000, // row 15
]];

/// Hand-authored glyph for 'H'.
const GLYPH_LETTER_H: Glyph8x16 = [[
    0b00000000, // row 0
    0b01000010, // row 1
    0b01000010, // row 2
    0b01000010, // row 3
    0b01000010, // row 4
    0b01000010, // row 5
    0b01111110, // row 6
    0b01000010, // row 7
    0b01000010, // row 8
    0b01000010, // row 9
    0b01000010, // row 10
    0b01000010, // row 11
    0b01000010, // row 12
    0b01000010, // row 13
    0b01000010, // row 14 (baseline)
    0b00000000, // row 15
]];

/// Hand-authored glyph for 'I'.
const GLYPH_I: Glyph8x16 = [[
    0b00000000, // row 0
    0b00111110, // row 1
    0b00001000, // row 2
    0b00001000, // row 3
    0b00001000, // row 4
    0b00001000, // row 5
    0b00001000, // row 6
    0b00001000, // row 7
    0b00001000, // row 8
    0b00001000, // row 9
    0b00001000, // row 10
    0b00001000, // row 11
    0b00001000, // row 12
    0b00001000, // row 13
    0b00111110, // row 14 (baseline)
    0b00000000, // row 15
]];

/// Hand-authored glyph for 'J'.
const GLYPH_J: Glyph8x16 = [[
    0b00000000, // row 0
    0b00011110, // row 1
    0b00000100, // row 2
    0b00000100, // row 3
    0b00000100, // row 4
    0b00000100, // row 5
    0b00000100, // row 6
    0b00000100, // row 7
    0b00000100, // row 8
    0b00000100, // row 9
    0b00000100, // row 10
    0b00000100, // row 11
    0b01000100, // row 12
    0b00100100, // row 13
    0b00011000, // row 14 (baseline)
    0b00000000, // row 15
]];

/// Hand-authored glyph for 'K'.
const GLYPH_K: Glyph8x16 = [[
    0b00000000, // row 0
    0b01000010, // row 1
    0b01000100, // row 2
    0b01001000, // row 3
    0b01010000, // row 4
    0b01100000, // row 5
    0b01000000, // row 6
    0b01010000, // row 7
    0b01001000, // row 8
    0b01000100, // row 9
    0b01000010, // row 10
    0b01000010, // row 11
    0b01000010, // row 12
    0b01000010, // row 13
    0b01000010, // row 14 (baseline)
    0b00000000, // row 15
]];

/// Hand-authored glyph for 'L'.
const GLYPH_L: Glyph8x16 = [[
    0b00000000, // row 0
    0b01000000, // row 1
    0b01000000, // row 2
    0b01000000, // row 3
    0b01000000, // row 4
    0b01000000, // row 5
    0b01000000, // row 6
    0b01000000, // row 7
    0b01000000, // row 8
    0b01000000, // row 9
    0b01000000, // row 10
    0b01000000, // row 11
    0b01000000, // row 12
    0b01000000, // row 13
    0b01111110, // row 14 (baseline)
    0b00000000, // row 15
]];

/// Hand-authored glyph for 'M'.
const GLYPH_M: Glyph8x16 = [[
    0b00000000, // row 0
    0b01000010, // row 1
    0b01100110, // row 2
    0b01100110, // row 3
    0b01011010, // row 4
    0b01011010, // row 5
    0b01000010, // row 6
    0b01000010, // row 7
    0b01000010, // row 8
    0b01000010, // row 9
    0b01000010, // row 10
    0b01000010, // row 11
    0b01000010, // row 12
    0b01000010, // row 13
    0b01000010, // row 14 (baseline)
    0b00000000, // row 15
]];

/// Hand-authored glyph for 'N'.
const GLYPH_N: Glyph8x16 = [[
    0b00000000, // row 0
    0b01100010, // row 1
    0b01100010, // row 2
    0b01010010, // row 3
    0b01010010, // row 4
    0b01001010, // row 5
    0b01001010, // row 6
    0b01000110, // row 7
    0b01000110, // row 8
    0b01000010, // row 9
    0b01000010, // row 10
    0b01000010, // row 11
    0b01000010, // row 12
    0b01000010, // row 13
    0b01000010, // row 14 (baseline)
    0b00000000, // row 15
]];

/// Hand-authored glyph for 'O'.
const GLYPH_O: Glyph8x16 = [[
    0b00000000, // row 0
    0b00111100, // row 1
    0b01000010, // row 2
    0b01000010, // row 3
    0b01000010, // row 4
    0b01000010, // row 5
    0b01000010, // row 6
    0b01000010, // row 7
    0b01000010, // row 8
    0b01000010, // row 9
    0b01000010, // row 10
    0b01000010, // row 11
    0b01000010, // row 12
    0b00100100, // row 13
    0b00011000, // row 14 (baseline)
    0b00000000, // row 15
]];

/// Hand-authored glyph for 'P'.
const GLYPH_P: Glyph8x16 = [[
    0b00000000, // row 0
    0b01111100, // row 1
    0b01000010, // row 2
    0b01000010, // row 3
    0b01000010, // row 4
    0b01000010, // row 5
    0b01111100, // row 6
    0b01000000, // row 7
    0b01000000, // row 8
    0b01000000, // row 9
    0b01000000, // row 10
    0b01000000, // row 11
    0b01000000, // row 12
    0b01000000, // row 13
    0b01000000, // row 14 (baseline)
    0b00000000, // row 15
]];

/// Hand-authored glyph for 'Q'.
const GLYPH_Q: Glyph8x16 = [[
    0b00000000, // row 0
    0b00111100, // row 1
    0b01000010, // row 2
    0b01000010, // row 3
    0b01000010, // row 4
    0b01000010, // row 5
    0b01000010, // row 6
    0b01000010, // row 7
    0b01000010, // row 8
    0b01000010, // row 9
    0b01001010, // row 10
    0b01010010, // row 11
    0b00100100, // row 12
    0b00011001, // row 13
    0b00000110, // row 14 (baseline; tail crosses it)
    0b00000000, // row 15
]];

/// Hand-authored glyph for 'R'.
const GLYPH_R: Glyph8x16 = [[
    0b00000000, // row 0
    0b01111100, // row 1
    0b01000010, // row 2
    0b01000010, // row 3
    0b01000010, // row 4
    0b01000010, // row 5
    0b01111100, // row 6
    0b01001000, // row 7
    0b01000100, // row 8
    0b01000010, // row 9
    0b01000010, // row 10
    0b01000010, // row 11
    0b01000010, // row 12
    0b01000010, // row 13
    0b01000010, // row 14 (baseline)
    0b00000000, // row 15
]];

/// Hand-authored glyph for 'S'.
const GLYPH_S: Glyph8x16 = [[
    0b00000000, // row 0
    0b00111100, // row 1
    0b01000010, // row 2
    0b01000000, // row 3
    0b01000000, // row 4
    0b00100000, // row 5
    0b00011100, // row 6
    0b00000100, // row 7
    0b00000010, // row 8
    0b00000010, // row 9
    0b00000010, // row 10
    0b00000010, // row 11
    0b00000010, // row 12
    0b00100010, // row 13
    0b00011100, // row 14 (baseline)
    0b00000000, // row 15
]];

/// Hand-authored glyph for 'T'.
const GLYPH_T: Glyph8x16 = [[
    0b00000000, // row 0
    0b01111110, // row 1
    0b00010000, // row 2
    0b00010000, // row 3
    0b00010000, // row 4
    0b00010000, // row 5
    0b00010000, // row 6
    0b00010000, // row 7
    0b00010000, // row 8
    0b00010000, // row 9
    0b00010000, // row 10
    0b00010000, // row 11
    0b00010000, // row 12
    0b00010000, // row 13
    0b00010000, // row 14 (baseline)
    0b00000000, // row 15
]];

/// Hand-authored glyph for 'U'.
const GLYPH_U: Glyph8x16 = [[
    0b00000000, // row 0
    0b01000010, // row 1
    0b01000010, // row 2
    0b01000010, // row 3
    0b01000010, // row 4
    0b01000010, // row 5
    0b01000010, // row 6
    0b01000010, // row 7
    0b01000010, // row 8
    0b01000010, // row 9
    0b01000010, // row 10
    0b01000010, // row 11
    0b01000010, // row 12
    0b00100100, // row 13
    0b00011000, // row 14 (baseline)
    0b00000000, // row 15
]];

/// Hand-authored glyph for 'V'.
const GLYPH_V: Glyph8x16 = [[
    0b00000000, // row 0
    0b01000010, // row 1
    0b01000010, // row 2
    0b01000010, // row 3
    0b01000010, // row 4
    0b00100100, // row 5
    0b00100100, // row 6
    0b00100100, // row 7
    0b00100100, // row 8
    0b00011000, // row 9
    0b00011000, // row 10
    0b00011000, // row 11
    0b00011000, // row 12
    0b00011000, // row 13
    0b00000000, // row 14 (apex sits on row 13)
    0b00000000, // row 15
]];

/// Hand-authored glyph for 'W'.
const GLYPH_LETTER_W: Glyph8x16 = [[
    0b00000000, // row 0
    0b01000010, // row 1
    0b01000010, // row 2
    0b01000010, // row 3
    0b01000010, // row 4
    0b01000010, // row 5
    0b01011010, // row 6
    0b01011010, // row 7
    0b01011010, // row 8
    0b01010110, // row 9
    0b00100100, // row 10
    0b00100100, // row 11
    0b00100100, // row 12
    0b00100100, // row 13
    0b00000000, // row 14 (feet sit on row 13)
    0b00000000, // row 15
]];

/// Hand-authored glyph for 'X'.
const GLYPH_X: Glyph8x16 = [[
    0b00000000, // row 0
    0b01000010, // row 1
    0b00100100, // row 2
    0b00100100, // row 3
    0b00011000, // row 4
    0b00011000, // row 5
    0b00011000, // row 6
    0b00011000, // row 7
    0b00011000, // row 8
    0b00100100, // row 9
    0b00100100, // row 10
    0b01000010, // row 11
    0b01000010, // row 12
    0b01000010, // row 13
    0b01000010, // row 14 (baseline)
    0b00000000, // row 15
]];

/// Hand-authored glyph for 'Y'.
const GLYPH_Y: Glyph8x16 = [[
    0b00000000, // row 0
    0b01000010, // row 1
    0b00100100, // row 2
    0b00100100, // row 3
    0b00011000, // row 4
    0b00011000, // row 5
    0b00011000, // row 6
    0b00010000, // row 7
    0b00010000, // row 8
    0b00010000, // row 9
    0b00010000, // row 10
    0b00010000, // row 11
    0b00010000, // row 12
    0b00010000, // row 13
    0b00010000, // row 14 (baseline)
    0b00000000, // row 15
]];

/// Hand-authored glyph for 'Z'.
const GLYPH_Z: Glyph8x16 = [[
    0b00000000, // row 0
    0b01111110, // row 1
    0b00000010, // row 2
    0b00000100, // row 3
    0b00000100, // row 4
    0b00001000, // row 5
    0b00001000, // row 6
    0b00010000, // row 7
    0b00010000, // row 8
    0b00100000, // row 9
    0b00100000, // row 10
    0b01000000, // row 11
    0b01000000, // row 12
    0b01000000, // row 13
    0b01111110, // row 14 (baseline)
    0b00000000, // row 15
]];

/// Hand-authored glyph for '0' (slashed so it never reads as 'O').
const GLYPH_0: Glyph8x16 = [[
    0b00000000, // row 0
    0b00000000, // row 1
    0b00111100, // row 2
    0b01000110, // row 3
    0b01001010, // row 4
    0b01010010, // row 5
    0b01010010, // row 6
    0b01100010, // row 7
    0b01100010, // row 8
    0b01000110, // row 9
    0b01001010, // row 10
    0b01001010, // row 11
    0b01010010, // row 12
    0b01100010, // row 13
    0b00111100, // row 14 (baseline)
    0b00000000, // row 15
]];

/// Hand-authored glyph for '1'.
const GLYPH_1: Glyph8x16 = [[
    0b00000000, // row 0
    0b00000000, // row 1
    0b00011000, // row 2
    0b00111000, // row 3
    0b00001000, // row 4
    0b00001000, // row 5
    0b00001000, // row 6
    0b00001000, // row 7
    0b00001000, // row 8
    0b00001000, // row 9
    0b00001000, // row 10
    0b00001000, // row 11
    0b00001000, // row 12
    0b00001000, // row 13
    0b00111110, // row 14 (baseline)
    0b00000000, // row 15
]];

/// Hand-authored glyph for '2'.
const GLYPH_2: Glyph8x16 = [[
    0b00000000, // row 0
    0b00000000, // row 1
    0b00111100, // row 2
    0b01000010, // row 3
    0b01000010, // row 4
    0b00000100, // row 5
    0b00001000, // row 6
    0b00010000, // row 7
    0b00100000, // row 8
    0b01000000, // row 9
    0b01000000, // row 10
    0b01000000, // row 11
    0b01000000, // row 12
    0b01000000, // row 13
    0b01111110, // row 14 (baseline)
    0b00000000, // row 15
]];

/// Hand-authored glyph for '3'.
const GLYPH_3: Glyph8x16 = [[
    0b00000000, // row 0
    0b00000000, // row 1
    0b01111100, // row 2
    0b00000010, // row 3
    0b00000010, // row 4
    0b00000100, // row 5
    0b00011000, // row 6
    0b00000100, // row 7
    0b00000010, // row 8
    0b00000010, // row 9
    0b00000010, // row 10
    0b00000010, // row 11
    0b00000010, // row 12
    0b01000010, // row 13
    0b00111100, // row 14 (baseline)
    0b00000000, // row 15
]];

/// Hand-authored glyph for '4'.
const GLYPH_4: Glyph8x16 = [[
    0b00000000, // row 0
    0b00000000, // row 1
    0b00001000, // row 2
    0b00011000, // row 3
    0b00101000, // row 4
    0b01001000, // row 5
    0b01001000, // row 6
    0b01001000, // row 7
    0b01001000, // row 8
    0b01001000, // row 9
    0b01111110, // row 10
    0b00001000, // row 11
    0b00001000, // row 12
    0b00001000, // row 13
    0b00001000, // row 14 (baseline)
    0b00000000, // row 15
]];

/// Hand-authored glyph for '5'.
const GLYPH_5: Glyph8x16 = [[
    0b00000000, // row 0
    0b00000000, // row 1
    0b01111110, // row 2
    0b01000000, // row 3
    0b01000000, // row 4
    0b01000000, // row 5
    0b01111100, // row 6
    0b00000010, // row 7
    0b00000010, // row 8
    0b00000010, // row 9
    0b00000010, // row 10
    0b00000010, // row 11
    0b00000010, // row 12
    0b01000010, // row 13
    0b00111100, // row 14 (baseline)
    0b00000000, // row 15
]];

/// Hand-authored glyph for '6'.
const GLYPH_6: Glyph8x16 = [[
    0b00000000, // row 0
    0b00000000, // row 1
    0b00011100, // row 2
    0b00100000, // row 3
    0b01000000, // row 4
    0b01000000, // row 5
    0b01111100, // row 6
    0b01000010, // row 7
    0b01000010, // row 8
    0b01000010, // row 9
    0b01000010, // row 10
    0b01000010, // row 11
    0b01000010, // row 12
    0b00100100, // row 13
    0b00011000, // row 14 (baseline)
    0b00000000, // row 15
]];

/// Hand-authored glyph for '7'.
const GLYPH_7: Glyph8x16 = [[
    0b00000000, // row 0
    0b00000000, // row 1
    0b01111110, // row 2
    0b01000010, // row 3
    0b00000100, // row 4
    0b00001000, // row 5
    0b00001000, // row 6
    0b00010000, // row 7
    0b00010000, // row 8
    0b00010000, // row 9
    0b00100000, // row 10
    0b00100000, // row 11
    0b00100000, // row 12
    0b00100000, // row 13
    0b00100000, // row 14 (baseline)
    0b00000000, // row 15
]];

/// Hand-authored glyph for '8'.
const GLYPH_8: Glyph8x16 = [[
    0b00000000, // row 0
    0b00000000, // row 1
    0b00111100, // row 2
    0b01000010, // row 3
    0b01000010, // row 4
    0b01000010, // row 5
    0b00111100, // row 6
    0b01000010, // row 7
    0b01000010, // row 8
    0b01000010, // row 9
    0b01000010, // row 10
    0b01000010, // row 11
    0b01000010, // row 12
    0b00100100, // row 13
    0b00011000, // row 14 (baseline)
    0b00000000, // row 15
]];

/// Hand-authored glyph for '9'.
const GLYPH_9: Glyph8x16 = [[
    0b00000000, // row 0
    0b00000000, // row 1
    0b00111100, // row 2
    0b01000010, // row 3
    0b01000010, // row 4
    0b01000010, // row 5
    0b01000010, // row 6
    0b01000010, // row 7
    0b01000010, // row 8
    0b01000010, // row 9
    0b01000110, // row 10
    0b01001010, // row 11
    0b00110010, // row 12
    0b00000100, // row 13
    0b00011000, // row 14 (baseline)
    0b00000000, // row 15
]];

/// The font table for part 1/3 (HW-10F1): space, A-Z and 0-9 — 37 entries.
/// Lookup is a linear scan over this `const` array: at console sizes (a few
/// hundred glyph lookups per frame) the scan is negligible, and the table
/// keeps the glyph data, not control flow, as the single source of truth —
/// parts 2/3 and 3/3 just extend the array.
const FONT_UPPER_DIGIT: [(char, &Glyph8x16); 37] = [
    (' ', &GLYPH_SPACE),
    ('A', &GLYPH_A),
    ('B', &GLYPH_B),
    ('C', &GLYPH_C),
    ('D', &GLYPH_D),
    ('E', &GLYPH_E),
    ('F', &GLYPH_F),
    ('G', &GLYPH_G),
    ('H', &GLYPH_LETTER_H),
    ('I', &GLYPH_I),
    ('J', &GLYPH_J),
    ('K', &GLYPH_K),
    ('L', &GLYPH_L),
    ('M', &GLYPH_M),
    ('N', &GLYPH_N),
    ('O', &GLYPH_O),
    ('P', &GLYPH_P),
    ('Q', &GLYPH_Q),
    ('R', &GLYPH_R),
    ('S', &GLYPH_S),
    ('T', &GLYPH_T),
    ('U', &GLYPH_U),
    ('V', &GLYPH_V),
    ('W', &GLYPH_LETTER_W),
    ('X', &GLYPH_X),
    ('Y', &GLYPH_Y),
    ('Z', &GLYPH_Z),
    ('0', &GLYPH_0),
    ('1', &GLYPH_1),
    ('2', &GLYPH_2),
    ('3', &GLYPH_3),
    ('4', &GLYPH_4),
    ('5', &GLYPH_5),
    ('6', &GLYPH_6),
    ('7', &GLYPH_7),
    ('8', &GLYPH_8),
    ('9', &GLYPH_9),
];

// ---------------------------------------------------------------------------
// Hand-authored font table, part 2/3 (HW-10F2): lowercase a-z — 26 glyphs.
//
// Style: 8 px wide, 16 rows, baseline at row 14 (row 15 is descender space).
// x-height letters (a,c,e,m,n,r,s,u,v,w,x,z) span rows 5..14 (10 rows);
// ascenders (b,d,f,h,k,l,t) reach up to row 4; round bowls (b,o,p,q) round
// off at rows 4 and 13. Descenders (g,j,p,q,y) would reach rows 15-16 and
// are clamped to row 15. `static` data only — no runtime generation.
// ---------------------------------------------------------------------------

/// Hand-authored glyph for 'a'.
const GLYPH_A_LOWER: Glyph8x16 = [[
    0b00000000, // row 0
    0b00000000, // row 1
    0b00000000, // row 2
    0b00000000, // row 3
    0b00000000, // row 4
    0b00111100, // row 5
    0b00000010, // row 6
    0b00000010, // row 7
    0b00111100, // row 8
    0b01000010, // row 9
    0b01000010, // row 10
    0b01000010, // row 11
    0b01000010, // row 12
    0b00100010, // row 13
    0b00011100, // row 14 (baseline)
    0b00000000, // row 15
]];

/// Hand-authored glyph for 'b'.
const GLYPH_B_LOWER: Glyph8x16 = [[
    0b00000000, // row 0
    0b00000000, // row 1
    0b00000000, // row 2
    0b00000000, // row 3
    0b01000000, // row 4  (ascender)
    0b01000000, // row 5
    0b01000000, // row 6
    0b01011100, // row 7  (bowl starts)
    0b01100010, // row 8
    0b01000010, // row 9
    0b01000010, // row 10
    0b01000010, // row 11
    0b01000010, // row 12
    0b01100010, // row 13
    0b01011100, // row 14 (baseline)
    0b00000000, // row 15
]];

/// Hand-authored glyph for 'c'.
const GLYPH_C_LOWER: Glyph8x16 = [[
    0b00000000, // row 0
    0b00000000, // row 1
    0b00000000, // row 2
    0b00000000, // row 3
    0b00000000, // row 4
    0b00111100, // row 5
    0b01000010, // row 6
    0b01000000, // row 7
    0b01000000, // row 8
    0b01000000, // row 9
    0b01000000, // row 10
    0b01000000, // row 11
    0b01000000, // row 12
    0b00100010, // row 13
    0b00011100, // row 14 (baseline)
    0b00000000, // row 15
]];

/// Hand-authored glyph for 'd'.
const GLYPH_D_LOWER: Glyph8x16 = [[
    0b00000000, // row 0
    0b00000000, // row 1
    0b00000000, // row 2
    0b00000000, // row 3
    0b00000100, // row 4  (ascender)
    0b00000100, // row 5
    0b00000100, // row 6
    0b00111100, // row 7  (bowl starts)
    0b01000100, // row 8
    0b01000100, // row 9
    0b01000100, // row 10
    0b01000100, // row 11
    0b01000100, // row 12
    0b01001100, // row 13
    0b00110100, // row 14 (baseline)
    0b00000000, // row 15
]];

/// Hand-authored glyph for 'e'.
const GLYPH_E_LOWER: Glyph8x16 = [[
    0b00000000, // row 0
    0b00000000, // row 1
    0b00000000, // row 2
    0b00000000, // row 3
    0b00000000, // row 4
    0b00111100, // row 5
    0b01000010, // row 6
    0b01000010, // row 7
    0b01111100, // row 8  (crossbar)
    0b01000000, // row 9
    0b01000000, // row 10
    0b01000000, // row 11
    0b01000000, // row 12
    0b00100010, // row 13
    0b00011100, // row 14 (baseline)
    0b00000000, // row 15
]];

/// Hand-authored glyph for 'f'.
const GLYPH_F_LOWER: Glyph8x16 = [[
    0b00000000, // row 0
    0b00000000, // row 1
    0b00000000, // row 2
    0b00011100, // row 3  (ascender hook)
    0b00100010, // row 4
    0b00100000, // row 5
    0b00100000, // row 6
    0b01111100, // row 7  (crossbar)
    0b00100000, // row 8
    0b00100000, // row 9
    0b00100000, // row 10
    0b00100000, // row 11
    0b00100000, // row 12
    0b00100000, // row 13
    0b00100000, // row 14 (baseline)
    0b00000000, // row 15
]];

/// Hand-authored glyph for 'g'. A single-pixel ascender flag on rows 0-1
/// (cols 4-5) disambiguates it from 'q' at a glance and is the mutation-gate
/// probe row (HW-10F2 test blanks it to prove glyph_g_blit catches damage).
const GLYPH_G_LOWER: Glyph8x16 = [[
    0b00001100, // row 0  (ascender flag — the mutation-gate probe row)
    0b00001100, // row 1
    0b00000000, // row 2
    0b00000000, // row 3
    0b00000000, // row 4
    0b00111100, // row 5  (top of bowl)
    0b01000010, // row 6
    0b01000010, // row 7
    0b01000010, // row 8
    0b01000010, // row 9
    0b01000010, // row 10
    0b01000010, // row 11
    0b00111100, // row 12
    0b00000100, // row 13
    0b00001000, // row 14 (baseline)
    0b00110000, // row 15 (descender tail, clamped at row 15)
]];

/// Hand-authored glyph for 'h'.
const GLYPH_H_LOWER: Glyph8x16 = [[
    0b00000000, // row 0
    0b00000000, // row 1
    0b00000000, // row 2
    0b00000000, // row 3
    0b01000000, // row 4  (ascender)
    0b01000000, // row 5
    0b01000000, // row 6
    0b01011100, // row 7  (arch)
    0b01100010, // row 8
    0b01000010, // row 9
    0b01000010, // row 10
    0b01000010, // row 11
    0b01000010, // row 12
    0b01000010, // row 13
    0b01000010, // row 14 (baseline)
    0b00000000, // row 15
]];

/// Hand-authored glyph for 'i'.
const GLYPH_I_LOWER: Glyph8x16 = [[
    0b00000000, // row 0
    0b00000000, // row 1
    0b00000000, // row 2
    0b00000000, // row 3
    0b00010000, // row 4  (dot)
    0b00000000, // row 5
    0b00010000, // row 6
    0b00010000, // row 7
    0b00010000, // row 8
    0b00010000, // row 9
    0b00010000, // row 10
    0b00010000, // row 11
    0b00010000, // row 12
    0b00010000, // row 13
    0b00010000, // row 14 (baseline)
    0b00000000, // row 15
]];

/// Hand-authored glyph for 'j'.
const GLYPH_J_LOWER: Glyph8x16 = [[
    0b00000000, // row 0
    0b00000000, // row 1
    0b00000000, // row 2
    0b00000000, // row 3
    0b00001000, // row 4  (dot)
    0b00000000, // row 5
    0b00001000, // row 6
    0b00001000, // row 7
    0b00001000, // row 8
    0b00001000, // row 9
    0b00001000, // row 10
    0b00001000, // row 11
    0b00001000, // row 12
    0b00001000, // row 13
    0b00001000, // row 14 (baseline)
    0b00110000, // row 15 (descender hook, clamped at row 15)
]];

/// Hand-authored glyph for 'k'.
const GLYPH_K_LOWER: Glyph8x16 = [[
    0b00000000, // row 0
    0b00000000, // row 1
    0b00000000, // row 2
    0b00000000, // row 3
    0b01000000, // row 4  (ascender)
    0b01000000, // row 5
    0b01000000, // row 6
    0b01000100, // row 7
    0b01001000, // row 8
    0b01010000, // row 9
    0b01100000, // row 10
    0b01010000, // row 11
    0b01001000, // row 12
    0b01000100, // row 13
    0b01000010, // row 14 (baseline)
    0b00000000, // row 15
]];

/// Hand-authored glyph for 'l'.
const GLYPH_L_LOWER: Glyph8x16 = [[
    0b00000000, // row 0
    0b00000000, // row 1
    0b00000000, // row 2
    0b00000000, // row 3
    0b01000000, // row 4  (ascender)
    0b01000000, // row 5
    0b01000000, // row 6
    0b01000000, // row 7
    0b01000000, // row 8
    0b01000000, // row 9
    0b01000000, // row 10
    0b01000000, // row 11
    0b01000000, // row 12
    0b01000010, // row 13
    0b01000100, // row 14 (baseline)
    0b00000000, // row 15
]];

/// Hand-authored glyph for 'm'.
const GLYPH_M_LOWER: Glyph8x16 = [[
    0b00000000, // row 0
    0b00000000, // row 1
    0b00000000, // row 2
    0b00000000, // row 3
    0b00000000, // row 4
    0b01101100, // row 5  (two humps)
    0b01010010, // row 6
    0b01010010, // row 7
    0b01010010, // row 8
    0b01010010, // row 9
    0b01010010, // row 10
    0b01010010, // row 11
    0b01010010, // row 12
    0b01010010, // row 13
    0b01010010, // row 14 (baseline)
    0b00000000, // row 15
]];

/// Hand-authored glyph for 'n'.
const GLYPH_N_LOWER: Glyph8x16 = [[
    0b00000000, // row 0
    0b00000000, // row 1
    0b00000000, // row 2
    0b00000000, // row 3
    0b00000000, // row 4
    0b01111000, // row 5  (arch)
    0b01000100, // row 6
    0b01000100, // row 7
    0b01000100, // row 8
    0b01000100, // row 9
    0b01000100, // row 10
    0b01000100, // row 11
    0b01000100, // row 12
    0b01000100, // row 13
    0b01000100, // row 14 (baseline)
    0b00000000, // row 15
]];

/// Hand-authored glyph for 'o'.
const GLYPH_O_LOWER: Glyph8x16 = [[
    0b00000000, // row 0
    0b00000000, // row 1
    0b00000000, // row 2
    0b00000000, // row 3
    0b00000000, // row 4
    0b00111100, // row 5
    0b01000010, // row 6
    0b01000010, // row 7
    0b01000010, // row 8
    0b01000010, // row 9
    0b01000010, // row 10
    0b01000010, // row 11
    0b01000010, // row 12
    0b00100100, // row 13
    0b00011000, // row 14 (baseline)
    0b00000000, // row 15
]];

/// Hand-authored glyph for 'p'.
const GLYPH_P_LOWER: Glyph8x16 = [[
    0b00000000, // row 0
    0b00000000, // row 1
    0b00000000, // row 2
    0b00000000, // row 3
    0b00000000, // row 4
    0b01011100, // row 5  (bowl starts)
    0b01100010, // row 6
    0b01000010, // row 7
    0b01000010, // row 8
    0b01000010, // row 9
    0b01000010, // row 10
    0b01000010, // row 11
    0b01000010, // row 12
    0b01100010, // row 13
    0b01011100, // row 14 (baseline)
    0b01000000, // row 15 (descender stem, clamped at row 15)
]];

/// Hand-authored glyph for 'q'.
const GLYPH_Q_LOWER: Glyph8x16 = [[
    0b00000000, // row 0
    0b00000000, // row 1
    0b00000000, // row 2
    0b00000000, // row 3
    0b00000000, // row 4
    0b00111100, // row 5  (bowl starts)
    0b01000100, // row 6
    0b01000100, // row 7
    0b01000100, // row 8
    0b01000100, // row 9
    0b01000100, // row 10
    0b01000100, // row 11
    0b01000100, // row 12
    0b01001100, // row 13
    0b00110100, // row 14 (baseline)
    0b00000100, // row 15 (descender stem, clamped at row 15)
]];

/// Hand-authored glyph for 'r'.
const GLYPH_R_LOWER: Glyph8x16 = [[
    0b00000000, // row 0
    0b00000000, // row 1
    0b00000000, // row 2
    0b00000000, // row 3
    0b00000000, // row 4
    0b01011100, // row 5
    0b01100010, // row 6
    0b01000000, // row 7
    0b01000000, // row 8
    0b01000000, // row 9
    0b01000000, // row 10
    0b01000000, // row 11
    0b01000000, // row 12
    0b01000000, // row 13
    0b01000000, // row 14 (baseline)
    0b00000000, // row 15
]];

/// Hand-authored glyph for 's'.
const GLYPH_S_LOWER: Glyph8x16 = [[
    0b00000000, // row 0
    0b00000000, // row 1
    0b00000000, // row 2
    0b00000000, // row 3
    0b00000000, // row 4
    0b00111100, // row 5
    0b01000010, // row 6
    0b01000000, // row 7
    0b00100000, // row 8
    0b00011000, // row 9
    0b00000100, // row 10
    0b00000010, // row 11
    0b00000010, // row 12
    0b00100010, // row 13
    0b00011100, // row 14 (baseline)
    0b00000000, // row 15
]];

/// Hand-authored glyph for 't'.
const GLYPH_T_LOWER: Glyph8x16 = [[
    0b00000000, // row 0
    0b00000000, // row 1
    0b00000000, // row 2
    0b00000000, // row 3
    0b00100000, // row 4  (ascender stem)
    0b00100000, // row 5
    0b00100000, // row 6
    0b01111100, // row 7  (crossbar)
    0b00100000, // row 8
    0b00100000, // row 9
    0b00100000, // row 10
    0b00100000, // row 11
    0b00100000, // row 12
    0b00100010, // row 13
    0b00011100, // row 14 (baseline)
    0b00000000, // row 15
]];

/// Hand-authored glyph for 'u'.
const GLYPH_U_LOWER: Glyph8x16 = [[
    0b00000000, // row 0
    0b00000000, // row 1
    0b00000000, // row 2
    0b00000000, // row 3
    0b00000000, // row 4
    0b01000100, // row 5
    0b01000100, // row 6
    0b01000100, // row 7
    0b01000100, // row 8
    0b01000100, // row 9
    0b01000100, // row 10
    0b01000100, // row 11
    0b01000100, // row 12
    0b01001100, // row 13
    0b00110100, // row 14 (baseline)
    0b00000000, // row 15
]];

/// Hand-authored glyph for 'v'.
const GLYPH_V_LOWER: Glyph8x16 = [[
    0b00000000, // row 0
    0b00000000, // row 1
    0b00000000, // row 2
    0b00000000, // row 3
    0b00000000, // row 4
    0b01000010, // row 5
    0b01000010, // row 6
    0b00100100, // row 7
    0b00100100, // row 8
    0b00100100, // row 9
    0b00011000, // row 10
    0b00011000, // row 11
    0b00011000, // row 12
    0b00011000, // row 13
    0b00000000, // row 14 (apex sits on row 13)
    0b00000000, // row 15
]];

/// Hand-authored glyph for 'w'.
const GLYPH_W_LOWER: Glyph8x16 = [[
    0b00000000, // row 0
    0b00000000, // row 1
    0b00000000, // row 2
    0b00000000, // row 3
    0b00000000, // row 4
    0b01000010, // row 5
    0b01000010, // row 6
    0b01000010, // row 7
    0b01011010, // row 8
    0b01011010, // row 9
    0b01011010, // row 10
    0b00100100, // row 11
    0b00100100, // row 12
    0b00100100, // row 13
    0b00000000, // row 14 (feet sit on row 13)
    0b00000000, // row 15
]];

/// Hand-authored glyph for 'x'.
const GLYPH_X_LOWER: Glyph8x16 = [[
    0b00000000, // row 0
    0b00000000, // row 1
    0b00000000, // row 2
    0b00000000, // row 3
    0b00000000, // row 4
    0b01000010, // row 5
    0b00100100, // row 6
    0b00100100, // row 7
    0b00011000, // row 8
    0b00011000, // row 9
    0b00011000, // row 10
    0b00100100, // row 11
    0b00100100, // row 12
    0b01000010, // row 13
    0b01000010, // row 14 (baseline)
    0b00000000, // row 15
]];

/// Hand-authored glyph for 'y'.
const GLYPH_Y_LOWER: Glyph8x16 = [[
    0b00000000, // row 0
    0b00000000, // row 1
    0b00000000, // row 2
    0b00000000, // row 3
    0b00000000, // row 4
    0b01000100, // row 5
    0b01000100, // row 6
    0b00101000, // row 7
    0b00101000, // row 8
    0b00010000, // row 9
    0b00010000, // row 10
    0b00010000, // row 11
    0b00010000, // row 12
    0b00010000, // row 13
    0b00100000, // row 14 (baseline)
    0b01000000, // row 15 (descender tail, clamped at row 15)
]];

/// Hand-authored glyph for 'z'.
const GLYPH_Z_LOWER: Glyph8x16 = [[
    0b00000000, // row 0
    0b00000000, // row 1
    0b00000000, // row 2
    0b00000000, // row 3
    0b00000000, // row 4
    0b01111110, // row 5
    0b00000010, // row 6
    0b00000100, // row 7
    0b00001000, // row 8
    0b00010000, // row 9
    0b00100000, // row 10
    0b00100000, // row 11
    0b00100000, // row 12
    0b01000000, // row 13
    0b01111110, // row 14 (baseline)
    0b00000000, // row 15
]];

/// The font table for part 2/3 (HW-10F2): lowercase a-z — 26 entries. Same
/// linear-scan rationale as [`FONT_UPPER_DIGIT`].
const FONT_LOWER: [(char, &Glyph8x16); 26] = [
    ('a', &GLYPH_A_LOWER),
    ('b', &GLYPH_B_LOWER),
    ('c', &GLYPH_C_LOWER),
    ('d', &GLYPH_D_LOWER),
    ('e', &GLYPH_E_LOWER),
    ('f', &GLYPH_F_LOWER),
    ('g', &GLYPH_G_LOWER),
    ('h', &GLYPH_H_LOWER),
    ('i', &GLYPH_I_LOWER),
    ('j', &GLYPH_J_LOWER),
    ('k', &GLYPH_K_LOWER),
    ('l', &GLYPH_L_LOWER),
    ('m', &GLYPH_M_LOWER),
    ('n', &GLYPH_N_LOWER),
    ('o', &GLYPH_O_LOWER),
    ('p', &GLYPH_P_LOWER),
    ('q', &GLYPH_Q_LOWER),
    ('r', &GLYPH_R_LOWER),
    ('s', &GLYPH_S_LOWER),
    ('t', &GLYPH_T_LOWER),
    ('u', &GLYPH_U_LOWER),
    ('v', &GLYPH_V_LOWER),
    ('w', &GLYPH_W_LOWER),
    ('x', &GLYPH_X_LOWER),
    ('y', &GLYPH_Y_LOWER),
    ('z', &GLYPH_Z_LOWER),
];

// ---------------------------------------------------------------------------
// Hand-authored font table, part 3/3 (HW-10F3): punctuation + symbols — the
// remaining 32 printable ASCII glyphs: ! " # $ % & ' ( ) * + , - . / : ;
// < = > ? @ [ \ ] ^ _ ` { | } ~
//
// Style: 8 px wide, 16 rows, baseline at row 14 (row 15 is descender space).
// Most glyphs sit in the x-height band rows 5..14; '#' and '$' span rows
// 2..13/14; '^' sits mid (rows 4..8), '_' low (row 13, just above baseline),
// '~' mid (row 9). Descenders (',' ';' '&' tail '@' tail) clamp at row 15.
// `static` data only — no runtime generation.
// ---------------------------------------------------------------------------

/// Hand-authored glyph for '!'. Stem rows 5..12, dot on the baseline.
const GLYPH_EXCL: Glyph8x16 = [[
    0b00000000, // row 0
    0b00000000, // row 1
    0b00000000, // row 2
    0b00000000, // row 3
    0b00000000, // row 4
    0b00010000, // row 5
    0b00010000, // row 6
    0b00010000, // row 7
    0b00010000, // row 8
    0b00010000, // row 9
    0b00010000, // row 10
    0b00010000, // row 11
    0b00010000, // row 12
    0b00000000, // row 13
    0b00010000, // row 14 (baseline dot)
    0b00000000, // row 15
]];

/// Hand-authored glyph for '"'. Two short double strokes.
const GLYPH_QUOTE: Glyph8x16 = [[
    0b00000000, // row 0
    0b00000000, // row 1
    0b00000000, // row 2
    0b00000000, // row 3
    0b00000000, // row 4
    0b00100100, // row 5
    0b00100100, // row 6
    0b00100100, // row 7
    0b00100100, // row 8
    0b00000000, // row 9
    0b00000000, // row 10
    0b00000000, // row 11
    0b00000000, // row 12
    0b00000000, // row 13
    0b00000000, // row 14
    0b00000000, // row 15
]];

/// Hand-authored glyph for '#'. Two verticals, two horizontals; spans rows
/// 2..13. Row 4 = 0b01010100: cols 1, 3, 5 set (mutation-gate style anchors).
const GLYPH_HASH: Glyph8x16 = [[
    0b00000000, // row 0
    0b00000000, // row 1
    0b00101000, // row 2
    0b00101000, // row 3
    0b01010100, // row 4  (horizontal 1)
    0b01111110, // row 5  (horizontal 1 full)
    0b00101000, // row 6
    0b00101000, // row 7
    0b01111110, // row 8  (horizontal 2 full)
    0b01010100, // row 9  (horizontal 2)
    0b00101000, // row 10
    0b00101000, // row 11
    0b00101000, // row 12
    0b00000000, // row 13
    0b00000000, // row 14
    0b00000000, // row 15
]];

/// Hand-authored glyph for '$'. S-bar through an S-body; spans rows 2..13.
const GLYPH_DOLLAR: Glyph8x16 = [[
    0b00000000, // row 0
    0b00000000, // row 1
    0b00010000, // row 2  (stem top)
    0b00111100, // row 3
    0b01010010, // row 4
    0b01010000, // row 5
    0b00111000, // row 6  (waist)
    0b00010100, // row 7
    0b00010010, // row 8
    0b00111100, // row 9
    0b01010000, // row 10
    0b00010000, // row 11 (stem bottom)
    0b00000000, // row 12
    0b00000000, // row 13
    0b00000000, // row 14
    0b00000000, // row 15
]];

/// Hand-authored glyph for '%'. Diagonal between two small rings.
const GLYPH_PERCENT: Glyph8x16 = [[
    0b00000000, // row 0
    0b00000000, // row 1
    0b00110000, // row 2  (upper-left ring)
    0b00110010, // row 3
    0b00000100, // row 4
    0b00001000, // row 5
    0b00010000, // row 6
    0b00100000, // row 7
    0b01000110, // row 8
    0b01000110, // row 9  (lower-right ring)
    0b00000000, // row 10
    0b00000000, // row 11
    0b00000000, // row 12
    0b00000000, // row 13
    0b00000000, // row 14
    0b00000000, // row 15
]];

/// Hand-authored glyph for '&'. Bowl with crossing right leg; tail clamps at
/// row 15.
const GLYPH_AMP: Glyph8x16 = [[
    0b00000000, // row 0
    0b00000000, // row 1
    0b00000000, // row 2
    0b00111000, // row 3
    0b01000100, // row 4
    0b01000100, // row 5
    0b00111000, // row 6
    0b01111110, // row 7  (crossing bar)
    0b01000000, // row 8
    0b01000100, // row 9
    0b00111010, // row 10
    0b01010100, // row 11 (leg starts)
    0b00101000, // row 12
    0b01010100, // row 13
    0b00101010, // row 14 (baseline: bowl foot + leg)
    0b10010100, // row 15 (leg tail clamped at row 15)
]];

/// Hand-authored glyph for '\'' (apostrophe). One short stroke.
const GLYPH_APOS: Glyph8x16 = [[
    0b00000000, // row 0
    0b00000000, // row 1
    0b00000000, // row 2
    0b00000000, // row 3
    0b00000000, // row 4
    0b00010000, // row 5
    0b00010000, // row 6
    0b00010000, // row 7
    0b00010000, // row 8
    0b00000000, // row 9
    0b00000000, // row 10
    0b00000000, // row 11
    0b00000000, // row 12
    0b00000000, // row 13
    0b00000000, // row 14
    0b00000000, // row 15
]];

/// Hand-authored glyph for '('.
const GLYPH_LPAREN: Glyph8x16 = [[
    0b00000000, // row 0
    0b00001000, // row 1
    0b00010000, // row 2
    0b00100000, // row 3
    0b00100000, // row 4
    0b00100000, // row 5
    0b00100000, // row 6
    0b00100000, // row 7
    0b00100000, // row 8
    0b00100000, // row 9
    0b00100000, // row 10
    0b00100000, // row 11
    0b00010000, // row 12
    0b00001000, // row 13
    0b00000000, // row 14
    0b00000000, // row 15
]];

/// Hand-authored glyph for ')'.
const GLYPH_RPAREN: Glyph8x16 = [[
    0b00000000, // row 0
    0b00010000, // row 1
    0b00001000, // row 2
    0b00000100, // row 3
    0b00000100, // row 4
    0b00000100, // row 5
    0b00000100, // row 6
    0b00000100, // row 7
    0b00000100, // row 8
    0b00000100, // row 9
    0b00000100, // row 10
    0b00000100, // row 11
    0b00001000, // row 12
    0b00010000, // row 13
    0b00000000, // row 14
    0b00000000, // row 15
]];

/// Hand-authored glyph for '*'. Six-arm asterisk centred at rows 5..9.
const GLYPH_STAR: Glyph8x16 = [[
    0b00000000, // row 0
    0b00000000, // row 1
    0b00000000, // row 2
    0b00000000, // row 3
    0b00010000, // row 4
    0b00111100, // row 5
    0b01011010, // row 6
    0b00011000, // row 7  (centre)
    0b01011010, // row 8
    0b00111100, // row 9
    0b00010000, // row 10
    0b00000000, // row 11
    0b00000000, // row 12
    0b00000000, // row 13
    0b00000000, // row 14
    0b00000000, // row 15
]];

/// Hand-authored glyph for '+'. Cross centred at row 8.
const GLYPH_PLUS: Glyph8x16 = [[
    0b00000000, // row 0
    0b00000000, // row 1
    0b00000000, // row 2
    0b00000000, // row 3
    0b00000000, // row 4
    0b00010000, // row 5
    0b00010000, // row 6
    0b00010000, // row 7
    0b00111100, // row 8  (horizontal)
    0b00010000, // row 9
    0b00010000, // row 10
    0b00010000, // row 11
    0b00000000, // row 12
    0b00000000, // row 13
    0b00000000, // row 14
    0b00000000, // row 15
]];

/// Hand-authored glyph for ','. Head on the baseline, tail clamps at row 15.
const GLYPH_COMMA: Glyph8x16 = [[
    0b00000000, // row 0
    0b00000000, // row 1
    0b00000000, // row 2
    0b00000000, // row 3
    0b00000000, // row 4
    0b00000000, // row 5
    0b00000000, // row 6
    0b00000000, // row 7
    0b00000000, // row 8
    0b00000000, // row 9
    0b00000000, // row 10
    0b00011000, // row 11
    0b00011000, // row 12
    0b00010000, // row 13
    0b00100000, // row 14 (baseline)
    0b01000000, // row 15 (tail clamped at row 15)
]];

/// Hand-authored glyph for '-'. One dash at mid-height.
const GLYPH_MINUS: Glyph8x16 = [[
    0b00000000, // row 0
    0b00000000, // row 1
    0b00000000, // row 2
    0b00000000, // row 3
    0b00000000, // row 4
    0b00000000, // row 5
    0b00000000, // row 6
    0b00000000, // row 7
    0b01111100, // row 8  (dash)
    0b00000000, // row 9
    0b00000000, // row 10
    0b00000000, // row 11
    0b00000000, // row 12
    0b00000000, // row 13
    0b00000000, // row 14
    0b00000000, // row 15
]];

/// Hand-authored glyph for '.'. One dot on the baseline.
const GLYPH_PERIOD: Glyph8x16 = [[
    0b00000000, // row 0
    0b00000000, // row 1
    0b00000000, // row 2
    0b00000000, // row 3
    0b00000000, // row 4
    0b00000000, // row 5
    0b00000000, // row 6
    0b00000000, // row 7
    0b00000000, // row 8
    0b00000000, // row 9
    0b00000000, // row 10
    0b00000000, // row 11
    0b00000000, // row 12
    0b00000000, // row 13
    0b00010000, // row 14 (baseline dot)
    0b00000000, // row 15
]];

/// Hand-authored glyph for '/'. Diagonal from lower-left to upper-right.
const GLYPH_SLASH: Glyph8x16 = [[
    0b00000000, // row 0
    0b00000010, // row 1
    0b00000010, // row 2
    0b00000100, // row 3
    0b00001000, // row 4
    0b00001000, // row 5
    0b00010000, // row 6
    0b00010000, // row 7
    0b00100000, // row 8
    0b00100000, // row 9
    0b01000000, // row 10
    0b01000000, // row 11
    0b10000000, // row 12
    0b10000000, // row 13
    0b00000000, // row 14
    0b00000000, // row 15
]];

/// Hand-authored glyph for ':'. Dots at x-height and baseline.
const GLYPH_COLON: Glyph8x16 = [[
    0b00000000, // row 0
    0b00000000, // row 1
    0b00000000, // row 2
    0b00000000, // row 3
    0b00000000, // row 4
    0b00010000, // row 5
    0b00000000, // row 6
    0b00000000, // row 7
    0b00000000, // row 8
    0b00000000, // row 9
    0b00000000, // row 10
    0b00000000, // row 11
    0b00000000, // row 12
    0b00000000, // row 13
    0b00010000, // row 14 (baseline dot)
    0b00000000, // row 15
]];

/// Hand-authored glyph for ';'. Dot at x-height, comma below; tail clamps at
/// row 15.
const GLYPH_SEMI: Glyph8x16 = [[
    0b00000000, // row 0
    0b00000000, // row 1
    0b00000000, // row 2
    0b00000000, // row 3
    0b00000000, // row 4
    0b00010000, // row 5  (upper dot)
    0b00000000, // row 6
    0b00000000, // row 7
    0b00000000, // row 8
    0b00000000, // row 9
    0b00011000, // row 10
    0b00011000, // row 11
    0b00010000, // row 12
    0b00100000, // row 13
    0b00100000, // row 14 (baseline)
    0b01000000, // row 15 (tail clamped at row 15)
]];

/// Hand-authored glyph for '<'. Chevron pointing left, centred at row 8.
const GLYPH_LT: Glyph8x16 = [[
    0b00000000, // row 0
    0b00000000, // row 1
    0b00000000, // row 2
    0b00000100, // row 3
    0b00001000, // row 4
    0b00010000, // row 5
    0b00100000, // row 6
    0b01000000, // row 7
    0b01000000, // row 8  (apex)
    0b00100000, // row 9
    0b00010000, // row 10
    0b00001000, // row 11
    0b00000100, // row 12
    0b00000000, // row 13
    0b00000000, // row 14
    0b00000000, // row 15
]];

/// Hand-authored glyph for '='. Two bars at rows 6 and 10 (mutation-gate
/// probe: blanking the FIRST set row — row 6 — turns glyph_equal_blit red).
const GLYPH_EQUAL: Glyph8x16 = [[
    0b00000000, // row 0
    0b00000000, // row 1
    0b00000000, // row 2
    0b00000000, // row 3
    0b00000000, // row 4
    0b00000000, // row 5
    0b01111110, // row 6  (upper bar — first set row, mutation-gate probe)
    0b00000000, // row 7
    0b00000000, // row 8
    0b00000000, // row 9
    0b01111110, // row 10 (lower bar)
    0b00000000, // row 11
    0b00000000, // row 12
    0b00000000, // row 13
    0b00000000, // row 14
    0b00000000, // row 15
]];

/// Hand-authored glyph for '>'. Chevron pointing right, centred at row 8.
const GLYPH_GT: Glyph8x16 = [[
    0b00000000, // row 0
    0b00000000, // row 1
    0b00000000, // row 2
    0b00100000, // row 3
    0b00010000, // row 4
    0b00001000, // row 5
    0b00000100, // row 6
    0b00000010, // row 7
    0b00000010, // row 8  (apex)
    0b00000100, // row 9
    0b00001000, // row 10
    0b00010000, // row 11
    0b00100000, // row 12
    0b00000000, // row 13
    0b00000000, // row 14
    0b00000000, // row 15
]];

/// Hand-authored glyph for '?'. Question curve with baseline dot.
const GLYPH_QUESTION: Glyph8x16 = [[
    0b00000000, // row 0
    0b00111100, // row 1
    0b01000010, // row 2
    0b01000010, // row 3
    0b00000100, // row 4
    0b00001000, // row 5
    0b00010000, // row 6
    0b00010000, // row 7
    0b00000000, // row 8
    0b00010000, // row 9
    0b00000000, // row 10
    0b00000000, // row 11
    0b00000000, // row 12
    0b00000000, // row 13
    0b00010000, // row 14 (baseline dot)
    0b00000000, // row 15
]];

/// Hand-authored glyph for '@'. Ring with inner hook; tail clamps at row 15.
const GLYPH_AT: Glyph8x16 = [[
    0b00000000, // row 0
    0b00000000, // row 1
    0b00111100, // row 2
    0b01000010, // row 3
    0b01000010, // row 4
    0b01011110, // row 5  (inner hook top)
    0b01010110, // row 6
    0b01010110, // row 7
    0b01010110, // row 8
    0b01000110, // row 9
    0b01000000, // row 10
    0b00100010, // row 11
    0b00011100, // row 12
    0b00000000, // row 13
    0b00000000, // row 14
    0b00000000, // row 15
]];

/// Hand-authored glyph for '['.
const GLYPH_LBRACK: Glyph8x16 = [[
    0b00000000, // row 0
    0b00011100, // row 1
    0b00010000, // row 2
    0b00010000, // row 3
    0b00010000, // row 4
    0b00010000, // row 5
    0b00010000, // row 6
    0b00010000, // row 7
    0b00010000, // row 8
    0b00010000, // row 9
    0b00010000, // row 10
    0b00010000, // row 11
    0b00010000, // row 12
    0b00010000, // row 13
    0b00011100, // row 14 (baseline)
    0b00000000, // row 15
]];

/// Hand-authored glyph for '\\' — mirror of '/'.
const GLYPH_BACKSLASH: Glyph8x16 = [[
    0b00000000, // row 0
    0b10000000, // row 1
    0b10000000, // row 2
    0b01000000, // row 3
    0b01000000, // row 4
    0b00100000, // row 5
    0b00100000, // row 6
    0b00010000, // row 7
    0b00010000, // row 8
    0b00001000, // row 9
    0b00001000, // row 10
    0b00000100, // row 11
    0b00000100, // row 12
    0b00000010, // row 13
    0b00000000, // row 14
    0b00000000, // row 15
]];

/// Hand-authored glyph for ']'.
const GLYPH_RBRACK: Glyph8x16 = [[
    0b00000000, // row 0
    0b00111000, // row 1
    0b00001000, // row 2
    0b00001000, // row 3
    0b00001000, // row 4
    0b00001000, // row 5
    0b00001000, // row 6
    0b00001000, // row 7
    0b00001000, // row 8
    0b00001000, // row 9
    0b00001000, // row 10
    0b00001000, // row 11
    0b00001000, // row 12
    0b00001000, // row 13
    0b00111000, // row 14 (baseline)
    0b00000000, // row 15
]];

/// Hand-authored glyph for '^'. Caret sitting MID (rows 4..8).
const GLYPH_CARET: Glyph8x16 = [[
    0b00000000, // row 0
    0b00000000, // row 1
    0b00000000, // row 2
    0b00000000, // row 3
    0b00010000, // row 4  (apex — mid band)
    0b00101000, // row 5
    0b01000100, // row 6
    0b01000100, // row 7
    0b00000000, // row 8
    0b00000000, // row 9
    0b00000000, // row 10
    0b00000000, // row 11
    0b00000000, // row 12
    0b00000000, // row 13
    0b00000000, // row 14
    0b00000000, // row 15
]];

/// Hand-authored glyph for '_'. Underscore LOW — one bar on row 13, just
/// above the baseline.
const GLYPH_UNDER: Glyph8x16 = [[
    0b00000000, // row 0
    0b00000000, // row 1
    0b00000000, // row 2
    0b00000000, // row 3
    0b00000000, // row 4
    0b00000000, // row 5
    0b00000000, // row 6
    0b00000000, // row 7
    0b00000000, // row 8
    0b00000000, // row 9
    0b00000000, // row 10
    0b00000000, // row 11
    0b00000000, // row 12
    0b11111111, // row 13 (low bar)
    0b00000000, // row 14
    0b00000000, // row 15
]];

/// Hand-authored glyph for '`'. Backtick — mirror of the apostrophe.
const GLYPH_BQUOTE: Glyph8x16 = [[
    0b00000000, // row 0
    0b00000000, // row 1
    0b00000000, // row 2
    0b00000000, // row 3
    0b00000000, // row 4
    0b00010000, // row 5
    0b00100000, // row 6
    0b00000000, // row 7
    0b00000000, // row 8
    0b00000000, // row 9
    0b00000000, // row 10
    0b00000000, // row 11
    0b00000000, // row 12
    0b00000000, // row 13
    0b00000000, // row 14
    0b00000000, // row 15
]];

/// Hand-authored glyph for '{'.
const GLYPH_LBRACE: Glyph8x16 = [[
    0b00000000, // row 0
    0b00001100, // row 1
    0b00010000, // row 2
    0b00010000, // row 3
    0b00010000, // row 4
    0b00010000, // row 5
    0b00100000, // row 6  (mid kink)
    0b00010000, // row 7
    0b00010000, // row 8
    0b00010000, // row 9
    0b00010000, // row 10
    0b00010000, // row 11
    0b00010000, // row 12
    0b00010000, // row 13
    0b00001100, // row 14 (baseline)
    0b00000000, // row 15
]];

/// Hand-authored glyph for '|'. Full-height stem.
const GLYPH_PIPE: Glyph8x16 = [[
    0b00000000, // row 0
    0b00010000, // row 1
    0b00010000, // row 2
    0b00010000, // row 3
    0b00010000, // row 4
    0b00010000, // row 5
    0b00010000, // row 6
    0b00010000, // row 7
    0b00010000, // row 8
    0b00010000, // row 9
    0b00010000, // row 10
    0b00010000, // row 11
    0b00010000, // row 12
    0b00010000, // row 13
    0b00010000, // row 14 (baseline)
    0b00000000, // row 15
]];

/// Hand-authored glyph for '}'.
const GLYPH_RBRACE: Glyph8x16 = [[
    0b00000000, // row 0
    0b00110000, // row 1
    0b00001000, // row 2
    0b00001000, // row 3
    0b00001000, // row 4
    0b00001000, // row 5
    0b00000100, // row 6  (mid kink)
    0b00001000, // row 7
    0b00001000, // row 8
    0b00001000, // row 9
    0b00001000, // row 10
    0b00001000, // row 11
    0b00001000, // row 12
    0b00001000, // row 13
    0b00110000, // row 14 (baseline)
    0b00000000, // row 15
]];

/// Hand-authored glyph for '~'. Tilde MID — zigzag on rows 8..10.
const GLYPH_TILDE: Glyph8x16 = [[
    0b00000000, // row 0
    0b00000000, // row 1
    0b00000000, // row 2
    0b00000000, // row 3
    0b00000000, // row 4
    0b00000000, // row 5
    0b00000000, // row 6
    0b00000000, // row 7
    0b00100010, // row 8  (peaks)
    0b01001100, // row 9  (troughs — mid band)
    0b01010000, // row 10 (valleys)
    0b00000000, // row 11
    0b00000000, // row 12
    0b00000000, // row 13
    0b00000000, // row 14
    0b00000000, // row 15
]];

/// The font table for part 3/3 (HW-10F3): punctuation + symbols — 32 entries.
/// Same linear-scan rationale as [`FONT_UPPER_DIGIT`]. With this table, every
/// printable ASCII char 0x20-0x7E has a hand-authored glyph.
const FONT_PUNCT: [(char, &Glyph8x16); 32] = [
    ('!', &GLYPH_EXCL),
    ('"', &GLYPH_QUOTE),
    ('#', &GLYPH_HASH),
    ('$', &GLYPH_DOLLAR),
    ('%', &GLYPH_PERCENT),
    ('&', &GLYPH_AMP),
    ('\'', &GLYPH_APOS),
    ('(', &GLYPH_LPAREN),
    (')', &GLYPH_RPAREN),
    ('*', &GLYPH_STAR),
    ('+', &GLYPH_PLUS),
    (',', &GLYPH_COMMA),
    ('-', &GLYPH_MINUS),
    ('.', &GLYPH_PERIOD),
    ('/', &GLYPH_SLASH),
    (':', &GLYPH_COLON),
    (';', &GLYPH_SEMI),
    ('<', &GLYPH_LT),
    ('=', &GLYPH_EQUAL),
    ('>', &GLYPH_GT),
    ('?', &GLYPH_QUESTION),
    ('@', &GLYPH_AT),
    ('[', &GLYPH_LBRACK),
    ('\\', &GLYPH_BACKSLASH),
    (']', &GLYPH_RBRACK),
    ('^', &GLYPH_CARET),
    ('_', &GLYPH_UNDER),
    ('`', &GLYPH_BQUOTE),
    ('{', &GLYPH_LBRACE),
    ('|', &GLYPH_PIPE),
    ('}', &GLYPH_RBRACE),
    ('~', &GLYPH_TILDE),
];

/// Look up the glyph for `ch`. Characters in the part-1/3 table (space, A-Z,
/// 0-9), the part-2/3 table (lowercase a-z) or the part-3/3 table (the 32
/// punctuation/symbol glyphs) get their hand-authored glyph — that is every
/// printable ASCII char 0x20-0x7E. Only chars OUTSIDE printable ASCII render
/// [`GLYPH_BOX`].
pub fn font_glyph(ch: char) -> &'static Glyph8x16 {
    for &(c, glyph) in FONT_UPPER_DIGIT.iter() {
        if c == ch {
            return glyph;
        }
    }
    for &(c, glyph) in FONT_LOWER.iter() {
        if c == ch {
            return glyph;
        }
    }
    for &(c, glyph) in FONT_PUNCT.iter() {
        if c == ch {
            return glyph;
        }
    }
    &GLYPH_BOX
}

/// Font cell metrics.
pub const GLYPH_W: usize = 8;
pub const GLYPH_H: usize = 16;

/// A writable framebuffer view: `pixels` is the raw buffer, addressed as
/// `pixels[y * stride + x]`. `stride` is in pixels per row and may exceed
/// `width` (padding). Pixels are X888 LE `u32` values `0x00RRGGBB`.
///
/// **Geometry invariant (HW-10B):** [`Frame::new`] is infallible and always
/// yields a *safe clipped* configuration. The effective drawing width is
/// `min(width, stride)` — a requested `width` larger than the `stride` can
/// never make a write address past the end of a row — and the effective
/// height is `min(height, pixels.len() / stride)` (0 when `stride == 0`).
/// Every primitive (`put_pixel`, `fill_rect`, `blit_glyph`, `scroll_up`)
/// derives its bounds from these clamped dimensions, so an inconsistent
/// geometry (width > stride, or dimensions the buffer cannot hold) is legal
/// and simply draws less — never a panic.
pub struct Frame<'a> {
    pub pixels: &'a mut [u32],
    pub width: usize,
    pub height: usize,
    pub stride: usize,
}

impl<'a> Frame<'a> {
    /// Wrap a raw buffer. See the [`Frame`] geometry invariant: `width` is
    /// clamped to `stride` and `height` to what the buffer can actually
    /// hold. Infallible; inconsistent inputs are a safe clipped frame.
    pub fn new(pixels: &'a mut [u32], width: usize, height: usize, stride: usize) -> Self {
        let width = core::cmp::min(width, stride);
        let height = if stride == 0 {
            0
        } else {
            core::cmp::min(height, pixels.len() / stride)
        };
        Self {
            pixels,
            width,
            height,
            stride,
        }
    }

    /// Write one pixel in X888 LE `0x00RRGGBB`. Out-of-bounds coordinates are
    /// **silently clipped**: the call is a no-op, never a panic. The index is
    /// computed with checked arithmetic, so even `x`/`y` near `usize::MAX`
    /// (which would overflow `y * stride + x`) are a no-op.
    pub fn put_pixel(&mut self, x: usize, y: usize, value: u32) {
        if x >= self.width || y >= self.height {
            return;
        }
        let idx = match y.checked_mul(self.stride).and_then(|o| o.checked_add(x)) {
            Some(idx) => idx,
            None => return,
        };
        if idx < self.pixels.len() {
            self.pixels[idx] = value;
        }
    }

    /// Fill the rectangle anchored at `(x, y)` of size `w x h` with `value`,
    /// clipped to the frame bounds. A rect entirely outside is a no-op.
    pub fn fill_rect(&mut self, x: usize, y: usize, w: usize, h: usize, value: u32) {
        let x_end = core::cmp::min(x.saturating_add(w), self.width);
        let y_end = core::cmp::min(y.saturating_add(h), self.height);
        if x >= x_end || y >= y_end {
            return;
        }
        for row in y..y_end {
            for col in x..x_end {
                let idx = row * self.stride + col;
                if idx < self.pixels.len() {
                    self.pixels[idx] = value;
                }
            }
        }
    }

    /// Copy the whole glyph bitmap of `glyph` with its top-left at pixel
    /// `(px, py)`: set bits draw `fg`, clear bits draw `bg`. Clipped: a
    /// glyph partly past an edge draws its visible part, and one entirely
    /// outside — including coordinates whose `px + gx` / `py + gy` would
    /// overflow — is a **no-op**, never a panic, for any `usize` inputs.
    pub fn blit_glyph(&mut self, px: usize, py: usize, glyph: &Glyph8x16, fg: u32, bg: u32) {
        for (gy, row_bits) in glyph[0].iter().enumerate() {
            for gx in 0..GLYPH_W {
                let bit = (row_bits >> (7 - gx)) & 1;
                let value = if bit == 1 { fg } else { bg };
                // Checked coordinate arithmetic: a glyph cell whose absolute
                // position overflows usize is outside the frame — clipped.
                let x = match px.checked_add(gx) {
                    Some(x) => x,
                    None => continue,
                };
                let y = match py.checked_add(gy) {
                    Some(y) => y,
                    None => continue,
                };
                self.put_pixel(x, y, value);
            }
        }
    }

    /// Move every row up by one font height (`GLYPH_H` pixel rows) and clear
    /// the last font row to `bg`. Rows are copied as full stride-width rows.
    /// Safe on any frame: `width` is already clamped to `stride` by
    /// [`Frame::new`] and `height` to the buffer, so the copy can never
    /// address past the buffer end — no panic, even for degenerate geometry.
    pub fn scroll_up(&mut self, bg: u32) {
        if self.height == 0 || self.stride == 0 {
            return;
        }
        let shift = core::cmp::min(GLYPH_H, self.height);
        // Copy rows up one at a time (top-down so each row moves up by shift).
        for row in 0..self.height {
            let src = row + shift;
            for col in 0..self.width {
                let value = if src < self.height {
                    self.pixels[src * self.stride + col]
                } else {
                    bg
                };
                self.pixels[row * self.stride + col] = value;
            }
        }
    }
}

/// A text console drawn over a [`Frame`] with an 8x16 cell grid.
pub struct Console<'a> {
    pub frame: Frame<'a>,
    pub fg: u32,
    pub bg: u32,
    pub col: usize,
    pub row: usize,
}

impl<'a> Console<'a> {
    pub fn new(frame: Frame<'a>, fg: u32, bg: u32) -> Self {
        Self {
            frame,
            fg,
            bg,
            col: 0,
            row: 0,
        }
    }

    /// Columns available: `width / GLYPH_W`. Zero-width frames give 0 cols.
    pub fn cols(&self) -> usize {
        self.frame.width / GLYPH_W
    }

    /// Rows available: `height / GLYPH_H`.
    pub fn rows(&self) -> usize {
        self.frame.height / GLYPH_H
    }

    /// Draw one character at the current cursor and advance it. `\n` moves to
    /// the next row; writing past the last column auto-wraps; running off the
    /// bottom scrolls the screen up. On zero-dim frames (0 cols or 0 rows)
    /// every write is a no-op — no panic, no scroll.
    pub fn write_char(&mut self, ch: char) {
        if self.cols() == 0 || self.rows() == 0 {
            return;
        }
        if ch == '\n' {
            self.col = 0;
            self.row += 1;
            if self.row >= self.rows() {
                self.row = self.rows() - 1;
                self.frame.scroll_up(self.bg);
            }
            return;
        }
        let px = self.col * GLYPH_W;
        let py = self.row * GLYPH_H;
        self.frame
            .blit_glyph(px, py, font_glyph(ch), self.fg, self.bg);
        self.col += 1;
        if self.col >= self.cols() {
            self.col = 0;
            self.row += 1;
            if self.row >= self.rows() {
                self.row = self.rows() - 1;
                self.frame.scroll_up(self.bg);
            }
        }
    }

    /// Write a string, character by character (see [`Console::write_char`]).
    pub fn write_str(&mut self, s: &str) {
        for ch in s.chars() {
            self.write_char(ch);
        }
    }
}
