//! The draw path: a [`Scene`] of positioned items onto any
//! `orbclient::Renderer` — software rendering, black + orange, nothing else.
//!
//! Generic over the renderer so the live SDL `Window` and the off-screen
//! `Surface` used by the golden-image tests run the *identical* code: the
//! screenshot is the app, not a second renderer that could drift from it.
//!
//! Palette discipline (VISION §8): five colours, no more. A heading is orange
//! and uppercase because unifont cannot scale; a chip's meaning is carried by
//! its words, not by a new colour.

use orbclient::{Color, Renderer};

use crate::scene::{ButtonVariant, Item, ItemKind, Rect, Scene, Style, Tone};

// — palette: black + orange + two grays + a track, nothing else —
pub const BG: Color = Color::rgb(0, 0, 0);
pub const ORANGE: Color = Color::rgb(255, 140, 0);
pub const TEXT: Color = Color::rgb(230, 230, 230);
pub const DIM: Color = Color::rgb(130, 130, 130);
pub const TRACK: Color = Color::rgb(45, 45, 45);

/// Unifont cell.
pub const CHAR_W: i32 = 8;
pub const LINE_H: i32 = 16;

/// Draw one line of text (unifont 8×16; orbclient clips at the edges).
pub fn draw_text<R: Renderer>(r: &mut R, x: i32, y: i32, s: &str, c: Color) {
    let mut x = x;
    for ch in s.chars() {
        r.char(x, y, ch, c);
        x += CHAR_W;
    }
}

/// A hollow rectangle `t` px thick.
fn border<R: Renderer>(r: &mut R, rect: Rect, t: i32, c: Color) {
    let Rect { x, y, w, h } = rect;
    r.rect(x, y, w as u32, t as u32, c);
    r.rect(x, y + h - t, w as u32, t as u32, c);
    r.rect(x, y, t as u32, h as u32, c);
    r.rect(x + w - t, y, t as u32, h as u32, c);
}

/// x that centres `s` inside `rect`.
fn center_x(rect: Rect, s: &str) -> i32 {
    rect.x + (rect.w - s.chars().count() as i32 * CHAR_W) / 2
}

/// Draw the whole scene: clear, then every item in order, then the overflow
/// marker if the content did not fit.
pub fn render<R: Renderer>(r: &mut R, scene: &Scene) {
    r.clear();
    for item in &scene.items {
        draw_item(r, item);
    }
    if scene.overflow {
        let y = scene.height - 20;
        r.rect(0, y, scene.width as u32, 20, BG);
        r.rect(0, y, scene.width as u32, 2, ORANGE);
        draw_text(r, center_x(Rect { x: 0, y, w: scene.width, h: 20 }, "MORE BELOW"), y + 4, "MORE BELOW", ORANGE);
    }
}

fn draw_item<R: Renderer>(r: &mut R, item: &Item) {
    match &item.kind {
        ItemKind::Text { lines, style } => {
            let color = match style {
                Style::Caption => DIM,
                Style::Body => TEXT,
                _ => ORANGE,
            };
            for (i, line) in lines.iter().enumerate() {
                draw_text(r, item.rect.x, item.rect.y + i as i32 * LINE_H, line, color);
            }
            if style.rule() {
                let y = item.rect.y + lines.len() as i32 * LINE_H + 2;
                r.rect(item.rect.x, y, item.rect.w as u32, 2, ORANGE);
            }
        }
        ItemKind::Button { label, variant, action, .. } => {
            let inert = matches!(action, crate::scene::Action::None);
            match (variant, inert) {
                (ButtonVariant::Primary, false) => {
                    r.rect(item.rect.x, item.rect.y, item.rect.w as u32, item.rect.h as u32, ORANGE);
                    draw_text(r, center_x(item.rect, label), item.rect.y + (item.rect.h - LINE_H) / 2, label, BG);
                }
                (ButtonVariant::Primary, true) => {
                    // A button with no action must not look tappable.
                    r.rect(item.rect.x, item.rect.y, item.rect.w as u32, item.rect.h as u32, TRACK);
                    draw_text(r, center_x(item.rect, label), item.rect.y + (item.rect.h - LINE_H) / 2, label, DIM);
                }
                (ButtonVariant::Borderless, _) => {
                    let c = if inert { DIM } else { ORANGE };
                    draw_text(r, item.rect.x + MARGIN_X, item.rect.y + (item.rect.h - LINE_H) / 2, label, c);
                }
                (ButtonVariant::Default, _) => {
                    r.rect(item.rect.x, item.rect.y, item.rect.w as u32, item.rect.h as u32, TRACK);
                    border(r, item.rect, 2, if inert { DIM } else { ORANGE });
                    draw_text(r, center_x(item.rect, label), item.rect.y + (item.rect.h - LINE_H) / 2, label, if inert { DIM } else { TEXT });
                }
            }
        }
        ItemKind::Field { label, value, focused, .. } => {
            let box_y = item.rect.y + LINE_H + 4;
            let box_h = item.rect.h - LINE_H - 4;
            let field = Rect { x: item.rect.x, y: box_y, w: item.rect.w, h: box_h };
            r.rect(field.x, field.y, field.w as u32, field.h as u32, TRACK);
            border(r, field, if *focused { 2 } else { 1 }, if *focused { ORANGE } else { DIM });
            draw_text(r, item.rect.x, item.rect.y, label, if *focused { ORANGE } else { DIM });
            let ty = field.y + (field.h - LINE_H) / 2;
            draw_text(r, field.x + MARGIN_X, ty, value, if value.is_empty() { DIM } else { TEXT });
            if *focused {
                // A caret: the keyboard is going here, so say where.
                let cx = field.x + MARGIN_X + value.chars().count() as i32 * CHAR_W + 2;
                r.rect(cx, ty, 2, LINE_H as u32, ORANGE);
            }
        }
        ItemKind::Progress { fraction, readout } => {
            r.rect(item.rect.x, item.rect.y, item.rect.w as u32, item.rect.h as u32, TRACK);
            match fraction {
                Some(f) => {
                    let fill = (item.rect.w as f64 * f).round() as i32;
                    if fill > 0 {
                        r.rect(item.rect.x, item.rect.y, fill as u32, item.rect.h as u32, ORANGE);
                    }
                }
                None => {
                    // Indeterminate: three blocks, evenly spread.
                    let seg = (item.rect.w / 6).max(1);
                    for k in 0..3 {
                        let x = item.rect.x + (item.rect.w / 3) * k + (item.rect.w / 3 - seg) / 2;
                        r.rect(x, item.rect.y, seg as u32, item.rect.h as u32, ORANGE);
                    }
                }
            }
            if !readout.is_empty() {
                let tx = item.rect.x + item.rect.w - readout.chars().count() as i32 * CHAR_W - 4;
                let ty = item.rect.y + (item.rect.h - LINE_H) / 2;
                // Over the orange fill the readout has to be dark to read.
                let over_fill = fraction.is_some_and(|f| (item.rect.w as f64 * f) as i32 > tx - item.rect.x + 4);
                draw_text(r, tx, ty, readout, if over_fill { BG } else { TEXT });
            }
        }
        ItemKind::Image { .. } => {
            let Rect { x, y, w, h } = item.rect;
            r.rect(x, y, w as u32, h as u32, TRACK);
            border(r, item.rect, 1, DIM);
            // A placeholder glyph: sun plus a mountain, in DIM.
            r.rect(x + w / 5, y + h / 5, 10, 10, DIM);
            let base = y + h - 12;
            for i in 0..4i32 {
                let mw = 16 + i * 24;
                r.rect(x + (w - mw) / 2, base + i * 3, mw as u32, 3, DIM);
            }
        }
        ItemKind::Divider => {
            r.rect(item.rect.x, item.rect.y, item.rect.w as u32, 1, DIM);
        }
        ItemKind::Chip { label, tone } => {
            let c = match tone {
                Tone::Waiting => DIM,
                Tone::Unsupported | Tone::Cycle | Tone::Broken => ORANGE,
            };
            border(r, item.rect, 1, c);
            draw_text(r, item.rect.x + MARGIN_X, item.rect.y + (item.rect.h - LINE_H) / 2, label, c);
        }
        ItemKind::Frame => {
            border(r, item.rect, 1, DIM);
        }
    }
}

/// Horizontal inset used inside buttons, fields and chips.
const MARGIN_X: i32 = 8;
