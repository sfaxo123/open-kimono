//! HW-10S acceptance tests for fb-write (6 tests, Vec<u32> buffers).

use fb_write::{Console, Frame, GLYPH_H, GLYPH_W};

const FG: u32 = 0x00FFFFFF;
const BG: u32 = 0x00000000;

/// 'A' at (0,0): pixels derived from the hand-authored GLYPH_A table in
/// src/lib.rs. Rows 0 and 1 of GLYPH_A are 0b00011000 -> cols 3 and 4 set
/// (the mutation gate blanks row 0; both rows carry the same assertion).
/// Row 7 is 0b01111110 -> crossbar, col 3 set. Row 9 is 0b10000001 -> the
/// outer legs (col 0) are set while col 3 is CLEAR.
#[test]
fn glyph_A_blit() {
    let mut buf = vec![BG; 80 * 32];
    let frame = Frame::new(&mut buf, 80, 32, 80);
    let mut con = Console::new(frame, FG, BG);
    con.write_char('A');

    // Known-SET pixels (from GLYPH_A rows 0, 1, 7 and 9):
    assert_eq!(buf[0 * 80 + 3], FG, "GLYPH_A row0 bit3 must be set");
    assert_eq!(buf[1 * 80 + 3], FG, "GLYPH_A row1 bit3 must be set");
    assert_eq!(buf[1 * 80 + 4], FG, "GLYPH_A row1 bit4 must be set");
    assert_eq!(buf[1 * 80 + 4], FG, "GLYPH_A row1 bit4 must be set");
    assert_eq!(buf[7 * 80 + 3], FG, "GLYPH_A row7 crossbar must be set");
    // Known-CLEAR pixel inside the glyph box (GLYPH_A row 9 = 0b10000001,
    // bit for col 3 is 0):
    assert_eq!(buf[9 * 80 + 3], BG, "GLYPH_A row9 col3 must be clear");
}

/// '~' had no hand-authored glyph when this test was written (HW-10S) ->
/// hollow-box fallback. Box row 0 is 0b00111100 (border) and rows 1..14 are
/// 0b01000010 with a clear interior.
/// (HW-10F3 amended this test: '~' now has a real tilde glyph, so the
/// box-fallback probe char switched from '~' to TAB 0x09 — outside printable
/// ASCII, where the box fallback still applies.)
#[test]
fn box_fallback() {
    let mut buf = vec![BG; 80 * 32];
    let frame = Frame::new(&mut buf, 80, 32, 80);
    let mut con = Console::new(frame, FG, BG);
    con.write_char('\t');

    // Border pixel: box row 1 bit6 (0b01000010) -> col 1 set.
    assert_eq!(buf[1 * 80 + 1], FG, "box border col1 must be set");
    // Interior pixel: same row, col 3 is clear in the box glyph.
    assert_eq!(buf[1 * 80 + 3], BG, "box interior col3 must be clear");
}

/// Writing more than `cols` characters wraps onto row 1.
#[test]
fn wrap() {
    // 64 px wide -> cols = 64/8 = 8; 32 px tall -> rows = 2.
    let mut buf = vec![BG; 64 * 32];
    let frame = Frame::new(&mut buf, 64, 32, 64);
    let mut con = Console::new(frame, FG, BG);
    assert_eq!(con.cols(), 8);

    // 9 'A's: the 9th must land on row 1 (pixel y offset 16).
    con.write_str("AAAAAAAAA");
    assert_eq!(con.row, 1, "cursor must have wrapped to row 1");
    // Row-1 glyph at col 0: GLYPH_A row 1 sets cols 3-4 -> buffer index
    // (16+1)*64 + 3 must be foreground.
    assert_eq!(buf[(GLYPH_H + 1) * 64 + 3], FG, "9th 'A' drawn on row 1");
    // Row-0 glyph at col 7 (8th 'A'): still drawn.
    assert_eq!(buf[1 * 64 + 7 * GLYPH_W + 3], FG, "8th 'A' drawn on row 0");
}

/// A marker pixel one font-row down moves up by exactly GLYPH_H rows after
/// `scroll_up`, and the vacated last row is cleared to bg.
#[test]
fn scroll() {
    let mut buf = vec![BG; 80 * 48];
    let mut frame = Frame::new(&mut buf, 80, 48, 80);
    // Marker at pixel row 16 (start of font row 1).
    frame.put_pixel(5, 16, FG);
    frame.scroll_up(BG);

    // Marker moved up one font height: row 16 -> row 0.
    assert_eq!(buf[0 * 80 + 5], FG, "marker must move up by GLYPH_H rows");
    assert_eq!(buf[16 * 80 + 5], BG, "old marker position cleared");
    // The last font row (rows 32..47) is cleared to bg.
    assert_eq!(buf[40 * 80 + 5], BG, "last row cleared to bg");
}

/// put_pixel past the edge is silently clipped: no panic, buffer unchanged.
#[test]
fn bounds() {
    let mut buf = vec![BG; 16 * 16];
    let mut frame = Frame::new(&mut buf, 16, 16, 16);
    frame.put_pixel(16, 0, FG); // x past width
    frame.put_pixel(0, 16, FG); // y past height
    frame.put_pixel(9999, 9999, FG);
    frame.fill_rect(10, 10, 100, 100, FG); // clipped fill: only 6x6 lands
    for (i, v) in buf.iter().enumerate() {
        let x = i % 16;
        let y = i / 16;
        if x >= 10 && y >= 10 {
            assert_eq!(*v, FG, "clipped fill must land at ({x},{y})");
        } else {
            assert_eq!(*v, BG, "out-of-bounds writes must not touch ({x},{y})");
        }
    }

    // Zero-dim frame: every write is a no-op, no panic.
    let mut zbuf = vec![];
    let zframe = Frame::new(&mut zbuf, 0, 0, 0);
    let mut zcon = Console::new(zframe, FG, BG);
    zcon.write_str("ABC\n");
}

/// fg 0x00FF0000 (pure red) stored as the raw u32 == 0x00FF0000 — the X888 LE
/// numeric form documented in lib.rs.
#[test]
fn byte_order() {
    let mut buf = vec![0u32; 4];
    let mut frame = Frame::new(&mut buf, 2, 2, 2);
    frame.put_pixel(1, 1, 0x00FF0000);
    assert_eq!(buf[3], 0x00FF0000u32, "raw u32 must equal 0x00RRGGBB");
}

/// HW-10F1 (font part 1/3): every char in A-Z, 0-9 and space must resolve to
/// a hand-authored glyph (one distinct bitmap per char, none equal to the
/// fallback box); chars outside the table must still resolve to the hollow-box
/// fallback.
/// (HW-10F2 amended this test: 'z' now has a hand-authored lowercase glyph,
/// so the unauthored probe char switched from 'z' to '{'.
/// HW-10F3 amended it again: every printable ASCII char is now authored, so
/// the box-fallback probes switched to chars outside printable ASCII — TAB
/// 0x09 and CR 0x0D.)
#[test]
fn font_table_complete() {
    let expected = "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789 ";
    let mut distinct = std::collections::HashSet::new();
    for ch in expected.chars() {
        let g = fb_write::font_glyph(ch);
        assert_ne!(
            g as *const fb_write::Glyph8x16,
            fb_write::font_glyph('\t') as *const fb_write::Glyph8x16,
            "char {ch:?} must have a hand-authored glyph, not the box"
        );
        distinct.insert(g as *const fb_write::Glyph8x16);
    }
    assert_eq!(
        distinct.len(),
        expected.chars().count(),
        "all 37 table chars must map to distinct glyphs"
    );

    // Chars outside printable ASCII fall back to the box: TAB and CR return
    // the same bitmap, and its border pixel (box row 1 = 0b01000010 -> col 1)
    // is set while its interior (col 3) is clear.
    let tab = fb_write::font_glyph('\t');
    let cr = fb_write::font_glyph('\r');
    assert_eq!(tab, cr, "TAB and CR must both return the fallback box");
    assert_eq!(tab[0][1] & 0b01000010, 0b01000010, "box border bits set");
    assert_eq!(tab[0][1] & 0b00001000, 0, "box interior clear");
}

/// 'C' at console cell (0,0): pixels derived from GLYPH_C in src/lib.rs.
/// Row 0 = 0b00011110 -> cols 3,4,5,6 set (col 3 is the mutation-gate probe:
/// blanking row 0 of the C bitmap turns this test red).
/// Rows 2..12 = 0b01000000 -> col 1 set (left stem).
/// Known-CLEAR: row 5 col 5 (0b01000000 has bit 5 clear) and row 1 col 4
/// (0b00100001 has bit 4 clear).
#[test]
fn glyph_C_blit() {
    let mut buf = vec![BG; 80 * 32];
    let frame = Frame::new(&mut buf, 80, 32, 80);
    let mut con = Console::new(frame, FG, BG);
    con.write_char('C');

    // Known-SET pixels:
    assert_eq!(buf[0 * 80 + 3], FG, "GLYPH_C row0 col3 must be set");
    assert_eq!(buf[0 * 80 + 6], FG, "GLYPH_C row0 col6 must be set");
    assert_eq!(buf[7 * 80 + 1], FG, "GLYPH_C row7 col1 (stem) must be set");
    assert_eq!(
        buf[14 * 80 + 3],
        FG,
        "GLYPH_C row14 col3 (baseline) must be set"
    );
    // Known-CLEAR pixels:
    assert_eq!(buf[5 * 80 + 5], BG, "GLYPH_C row5 col5 must be clear");
    assert_eq!(buf[1 * 80 + 4], BG, "GLYPH_C row1 col4 must be clear");
}

/// HW-10B: hammer a 4-pixel frame with extreme/overflowing coordinates and
/// inconsistent geometry. Nothing may panic, and every call documented as a
/// clipped no-op must leave the buffer untouched.
#[test]
fn geometry_extremes_no_panic() {
    let mut buf = [BG; 4];

    // width > stride: safe clipped configuration (effective width = stride).
    let mut frame = Frame::new(&mut buf, 3, 2, 2);
    frame.scroll_up(BG);
    frame.scroll_up(FG);
    frame.fill_rect(0, 0, usize::MAX, usize::MAX, BG);
    assert_eq!(
        frame.pixels, [BG; 4],
        "zero-fill of the clipped frame keeps it clear"
    );

    // Glyph rects that overflow or start outside: clipped no-ops.
    frame.blit_glyph(usize::MAX, 0, fb_write::font_glyph('A'), FG, BG);
    frame.blit_glyph(0, usize::MAX, fb_write::font_glyph('A'), FG, BG);
    frame.blit_glyph(usize::MAX, usize::MAX, fb_write::font_glyph('A'), FG, BG);
    frame.blit_glyph(usize::MAX - 3, 0, fb_write::font_glyph('A'), FG, BG);
    frame.blit_glyph(9999, 9999, fb_write::font_glyph('~'), FG, FG);
    assert_eq!(
        frame.pixels, [BG; 4],
        "out-of-frame / overflowing glyphs must not draw"
    );

    // Extreme rects and pixels: no panic; far-out writes must not touch.
    frame.fill_rect(usize::MAX, 0, 1, 1, FG);
    frame.fill_rect(0, usize::MAX, 1, 1, FG);
    frame.fill_rect(usize::MAX, usize::MAX, usize::MAX, usize::MAX, FG);
    frame.put_pixel(usize::MAX, usize::MAX, FG);
    frame.put_pixel(usize::MAX, 0, FG);
    frame.put_pixel(0, usize::MAX, FG);
    assert_eq!(
        frame.pixels, [BG; 4],
        "extreme coordinates must be clipped no-ops"
    );

    // Extreme geometry variants: infallible constructors, safe no-op draws.
    let mut b2 = [BG; 4];
    let mut f2 = Frame::new(&mut b2, usize::MAX, usize::MAX, usize::MAX);
    f2.scroll_up(BG);
    f2.blit_glyph(usize::MAX, 0, fb_write::font_glyph('A'), FG, BG);
    f2.fill_rect(usize::MAX, usize::MAX, usize::MAX, usize::MAX, FG);
    assert_eq!(
        b2, [BG; 4],
        "MAX-stride frame (0x0 effective) draws nothing"
    );

    let mut b3 = [BG; 4];
    let mut f3 = Frame::new(&mut b3, usize::MAX, usize::MAX, 2);
    let mut con = Console::new(f3, FG, BG);
    con.write_str("AAAAAAAAAAAAAAAAAAAA");
    con.write_char('\n');
    con.write_str("A");
    // Console draws are legal on this clipped frame; whatever landed must be
    // inside the buffer — the point is that nothing panicked.

    let mut b4 = [BG; 4];
    let mut f4 = Frame::new(&mut b4, 2, 2, 0);
    f4.scroll_up(FG);
    f4.blit_glyph(0, 0, fb_write::font_glyph('A'), FG, BG);
    assert_eq!(b4, [BG; 4], "zero-stride frame must draw nothing");
}

/// HW-10F2 (font part 2/3): every lowercase a-z must resolve to a distinct
/// hand-authored glyph (none equal to the fallback box); chars outside the
/// tables must still resolve to the hollow-box fallback.
/// (HW-10F3 amended this test: '~' and '{' now have hand-authored glyphs, so
/// the box-fallback probes switched to chars outside printable ASCII — TAB
/// 0x09 and CR 0x0D.)
#[test]
fn font_lower_complete() {
    let expected = "abcdefghijklmnopqrstuvwxyz";
    let mut distinct = std::collections::HashSet::new();
    for ch in expected.chars() {
        let g = fb_write::font_glyph(ch);
        assert_ne!(
            g as *const fb_write::Glyph8x16,
            fb_write::font_glyph('\t') as *const fb_write::Glyph8x16,
            "char {ch:?} must have a hand-authored glyph, not the box"
        );
        distinct.insert(g as *const fb_write::Glyph8x16);
    }
    assert_eq!(
        distinct.len(),
        expected.chars().count(),
        "all 26 lowercase chars must map to distinct glyphs"
    );

    // Chars outside every table fall back to the box: TAB and CR return the
    // same bitmap, with its border bit set (box row 1 = 0b01000010 -> col 1)
    // and interior clear (col 3).
    let tab = fb_write::font_glyph('\t');
    let cr = fb_write::font_glyph('\r');
    assert_eq!(tab, cr, "TAB and CR must both return the fallback box");
    assert_eq!(tab[0][1] & 0b01000010, 0b01000010, "box border bits set");
    assert_eq!(tab[0][1] & 0b00001000, 0, "box interior clear");
}

/// 'g' at console cell (0,0): pixels derived from GLYPH_G_LOWER in src/lib.rs.
/// Row 0 = 0b00001100 -> cols 4,5 set (the mutation-gate probe row: blanking
/// row 0 of the g bitmap turns this first assertion red — g carries a small
/// ascender flag on rows 0-1 above its x-height bowl).
/// Row 8 = 0b01000010 -> cols 1 and 6 set (left/right stems of the bowl).
/// Row 13 = 0b00000100 -> col 5 set (descender hook start; the tail
/// continues on rows 14-15, clamped at row 15).
#[test]
fn glyph_g_blit() {
    let mut buf = vec![BG; 80 * 32];
    let frame = Frame::new(&mut buf, 80, 32, 80);
    let mut con = Console::new(frame, FG, BG);
    con.write_char('g');

    // Known-SET pixels (3):
    assert_eq!(buf[0 * 80 + 4], FG, "GLYPH_G_LOWER row0 col4 must be set");
    assert_eq!(buf[8 * 80 + 1], FG, "GLYPH_G_LOWER row8 col1 must be set");
    assert_eq!(buf[13 * 80 + 5], FG, "GLYPH_G_LOWER row13 col5 must be set");
    // Known-CLEAR pixel (1):
    assert_eq!(buf[8 * 80 + 3], BG, "GLYPH_G_LOWER row8 col3 must be clear");
}

/// HW-10F3 (font part 3/3): after the punctuation/symbol table, EVERY
/// printable ASCII char 0x20-0x7E must resolve to a hand-authored glyph —
/// none may equal the fallback box. Chars OUTSIDE printable ASCII — 0x1F
/// (unit separator) and 0x7F (DEL) — must still resolve to the box, whose
/// border bit is set and interior clear.
#[test]
fn font_ascii_complete() {
    let box_ptr = fb_write::font_glyph('\u{1F}') as *const fb_write::Glyph8x16;
    for byte in 0x20u8..=0x7E {
        let ch = char::from(byte);
        let g = fb_write::font_glyph(ch);
        assert_ne!(
            g as *const fb_write::Glyph8x16, box_ptr,
            "char {:?} (0x{:02X}) must have a hand-authored glyph, not the box",
            ch, byte
        );
    }
    // Outside printable ASCII: the box fallback remains.
    for byte in [0x1Fu8, 0x7F] {
        let g = fb_write::font_glyph(char::from(byte));
        assert_eq!(
            g as *const fb_write::Glyph8x16, box_ptr,
            "0x{:02X} is outside printable ASCII and must render the box",
            byte
        );
    }
    // Sanity: the box itself has its border bit set (box row 1 = 0b01000010
    // -> col 1) and interior clear (col 3).
    let bx = fb_write::font_glyph('\u{7F}');
    assert_eq!(bx[0][1] & 0b01000010, 0b01000010, "box border bits set");
    assert_eq!(bx[0][1] & 0b00001000, 0, "box interior clear");
}

/// '=' at console cell (0,0): pixels derived from GLYPH_EQUAL in src/lib.rs.
/// The glyph is two bars: row 6 = 0b01111110 (upper bar — the FIRST set row,
/// mutation-gate probe) and row 10 = 0b01111110 (lower bar).
/// Known-SET coords: (row 6, col 1), (row 6, col 3), (row 10, col 6) — bars
/// span cols 1..7. Known-CLEAR coord: (row 8, col 3) — the gap between the
/// two bars is empty.
#[test]
fn glyph_equal_blit() {
    let mut buf = vec![BG; 80 * 32];
    let frame = Frame::new(&mut buf, 80, 32, 80);
    let mut con = Console::new(frame, FG, BG);
    con.write_char('=');

    // Known-SET pixels (3): upper bar row 6 cols 1 and 3; lower bar row 10 col 6.
    assert_eq!(buf[6 * 80 + 1], FG, "GLYPH_EQUAL row6 col1 must be set");
    assert_eq!(buf[6 * 80 + 3], FG, "GLYPH_EQUAL row6 col3 must be set");
    assert_eq!(buf[10 * 80 + 6], FG, "GLYPH_EQUAL row10 col6 must be set");
    // Known-CLEAR pixel (1): gap between the bars.
    assert_eq!(buf[8 * 80 + 3], BG, "GLYPH_EQUAL row8 col3 must be clear");
}

// ---------------------------------------------------------------------------
// HW-11: deterministic console banner evidence + pinned digest.
// ---------------------------------------------------------------------------

/// Hand-written SHA-256 must match the FIPS 180-4 test vectors.
#[test]
fn sha256_known_vectors() {
    fn hex(d: &[u8; 32]) -> String {
        d.iter().map(|b| format!("{:02x}", b)).collect()
    }
    // FIPS vector: empty string.
    assert_eq!(
        hex(&fb_write::evidence::sha256(b"")),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    // FIPS vector: "abc".
    assert_eq!(
        hex(&fb_write::evidence::sha256(b"abc")),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

/// The 320x128 banner render must hash to the pinned golden digest, and two
/// renders must be identical (determinism). Golden pin recorded ONCE from a
/// real run of frame_digest(320,128) — see HW-11 receipt.
#[test]
fn banner_digest_pinned() {
    fn hex(d: &[u8; 32]) -> String {
        d.iter().map(|b| format!("{:02x}", b)).collect()
    }
    let digest = fb_write::evidence::frame_digest(320, 128);
    assert_eq!(
        hex(&digest),
        "b2115ecb68fcee13a9055b0adac4717e3ef768c52243ee7a3f0739f240df69d2"
    );
    // Determinism: rendering twice yields identical digests.
    let again = fb_write::evidence::frame_digest(320, 128);
    assert_eq!(digest, again);
}
