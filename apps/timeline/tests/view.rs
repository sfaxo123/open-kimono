//! Renderer tests: wrapping, and a real off-screen render proving the
//! black+orange frame actually lands in the pixel buffer.

use orbclient::{Color, Renderer, Surface};

use timeline::model::{ServiceEntry, TimelineModel, ToolCall, ToolState};
use timeline::view::{render, tail_lines, wrap, BG, ORANGE, TRACK};

fn data(c: Color) -> u32 {
    c.data
}

#[test]
fn wrap_respects_columns() {
    let lines = wrap("the quick brown fox jumps over the lazy dog", 10);
    assert!(lines.iter().all(|l| l.chars().count() <= 10), "{lines:?}");
    assert_eq!(lines[0], "the quick");
    // Round-trip: same words, same order.
    let joined: Vec<&str> = lines.iter().flat_map(|l| l.split_whitespace()).collect();
    assert_eq!(joined, "the quick brown fox jumps over the lazy dog".split_whitespace().collect::<Vec<_>>());
}

#[test]
fn wrap_hard_breaks_long_words() {
    let lines = wrap("supercalifragilistic", 5);
    assert_eq!(lines.len(), 4);
    assert_eq!(lines.concat(), "supercalifragilistic");
}

#[test]
fn wrap_is_char_safe_for_multibyte() {
    let lines = wrap("πéπé πéπé πéπé", 4);
    assert!(lines.iter().all(|l| l.chars().count() <= 4), "{lines:?}");
}

#[test]
fn tail_lines_keeps_the_newest() {
    let s = "a b c d e f g h i j".repeat(10); // long reasoning
    let t = tail_lines(&s, 20, 3);
    assert_eq!(t.len(), 3);
    assert!(t.last().unwrap().ends_with("j"));
}

#[test]
fn render_paints_mic_bar_and_labels_in_orange_on_black() {
    let mut m = TimelineModel::default();
    m.mic_live = true;
    m.mic_level = 0.5;
    m.transcript = "navigate me home".into();
    m.cleaned = Some("navigate to home".into());
    m.reasoning = "offline maps cover this area".into();
    m.tools.push(ToolCall {
        id: "t1".into(),
        name: "navigate_map".into(),
        args: "{\"to\":\"home\"}".into(),
        state: ToolState::Result,
        result: Some("{\"eta_min\":12}".into()),
    });
    m.services.push(ServiceEntry { name: "maps".into(), generation: 1, running: true, reason: "nav".into() });

    let mut s = Surface::new(480, 800).expect("off-screen surface");
    s.clear();
    render(&mut s, &m);

    let px = s.data();
    // Background stays black in the corners.
    assert_eq!(data(px[0]), data(orbclient::Color::BLACK), "top-left corner is black");
    assert_eq!(data(px[px.len() - 1]), data(orbclient::Color::BLACK), "bottom-right corner is black");

    // The mic level bar row (y=11, x in 58..128 for level 0.5) is orange.
    let w = s.width() as usize;
    let bar = px[11 * w + 60];
    assert_eq!(data(bar), data(ORANGE), "mic bar fill is orange at 50%");
    let beyond = px[11 * w + 140];
    assert_eq!(data(beyond), data(TRACK), "past the fill is the dark track");

    // Orange section labels exist somewhere (count orange pixels overall).
    let orange = px.iter().filter(|c| data(**c) == data(ORANGE)).count();
    assert!(orange > 500, "labels + bar + dot should paint plenty of orange, got {orange}");
}

#[test]
fn render_idle_screen_has_no_orange_bar() {
    let m = TimelineModel::default();
    let mut s = Surface::new(480, 800).unwrap();
    render(&mut s, &m);
    let w = s.width() as usize;
    let bar = s.data()[11 * w + 60];
    assert_eq!(data(bar), data(TRACK), "no fill when the level is 0");
}

/// Count orange pixels in the text cell starting at row `y0` (unifont glyphs
/// don't fill every row of the 16-px cell, so scan the whole cell).
fn cell_orange(s: &Surface, y0: usize) -> usize {
    let w = s.width() as usize;
    s.data()[y0 * w..(y0 + 16) * w]
        .iter()
        .filter(|c| data(**c) == data(ORANGE))
        .count()
}

#[test]
fn render_shows_dropped_records_below_the_error_row() {
    // Honesty: a journal gap must be visible on screen, in the honesty band.
    //
    // Why not fixed pixel cells: the honesty band is conditional (MODEL,
    // ERROR, DROPPED rows each add 16 px), so every section label below it
    // shifts with model state — on the idle screen the orange "YOU" label
    // lands exactly where the DROPPED row lands with drops. Any absolute y is
    // a control that overlaps another label. (Three earlier versions failed
    // exactly this way.)
    //
    // The robust control is the *same y-band, both renders*: the band starts
    // at the first orange pixel below the mic row, which is the DROPPED row
    // when drops exist and the "YOU" label when they don't. The DROPPED row
    // must paint orange text there; the idle screen must have no orange text
    // at the left margin in that band (its "YOU" label is the only thing
    // there, and it is one short label — the DROPPED row is a longer line).
    let m = TimelineModel {
        dropped: 7,
        dropped_reason: Some("ring overflow".into()),
        ..TimelineModel::default()
    };

    let mut s = Surface::new(480, 800).unwrap();
    render(&mut s, &m);
    let mut s2 = Surface::new(480, 800).unwrap();
    render(&mut s2, &TimelineModel::default());

    // First orange pixel below the mic row (y >= 24), scanning row by row.
    let w = s.width() as usize;
    let first_orange_row = |s: &Surface| -> usize {
        (24..s.height() as usize)
            .find(|&y| {
                s.data()[y * w..(y + 1) * w].iter().any(|c| data(*c) == data(ORANGE))
            })
            .expect("some orange below the mic row")
    };
    let y_drops = first_orange_row(&s);
    let y_idle = first_orange_row(&s2);

    // With drops, the first orange band below the mic row is the DROPPED row:
    // a full line of orange text at the left margin (26+ chars → >100 px wide).
    assert!(cell_orange(&s, y_drops) > 100, "DROPPED row paints a full orange line at y={y_drops}");
    // The idle screen's first orange band is the short "YOU" label (3 glyphs).
    assert!(cell_orange(&s2, y_idle) < 100, "idle band at y={y_idle} is only the short YOU label");

    // And the DROPPED row's text is literally there: the row starts with the
    // word DROPPED at the left margin, so its first 8 glyph cells (64 px) are
    // non-black on the drops screen.
    let row_nonblack = |s: &Surface, y0: usize| {
        s.data()[y0 * w..(y0 + 16) * w]
            .chunks(w)
            .flat_map(|row| row[..8 * 8].iter())
            .filter(|c| data(**c) != data(BG))
            .count()
    };
    assert!(row_nonblack(&s, y_drops) > 100, "the DROPPED row paints glyphs at the left margin");
}
