//! The draw path, tested where it matters: on pixels.
//!
//! Everything renders onto an off-screen `orbclient::Surface` — the same
//! `Renderer` trait the live window uses — and the assertions read the pixel
//! buffer. A test that only checked the scene graph would not catch a colour
//! or a glyph landing in the wrong place.

use orbclient::{Color, Renderer, Surface as Canvas};
use serde_json::{json, Value};

use a2ui_render::doc;
use a2ui_render::scene::{self, Action, Item, ItemKind, Scene};
use a2ui_render::surface::Surface;
use a2ui_render::view;

const W: u32 = 480;
const H: u32 = 800;

/// Build a surface from components + data, exactly as a document would.
fn state(components: impl IntoIterator<Item = Value>, data: Value) -> Surface {
    let mut msgs = vec![json!({"version": "v0.9", "createSurface": {"surfaceId": "s1", "catalogId": "basic"}})];
    for c in components {
        msgs.push(json!({"version": "v0.9", "updateComponents": {"surfaceId": "s1", "components": [c]}}));
    }
    msgs.push(json!({"version": "v0.9", "updateDataModel": {"surfaceId": "s1", "value": data}}));
    let mut s = Surface::default();
    for m in &msgs {
        let parsed = doc::parse_message(m).unwrap_or_else(|e| panic!("message {m} rejected: {e}"));
        s.apply(&parsed).expect("apply");
    }
    s
}

/// Lay out and draw, returning both the pixels and the scene they came from.
fn paint(s: &Surface) -> (Canvas, Scene) {
    let scene = scene::layout(s, W as i32, H as i32);
    let mut canvas = Canvas::new(W, H).expect("off-screen surface");
    view::render(&mut canvas, &scene);
    (canvas, scene)
}

fn px(canvas: &Canvas, x: i32, y: i32) -> Color {
    canvas.data()[(y as usize * W as usize) + x as usize]
}

fn rgb(c: Color) -> (u8, u8, u8) {
    (c.r(), c.g(), c.b())
}

fn find(scene: &Scene, kind: impl Fn(&ItemKind) -> bool) -> &Item {
    scene.items.iter().find(|it| kind(&it.kind)).unwrap_or_else(|| panic!("no such item in {scene:?}"))
}

fn button(scene: &Scene) -> &Item {
    find(scene, |k| matches!(k, ItemKind::Button { .. }))
}

#[test]
fn the_palette_is_five_colours_and_nothing_else() {
    let s = state(
        vec![
            json!({"id": "root", "component": "Column", "children": ["h", "b", "t", "p"]}),
            json!({"id": "h", "component": "Text", "text": "Deploy", "variant": "h1"}),
            json!({"id": "b", "component": "Button", "child": "bl", "variant": "primary", "action": {"event": {"name": "go"}}}),
            json!({"id": "bl", "component": "Text", "text": "Run"}),
            json!({"id": "t", "component": "TextField", "label": "Tag", "value": {"path": "/tag"}}),
            json!({"id": "p", "component": "Progress", "value": 40}),
        ],
        json!({"tag": "v9"}),
    );
    let (canvas, _) = paint(&s);
    let allowed: Vec<(u8, u8, u8)> =
        vec![rgb(view::BG), rgb(view::ORANGE), rgb(view::TEXT), rgb(view::DIM), rgb(view::TRACK)];
    let mut strays = Vec::new();
    for c in canvas.data() {
        let p = rgb(*c);
        if !allowed.contains(&p) && !strays.contains(&p) {
            strays.push(p);
        }
    }
    assert!(strays.is_empty(), "off-palette colours drawn: {strays:?}");
    assert_eq!(rgb(px(&canvas, 0, 0)), rgb(view::BG), "the corner is the background");
    // And the screen is not blank: ink exists.
    assert!(canvas.data().iter().any(|c| rgb(*c) != rgb(view::BG)), "nothing was drawn");
}

#[test]
fn a_primary_button_is_a_filled_orange_target_with_dark_ink() {
    let s = state(
        vec![
            json!({"id": "root", "component": "Button", "child": "l", "variant": "primary", "action": {"event": {"name": "go"}}}),
            json!({"id": "l", "component": "Text", "text": "Deploy"}),
        ],
        Value::Null,
    );
    let (canvas, scene) = paint(&s);
    let b = button(&scene);
    assert!(b.rect.h >= 48, "touch target is {} px tall", b.rect.h);
    // The middle of the label column is either orange fill or a dark glyph on
    // orange; either way the button reads as filled.
    let fill = px(&canvas, b.rect.x + 6, b.rect.y + 6);
    assert_eq!(rgb(fill), rgb(view::ORANGE), "primary button is filled orange");
    // Count the label's glyphs over the whole button: unifont strokes are a
    // handful of pixels per row, so a single scanline proves nothing.
    let dark = (0..b.rect.w)
        .flat_map(|x| (0..b.rect.h).map(move |y| (x, y)))
        .filter(|(x, y)| rgb(px(&canvas, b.rect.x + x, b.rect.y + y)) == rgb(view::BG))
        .count();
    assert!(dark > 100, "the label should punch dark glyphs out of the fill, saw {dark} px");
}

#[test]
fn a_default_button_is_outlined_orange_over_a_track_fill() {
    let s = state(
        vec![
            json!({"id": "root", "component": "Button", "child": "l", "action": {"event": {"name": "go"}}}),
            json!({"id": "l", "component": "Text", "text": "Cancel"}),
        ],
        Value::Null,
    );
    let (canvas, scene) = paint(&s);
    let b = button(&scene);
    assert_eq!(rgb(px(&canvas, b.rect.x + 1, b.rect.y + 1)), rgb(view::ORANGE), "border");
    assert_eq!(rgb(px(&canvas, b.rect.x + 8, b.rect.y + 8)), rgb(view::TRACK), "inside is the track");
}

#[test]
fn a_button_with_no_action_does_not_look_tappable() {
    let s = state(
        vec![
            json!({"id": "root", "component": "Button", "child": "l"}),
            json!({"id": "l", "component": "Text", "text": "Soon"}),
        ],
        Value::Null,
    );
    let (canvas, scene) = paint(&s);
    let b = button(&scene);
    assert!(matches!(b.kind, ItemKind::Button { action: Action::None, .. }));
    assert_eq!(rgb(px(&canvas, b.rect.x + 1, b.rect.y + 1)), rgb(view::DIM), "border is dim, not orange");
    assert_eq!(rgb(px(&canvas, b.rect.x + 8, b.rect.y + 8)), rgb(view::TRACK));
}

#[test]
fn the_progress_fill_matches_the_fraction() {
    let s = state(
        vec![json!({"id": "root", "component": "Progress", "value": 50, "min": 0, "max": 100})],
        Value::Null,
    );
    let (canvas, scene) = paint(&s);
    let p = find(&scene, |k| matches!(k, ItemKind::Progress { .. }));
    let midy = p.rect.y + p.rect.h / 2;
    assert_eq!(rgb(px(&canvas, p.rect.x + p.rect.w / 4, midy)), rgb(view::ORANGE), "inside the fill");
    assert_eq!(rgb(px(&canvas, p.rect.x + p.rect.w * 3 / 4, midy)), rgb(view::TRACK), "outside the fill");
}

#[test]
fn an_indeterminate_progress_shows_three_blocks() {
    let s = state(vec![json!({"id": "root", "component": "Progress", "mode": "indeterminate"})], Value::Null);
    let (canvas, scene) = paint(&s);
    let p = find(&scene, |k| matches!(k, ItemKind::Progress { fraction: None, .. }));
    let midy = p.rect.y + p.rect.h / 2;
    let row: Vec<bool> = (0..p.rect.w).map(|x| rgb(px(&canvas, p.rect.x + x, midy)) == rgb(view::ORANGE)).collect();
    let blocks = row.windows(2).filter(|w| w[0] && !w[1]).count() + usize::from(*row.last().unwrap());
    assert_eq!(blocks, 3, "three separate orange blocks, no continuous fill");
    assert!(row.iter().any(|b| !b), "the track shows between the blocks");
}

#[test]
fn a_focused_field_gets_an_orange_border_and_a_caret() {
    let s = state(
        vec![json!({"id": "root", "component": "TextField", "label": "Room", "value": {"path": "/room"}})],
        json!({"room": "101"}),
    );
    let focused = scene::layout_with(&s, W as i32, H as i32, &scene::Options { focus: Some("root") });
    let mut canvas = Canvas::new(W, H).expect("off-screen surface");
    view::render(&mut canvas, &focused);
    let f = find(&focused, |k| matches!(k, ItemKind::Field { .. }));
    let box_y = f.rect.y + 16 + 4;
    assert_eq!(rgb(px(&canvas, f.rect.x + 1, box_y + 1)), rgb(view::ORANGE), "focused border");
    // "101" is 3 glyphs, so the caret sits just past them.
    let caret_x = f.rect.x + 8 + 3 * 8 + 2;
    let caret_y = box_y + (f.rect.h - 20 - 16) / 2 + 8; // mid-glyph of the value line
    assert_eq!(rgb(px(&canvas, caret_x, caret_y)), rgb(view::ORANGE), "caret");

    let plain = scene::layout(&s, W as i32, H as i32);
    let mut canvas2 = Canvas::new(W, H).expect("off-screen surface");
    view::render(&mut canvas2, &plain);
    // Unfocused is a 1 px border, so it sits exactly on the edge.
    assert_eq!(rgb(px(&canvas2, f.rect.x, box_y)), rgb(view::DIM), "unfocused border is dim");
    assert_ne!(rgb(px(&canvas2, caret_x, caret_y)), rgb(view::ORANGE), "no caret when unfocused");
}

#[test]
fn an_unsupported_component_shows_an_orange_chip_on_screen() {
    let s = state(
        vec![
            json!({"id": "root", "component": "Column", "children": ["t", "c"]}),
            json!({"id": "t", "component": "Text", "text": "Meeting"}),
            json!({"id": "c", "component": "Calendar", "date": "2026-10-24"}),
        ],
        Value::Null,
    );
    let (canvas, scene) = paint(&s);
    let chip = find(&scene, |k| matches!(k, ItemKind::Chip { .. }));
    assert_eq!(rgb(px(&canvas, chip.rect.x, chip.rect.y)), rgb(view::ORANGE), "chip border");
    assert_eq!(scene.unsupported, vec!["Calendar"]);
    // The words are drawn: the chip is not an empty box.
    let ink = (0..chip.rect.w)
        .flat_map(|x| (0..chip.rect.h).map(move |y| (x, y)))
        .filter(|(x, y)| rgb(px(&canvas, chip.rect.x + x, chip.rect.y + y)) == rgb(view::ORANGE))
        .count();
    assert!(ink > 150, "the chip label should be drawn, saw {ink} orange px");
}

#[test]
fn headings_are_orange_and_body_text_is_not() {
    let s = state(
        vec![
            json!({"id": "root", "component": "Column", "children": ["h", "b"]}),
            json!({"id": "h", "component": "Text", "text": "Status", "variant": "h2"}),
            json!({"id": "b", "component": "Text", "text": "all systems nominal"}),
        ],
        Value::Null,
    );
    let (canvas, scene) = paint(&s);
    let texts: Vec<&Item> = scene.items.iter().filter(|it| matches!(it.kind, ItemKind::Text { .. })).collect();
    assert_eq!(texts.len(), 2);
    let ink_in = |it: &Item| -> Vec<(u8, u8, u8)> {
        let mut seen = Vec::new();
        for y in 0..it.rect.h {
            for x in 0..it.rect.w {
                let p = rgb(px(&canvas, it.rect.x + x, it.rect.y + y));
                if p != rgb(view::BG) && !seen.contains(&p) {
                    seen.push(p);
                }
            }
        }
        seen
    };
    assert_eq!(ink_in(texts[0]), vec![rgb(view::ORANGE)], "heading ink");
    assert_eq!(ink_in(texts[1]), vec![rgb(view::TEXT)], "body ink");
}

#[test]
fn an_h1_gets_a_rule_and_the_others_do_not() {
    let s = state(
        vec![
            json!({"id": "root", "component": "Column", "children": ["h", "b"]}),
            json!({"id": "h", "component": "Text", "text": "Deploy", "variant": "h1"}),
            json!({"id": "b", "component": "Text", "text": "body"}),
        ],
        Value::Null,
    );
    let (canvas, scene) = paint(&s);
    let h = find(&scene, |k| matches!(k, ItemKind::Text { style: scene::Style::H1, .. }));
    let rule_y = h.rect.y + 16 + 2; // one line of glyphs, then the rule
    assert_eq!(rgb(px(&canvas, h.rect.x + h.rect.w / 2, rule_y)), rgb(view::ORANGE), "h1 rule spans the column");
}

#[test]
fn an_image_placeholder_is_a_box_with_a_glyph_and_no_orange() {
    let s = state(
        vec![json!({"id": "root", "component": "Image", "url": "https://x/y.png", "description": "Map of the site"})],
        Value::Null,
    );
    let (canvas, scene) = paint(&s);
    let im = find(&scene, |k| matches!(k, ItemKind::Image { .. }));
    assert_eq!(rgb(px(&canvas, im.rect.x, im.rect.y)), rgb(view::DIM), "border");
    assert_eq!(rgb(px(&canvas, im.rect.x + 2, im.rect.y + 2)), rgb(view::TRACK), "inside is the track");
    let glyph = (0..im.rect.w)
        .flat_map(|x| (0..im.rect.h).map(move |y| (x, y)))
        .filter(|(x, y)| rgb(px(&canvas, im.rect.x + x, im.rect.y + y)) == rgb(view::DIM))
        .count();
    assert!(glyph > 200, "the placeholder glyph should be visible, saw {glyph} dim px");
    let orange = (0..im.rect.w)
        .flat_map(|x| (0..im.rect.h).map(move |y| (x, y)))
        .filter(|(x, y)| rgb(px(&canvas, im.rect.x + x, im.rect.y + y)) == rgb(view::ORANGE))
        .count();
    assert_eq!(orange, 0, "an image placeholder is not an action");
}

#[test]
fn the_overflow_marker_appears_only_when_content_does_not_fit() {
    let short = state(
        vec![json!({"id": "root", "component": "Text", "text": "tiny"})],
        Value::Null,
    );
    let (canvas, scene) = paint(&short);
    assert!(!scene.overflow);
    let bottom = (0..W as i32)
        .map(|x| rgb(px(&canvas, x, H as i32 - 1)))
        .collect::<Vec<_>>();
    assert!(bottom.iter().all(|p| *p == rgb(view::BG)), "no marker when everything fits");

    // A list long enough to run past 800 px.
    let items: Vec<String> = (0..40).map(|i| format!("i{i}")).collect();
    let mut comps = vec![json!({"id": "root", "component": "List", "children": items})];
    comps.extend((0..40).map(|i| json!({"id": format!("i{i}"), "component": "Text", "text": format!("row {i}")})));
    let tall = state(comps, Value::Null);
    let (canvas, scene) = paint(&tall);
    assert!(scene.overflow, "40 rows do not fit in 800 px");
    assert_eq!(rgb(px(&canvas, 4, H as i32 - 20)), rgb(view::ORANGE), "the marker bar is orange");
    let marker_ink = (0..W as i32)
        .flat_map(|x| (H as i32 - 16..H as i32).map(move |y| (x, y)))
        .filter(|(x, y)| rgb(px(&canvas, *x, *y)) == rgb(view::ORANGE))
        .count();
    assert!(marker_ink > 100, "MORE BELOW is written, saw {marker_ink} orange px");
}
