use fb_write::{font_glyph, Frame};
#[test]
fn scroll_clips_width_larger_than_stride() {
    let mut pixels = [0u32; 4];
    let mut frame = Frame::new(&mut pixels, 3, 2, 2);
    frame.scroll_up(0);
}
#[test]
fn glyph_entirely_outside_frame_is_noop() {
    let mut pixels = [0u32; 4];
    let mut frame = Frame::new(&mut pixels, 2, 2, 2);
    frame.blit_glyph(usize::MAX, 0, font_glyph('A'), 1, 0);
    assert_eq!(pixels, [0; 4]);
}

// NOTE: body kept VERBATIM from FB-BOUNDS-ROOT-PROBE.rs (the root probe);
// rustfmt only normalizes whitespace/import order — no semantic change.
