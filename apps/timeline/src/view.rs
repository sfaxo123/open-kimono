//! The one screen: black + orange, six elements, no clutter (VISION §8).
//!
//! Generic over `orbclient::Renderer`, so the identical draw path feeds both
//! the live SDL `Window` and the off-screen `Surface` used for PNG evidence —
//! the screenshot is the app, not a separate renderer.

use orbclient::{Color, Renderer};

use crate::model::{TimelineModel, ToolState};

// — palette: black + orange + two grays, nothing else —
pub const BG: Color = Color::rgb(0, 0, 0);
pub const ORANGE: Color = Color::rgb(255, 140, 0);
pub const TEXT: Color = Color::rgb(230, 230, 230);
pub const DIM: Color = Color::rgb(130, 130, 130);
pub const TRACK: Color = Color::rgb(45, 45, 45);

/// Unifont cell.
pub const CHAR_W: i32 = 8;
pub const LINE_H: i32 = 16;
pub const MARGIN: i32 = 8;

/// Draw a single line of text (unifont 8×16, no wrapping).
pub fn draw_text<R: Renderer>(r: &mut R, x: i32, y: i32, s: &str, c: Color) {
    let mut x = x;
    for ch in s.chars() {
        r.char(x, y, ch, c);
        x += CHAR_W;
    }
}

/// Word-wrap to `cols` characters; words longer than a line are hard-broken.
pub fn wrap(s: &str, cols: usize) -> Vec<String> {
    let cols = cols.max(1);
    let mut lines = Vec::new();
    let mut cur = String::new();
    for word in s.split_whitespace() {
        let mut rest: Vec<char> = word.chars().collect();
        while rest.len() > cols {
            if !cur.is_empty() {
                lines.push(std::mem::take(&mut cur));
            }
            lines.push(rest[..cols].iter().collect());
            rest.drain(..cols);
        }
        let word: String = rest.into_iter().collect();
        if cur.is_empty() {
            cur = word;
        } else if cur.chars().count() + 1 + word.chars().count() <= cols {
            cur.push(' ');
            cur.push_str(&word);
        } else {
            lines.push(std::mem::take(&mut cur));
            cur = word;
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
}

/// The last `rows` wrapped lines of `s` (the screen shows the newest tail).
pub fn tail_lines(s: &str, cols: usize, rows: usize) -> Vec<String> {
    let mut lines = wrap(s, cols);
    if lines.len() > rows {
        lines.drain(0..lines.len() - rows);
    }
    lines
}

/// Fit to one line, truncating with an ASCII ellipsis.
fn one_line(s: &str, cols: usize) -> String {
    let flat = s.replace('\n', " ");
    if flat.chars().count() <= cols {
        flat
    } else {
        let mut t: String = flat.chars().take(cols.saturating_sub(3)).collect();
        t.push_str("...");
        t
    }
}

/// Draw the whole model onto any renderer (window or off-screen surface).
pub fn render<R: Renderer>(r: &mut R, m: &TimelineModel) {
    let w = r.width() as i32;
    let cols = ((w - 2 * MARGIN) / CHAR_W).max(10) as usize;
    r.clear();
    let mut y = MARGIN;

    // — 1. mic: dot, level bar, live/off, last stop reason —
    let dot = if m.mic_live { ORANGE } else { TRACK };
    r.rect(MARGIN, y + 2, 12, 12, dot);
    draw_text(r, MARGIN + 18, y, "MIC", ORANGE);
    let bar_x = MARGIN + 50;
    let bar_w: i32 = 140;
    r.rect(bar_x, y + 3, bar_w as u32, 10, TRACK);
    let fill = (m.mic_level.clamp(0.0, 1.0) * bar_w as f32) as u32;
    if fill > 0 {
        r.rect(bar_x, y + 3, fill, 10, ORANGE);
    }
    let state = if m.mic_live { "LIVE" } else { "OFF" };
    draw_text(r, bar_x + bar_w + 8, y, state, if m.mic_live { ORANGE } else { DIM });
    if let Some(reason) = &m.mic_reason {
        let x = bar_x + bar_w + 52;
        if x + 8 < w - MARGIN {
            draw_text(r, x, y, &one_line(reason, ((w - MARGIN - x) / CHAR_W).max(0) as usize), DIM);
        }
    }
    y += LINE_H + 4;

    // — header line 2: model span + hand-off + error (honesty, VISION §5) —
    {
        let mut x = MARGIN;
        if let Some(span) = &m.model_span {
            draw_text(r, x, y, &format!("MODEL {}", one_line(span, 24)), ORANGE);
            x += (13 + span.chars().count().min(24)) as i32 * CHAR_W + 16;
        }
        if let Some(h) = &m.handoff {
            let status = match h.status {
                trace_events::HandoffStatus::Started => "…",
                trace_events::HandoffStatus::Completed => "done",
                trace_events::HandoffStatus::Failed => "FAILED",
            };
            let s = one_line(&format!("-> {} {status}", h.destination), ((w - MARGIN - x) / CHAR_W).max(0) as usize);
            if !s.is_empty() {
                draw_text(r, x, y, &s, DIM);
            }
        }
        y += LINE_H;
    }
    if let Some(err) = &m.error {
        draw_text(r, MARGIN, y, &one_line(&format!("ERROR {err}"), cols), ORANGE);
        y += LINE_H;
    }
    if m.dropped > 0 {
        // A gap in the history is shown, never hidden (B1 backpressure contract).
        let s = match &m.dropped_reason {
            Some(reason) => format!("DROPPED {} records: {}", m.dropped, one_line(reason, 24)),
            None => format!("DROPPED {} records", m.dropped),
        };
        draw_text(r, MARGIN, y, &one_line(&s, cols), ORANGE);
        y += LINE_H;
    }
    y += 4;

    // — 2. live transcript —
    y = section(r, y, "YOU", w);
    let t_color = if m.transcript_final { TEXT } else { DIM };
    for line in tail_lines(&m.transcript, cols, 2) {
        draw_text(r, MARGIN, y, &line, t_color);
        y += LINE_H;
    }
    y += 4;

    // — 3. cleaned text —
    y = section(r, y, "CLEANED", w);
    if let Some(c) = &m.cleaned {
        for line in tail_lines(c, cols, 2) {
            draw_text(r, MARGIN, y, &line, TEXT);
            y += LINE_H;
        }
    }
    y += 4;

    // — 4. streaming reasoning: the open kimono's main area —
    y = section(r, y, if m.reasoning_done { "REASONING" } else { "REASONING ..." }, w);
    let fixed_below = 2 * (LINE_H + 4) + LINE_H * (1 + 3 + 4); // tools + services labels+rows
    let room = (r.height() as i32 - MARGIN - y - fixed_below).max(0) / LINE_H;
    for line in tail_lines(&m.reasoning, cols, room as usize) {
        draw_text(r, MARGIN, y, &line, DIM);
        y += LINE_H;
    }
    y += 4;

    // — 5. tool calls —
    y = section(r, y, "TOOLS", w);
    for t in m.tools.iter().rev().take(3).rev() {
        let mark = match t.state {
            ToolState::Running => "…",
            ToolState::Ended => ">",
            ToolState::Result => "ok",
        };
        let detail = t.result.clone().unwrap_or_else(|| t.args.clone());
        let line = one_line(&format!("{mark} {} {detail}", t.name), cols);
        draw_text(r, MARGIN, y, &line, TEXT);
        y += LINE_H;
    }
    y += 4;

    // — 6. services & power —
    y = section(r, y, "SERVICES · POWER", w);
    let mut rows = 0;
    for s in m.services.iter().rev().take(4).rev() {
        let mark = if s.running { ORANGE } else { TRACK };
        r.rect(MARGIN, y + 3, 8, 8, mark);
        let line = one_line(&format!("{} g{} {}", s.name, s.generation, s.reason), cols - 2);
        draw_text(r, MARGIN + 14, y, &line, DIM);
        y += LINE_H;
        rows += 1;
    }
    for p in m.power.iter().rev().take(4 - rows as usize).rev() {
        let mark = if p.on { ORANGE } else { TRACK };
        r.rect(MARGIN, y + 3, 8, 8, mark);
        let line = one_line(&format!("{} {}", p.component, p.reason), cols - 2);
        draw_text(r, MARGIN + 14, y, &line, DIM);
        y += LINE_H;
    }
}

/// Orange section label; returns the y below it.
fn section<R: Renderer>(r: &mut R, y: i32, label: &str, _w: i32) -> i32 {
    draw_text(r, MARGIN, y, label, ORANGE);
    y + LINE_H
}
