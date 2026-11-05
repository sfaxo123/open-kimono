//! Layout tests: one column, real touch targets, and the three rules that
//! make a renderer trustworthy — unknown components are loud, missing data is
//! a placeholder, and a cycle terminates.

use serde_json::{json, Value};

use a2ui_render::scene::{self, Action, ItemKind, Style, Tone};
use a2ui_render::surface::Surface;

const W: i32 = 480;
const H: i32 = 800;

/// Build an open surface from a list of A2UI messages.
fn surface(messages: impl IntoIterator<Item = Value>) -> Surface {
    let mut s = Surface::default();
    for m in messages {
        let msg = a2ui_render::doc::parse_message(&m).expect("message should parse");
        s.apply(&msg).expect("message should apply");
    }
    s
}

/// The usual shape: create a surface, put components on it, done.
fn doc(components: Value, data: Value) -> Vec<Value> {
    vec![
        json!({"version": "v0.9", "createSurface": {"surfaceId": "s1", "catalogId": "basic"}}),
        json!({"version": "v0.9", "updateComponents": {"surfaceId": "s1", "components": components}}),
        json!({"version": "v0.9", "updateDataModel": {"surfaceId": "s1", "value": data}}),
    ]
}

fn kinds(s: &Surface) -> Vec<ItemKind> {
    scene::layout(s, W, H).items.iter().map(|i| i.kind.clone()).collect()
}

fn chips(s: &Surface) -> Vec<(String, Tone)> {
    scene::layout(s, W, H)
        .items
        .iter()
        .filter_map(|i| match &i.kind {
            ItemKind::Chip { label, tone } => Some((label.clone(), *tone)),
            _ => None,
        })
        .collect()
}

fn texts(s: &Surface) -> Vec<(Vec<String>, Style)> {
    scene::layout(s, W, H)
        .items
        .iter()
        .filter_map(|i| match &i.kind {
            ItemKind::Text { lines, style } => Some((lines.clone(), *style)),
            _ => None,
        })
        .collect()
}

// ---- the column ----------------------------------------------------------

#[test]
fn children_stack_top_to_bottom_in_one_column() {
    let s = surface(doc(
        json!([
            {"id": "root", "component": "Column", "children": ["a", "b", "c"]},
            {"id": "a", "component": "Text", "text": "first"},
            {"id": "b", "component": "Text", "text": "second"},
            {"id": "c", "component": "Text", "text": "third"}
        ]),
        Value::Null,
    ));
    let sc = scene::layout(&s, W, H);
    let text: Vec<_> = sc.items.iter().filter(|i| matches!(i.kind, ItemKind::Text { .. })).collect();
    assert_eq!(text.len(), 3, "every child must be drawn");
    // One column: same x, same width, strictly increasing y.
    for it in &text {
        assert_eq!(it.rect.x, scene::MARGIN);
        assert_eq!(it.rect.w, W - 2 * scene::MARGIN);
    }
    assert!(text[0].rect.y < text[1].rect.y && text[1].rect.y < text[2].rect.y);
    // No overlap: each item ends at or before the next begins.
    assert!(text[0].rect.y + text[0].rect.h <= text[1].rect.y);
    assert!(text[1].rect.y + text[1].rect.h <= text[2].rect.y);
    assert!(!sc.overflow, "three lines fit in 800 px");
}

#[test]
fn headings_are_uppercased_and_h1_gets_a_rule() {
    let s = surface(doc(
        json!([
            {"id": "root", "component": "Column", "children": ["t1", "t2"]},
            {"id": "t1", "component": "Text", "text": "system status", "variant": "h1"},
            {"id": "t2", "component": "Text", "text": "plain body"}
        ]),
        Value::Null,
    ));
    let out = texts(&s);
    assert_eq!(out[0], (vec!["SYSTEM STATUS".to_string()], Style::H1));
    assert_eq!(out[1], (vec!["plain body".to_string()], Style::Body));
    assert!(Style::H1.rule(), "h1 is the one role with a rule");
    assert!(Style::H1.top_space() > Style::H3.top_space());
}

#[test]
fn long_text_wraps_and_reserves_exactly_that_height() {
    let long = "the quick brown fox jumps over the lazy dog and keeps running past the margin";
    let s = surface(doc(
        json!([
            {"id": "root", "component": "Text", "text": long}
        ]),
        Value::Null,
    ));
    let sc = scene::layout(&s, W, H);
    let ItemKind::Text { lines, .. } = &sc.items[0].kind else {
        panic!("expected text, got {:?}", sc.items[0].kind)
    };
    assert!(lines.len() > 1, "a 79-char sentence must wrap in 464 px");
    for l in lines {
        assert!(l.chars().count() as i32 * scene::CHAR_W <= W - 2 * scene::MARGIN, "line too wide: {l}");
    }
    assert_eq!(sc.items[0].rect.h, lines.len() as i32 * scene::LINE_H);
}

#[test]
fn wrap_hard_breaks_words_longer_than_the_column() {
    let lines = scene::wrap("supercalifragilisticexpialidocious", 10);
    assert_eq!(lines.iter().map(|l| l.chars().count()).max(), Some(10));
    assert_eq!(lines.concat(), "supercalifragilisticexpialidocious");
    assert_eq!(lines.len(), 4, "34 chars in 10-char chunks, no duplicate tail");
    assert_eq!(scene::wrap("a\nb", 40), vec!["a", "b"]);
    assert_eq!(scene::wrap("", 40), vec![""]);
    assert_eq!(scene::wrap("one two\n\nthree", 40), vec!["one two", "", "three"]);
    assert_eq!(scene::one_line("abcdefghij", 6), "abc...");
    assert_eq!(scene::one_line("short", 6), "short");
}

// ---- buttons -------------------------------------------------------------

#[test]
fn button_is_a_touch_target_and_hit_testable() {
    let s = surface(doc(
        json!([
            {"id": "root", "component": "Column", "children": ["btn"]},
            {"id": "btn", "component": "Button", "child": "lbl", "variant": "primary",
             "action": {"event": {"name": "confirm", "context": {"orderId": {"path": "/order/id"}}}}},
            {"id": "lbl", "component": "Text", "text": "Confirm order"}
        ]),
        json!({"order": {"id": "A-77"}}),
    ));
    let sc = scene::layout(&s, W, H);
    let it = sc.hit_test(W / 2, sc.items[0].rect.y + 1).expect("tap in the button must hit it");
    let ItemKind::Button { id, label, variant, action } = &it.kind else {
        panic!("expected a button, got {:?}", it.kind)
    };
    assert_eq!(id, "btn");
    assert_eq!(label, "Confirm order", "the label comes from the child Text");
    assert_eq!(*variant, scene::ButtonVariant::Primary);
    assert!(it.rect.h >= scene::TOUCH_H, "finger-sized: {} px", it.rect.h);
    // The action's dynamic context is resolved at layout time, not at tap time.
    assert_eq!(
        action,
        &Action::Event { name: "confirm".into(), context: json!({"orderId": "A-77"}) }
    );
    // A tap outside every interactive box hits nothing.
    assert!(sc.hit_test(2, 2).is_none());
}

#[test]
fn button_without_an_action_is_inert_and_says_so() {
    let s = surface(doc(
        json!([
            {"id": "root", "component": "Button", "text": "Nothing"}
        ]),
        Value::Null,
    ));
    let sc = scene::layout(&s, W, H);
    let ItemKind::Button { action, .. } = &sc.items[0].kind else { panic!("not a button") };
    assert_eq!(action, &Action::None);
    assert!(sc.diagnostics.iter().any(|d| d.contains("no action")));
}

#[test]
fn row_puts_targets_side_by_side() {
    let s = surface(doc(
        json!([
            {"id": "root", "component": "Row", "children": ["yes", "no"]},
            {"id": "yes", "component": "Button", "text": "Yes", "action": {"event": {"name": "yes"}}},
            {"id": "no", "component": "Button", "text": "No", "action": {"event": {"name": "no"}}}
        ]),
        Value::Null,
    ));
    let sc = scene::layout(&s, W, H);
    let btns: Vec<_> = sc.items.iter().filter(|i| matches!(i.kind, ItemKind::Button { .. })).collect();
    assert_eq!(btns.len(), 2);
    assert_eq!(btns[0].rect.y, btns[1].rect.y, "side by side, not stacked");
    assert!(btns[0].rect.x < btns[1].rect.x);
    for b in &btns {
        assert!(b.rect.h >= scene::TOUCH_H);
    }
    // Both halves are reachable.
    assert!(sc.hit_test(btns[0].rect.x + 4, btns[0].rect.y + 4).is_some());
    assert!(sc.hit_test(btns[1].rect.x + 4, btns[1].rect.y + 4).is_some());
}

// ---- loud failures -------------------------------------------------------

#[test]
fn unknown_component_renders_a_visible_chip() {
    let s = surface(doc(
        json!([
            {"id": "root", "component": "Column", "children": ["t", "tabs"]},
            {"id": "t", "component": "Text", "text": "above"},
            {"id": "tabs", "component": "Tabs", "tabs": [{"label": "one", "child": "t"}]}
        ]),
        Value::Null,
    ));
    let sc = scene::layout(&s, W, H);
    assert_eq!(sc.unsupported, vec!["Tabs".to_string()]);
    let chip = chips(&s);
    assert_eq!(chip, vec![("unsupported: Tabs".to_string(), Tone::Unsupported)]);
    // Visible means it occupies real pixels, not a zero-height note.
    let it = sc.items.iter().find(|i| matches!(i.kind, ItemKind::Chip { .. })).unwrap();
    assert!(it.rect.h >= scene::LINE_H && it.rect.w > 100);
    // And the rest of the screen still rendered.
    assert!(texts(&s).iter().any(|(l, _)| l == &vec!["above".to_string()]));
}

#[test]
fn two_unknown_types_are_both_named_once_each() {
    let s = surface(doc(
        json!([
            {"id": "root", "component": "Column", "children": ["a", "b", "c"]},
            {"id": "a", "component": "Tabs"},
            {"id": "b", "component": "Modal"},
            {"id": "c", "component": "Tabs"}
        ]),
        Value::Null,
    ));
    let sc = scene::layout(&s, W, H);
    assert_eq!(sc.unsupported, vec!["Tabs".to_string(), "Modal".to_string()]);
    assert_eq!(chips(&s).len(), 3, "each instance still gets its own chip");
}

#[test]
fn a_child_that_has_not_arrived_yet_is_a_placeholder() {
    let s = surface(doc(
        json!([
            {"id": "root", "component": "Column", "children": ["t", "later"]},
            {"id": "t", "component": "Text", "text": "here now"}
        ]),
        Value::Null,
    ));
    let sc = scene::layout(&s, W, H);
    assert_eq!(chips(&s), vec![("waiting: later".to_string(), Tone::Waiting)]);
    assert!(sc.diagnostics.iter().any(|d| d.contains("not defined yet")));
}

#[test]
fn a_document_without_a_root_is_reported_not_blank() {
    let s = surface(doc(
        json!([
            {"id": "notroot", "component": "Text", "text": "orphan"}
        ]),
        Value::Null,
    ));
    let sc = scene::layout(&s, W, H);
    assert_eq!(chips(&s), vec![("no root component".to_string(), Tone::Broken)]);
    assert!(sc.diagnostics.iter().any(|d| d.contains("root")));
}

#[test]
fn a_closed_surface_renders_a_notice() {
    let s = Surface::default();
    let sc = scene::layout(&s, W, H);
    assert_eq!(chips(&s), vec![("no surface open".to_string(), Tone::Broken)]);
    assert!(sc.items.len() == 1);
}

#[test]
fn self_referencing_component_terminates_with_a_cycle_chip() {
    let s = surface(doc(
        json!([
            {"id": "root", "component": "Column", "children": ["root", "t"]},
            {"id": "t", "component": "Text", "text": "after"}
        ]),
        Value::Null,
    ));
    let sc = scene::layout(&s, W, H);
    assert_eq!(chips(&s), vec![("cycle: root".to_string(), Tone::Cycle)]);
    assert!(sc.diagnostics.iter().any(|d| d.contains("cycle at `root`")));
    // The sibling after the cycle still renders — the guard did not bail out.
    assert!(texts(&s).iter().any(|(l, _)| l == &vec!["after".to_string()]));
}

#[test]
fn mutual_recursion_terminates() {
    let s = surface(doc(
        json!([
            {"id": "root", "component": "Card", "child": "a"},
            {"id": "a", "component": "Column", "child": "b"},
            {"id": "b", "component": "Column", "child": "a"}
        ]),
        Value::Null,
    ));
    let sc = scene::layout(&s, W, H);
    assert!(chips(&s).iter().any(|(l, t)| l == "cycle: a" && *t == Tone::Cycle));
    assert!(sc.content_height < 4 * scene::LINE_H + 4 * scene::MARGIN, "must not unroll");
}

// ---- lists ---------------------------------------------------------------

#[test]
fn list_template_renders_one_row_per_data_item() {
    let s = surface(doc(
        json!([
            {"id": "root", "component": "List", "children": {"componentId": "row", "path": "/servers"}},
            {"id": "row", "component": "Text", "text": {"call": "formatString", "args": {"value": "${name}: ${state}"}, "returnType": "string"}}
        ]),
        json!({"servers": [
            {"name": "5090", "state": "up"},
            {"name": "spark", "state": "up"},
            {"name": "3090", "state": "draining"}
        ]}),
    ));
    let out = texts(&s);
    assert_eq!(
        out.iter().map(|(l, _)| l.concat()).collect::<Vec<_>>(),
        vec!["5090: up", "spark: up", "3090: draining"]
    );
    // Relative bindings resolved against each item, not the model root.
    assert!(chips(&s).is_empty());
}

#[test]
fn list_waiting_for_its_data_shows_a_placeholder() {
    let s = surface(doc(
        json!([
            {"id": "root", "component": "List", "children": {"componentId": "row", "path": "/servers"}},
            {"id": "row", "component": "Text", "text": {"path": "name"}}
        ]),
        json!({"nothing": 1}),
    ));
    assert_eq!(chips(&s), vec![("waiting: /servers".to_string(), Tone::Waiting)]);
}

#[test]
fn list_bound_to_a_non_list_is_loud() {
    let s = surface(doc(
        json!([
            {"id": "root", "component": "List", "children": {"componentId": "row", "path": "/server"}},
            {"id": "row", "component": "Text", "text": "x"}
        ]),
        json!({"server": {"name": "5090"}}),
    ));
    assert_eq!(chips(&s), vec![("not a list: /server".to_string(), Tone::Broken)]);
}

#[test]
fn empty_list_says_empty_instead_of_vanishing() {
    let s = surface(doc(
        json!([
            {"id": "root", "component": "List", "children": {"componentId": "row", "path": "/servers"}},
            {"id": "row", "component": "Text", "text": "x"}
        ]),
        json!({"servers": []}),
    ));
    assert_eq!(chips(&s), vec![("empty list: /servers".to_string(), Tone::Broken)]);
}

#[test]
fn static_list_stacks_its_rows() {
    let s = surface(doc(
        json!([
            {"id": "root", "component": "List", "children": ["a", "b"]},
            {"id": "a", "component": "Text", "text": "one"},
            {"id": "b", "component": "Text", "text": "two"}
        ]),
        Value::Null,
    ));
    let sc = scene::layout(&s, W, H);
    let ys: Vec<i32> = sc.items.iter().filter(|i| matches!(i.kind, ItemKind::Text { .. })).map(|i| i.rect.y).collect();
    assert_eq!(ys.len(), 2);
    assert!(ys[0] < ys[1]);
}

// ---- progress ------------------------------------------------------------

fn progress_item(s: &Surface) -> (Option<f64>, String) {
    let sc = scene::layout(s, W, H);
    for i in &sc.items {
        if let ItemKind::Progress { fraction, readout } = &i.kind {
            return (*fraction, readout.clone());
        }
    }
    panic!("no progress bar drawn: {:?}", kinds(s))
}

#[test]
fn progress_normalises_value_against_min_and_max() {
    let s = surface(doc(
        json!([
            {"id": "root", "component": "Progress", "value": {"path": "/done"}, "min": 0, "max": 60, "label": "Restore"}
        ]),
        json!({"done": 15}),
    ));
    let (f, readout) = progress_item(&s);
    assert_eq!(f, Some(0.25));
    assert_eq!(readout, "25%");
    // The label is drawn as its own caption line.
    assert!(texts(&s).iter().any(|(l, st)| l == &vec!["Restore".to_string()] && *st == Style::Caption));
}

#[test]
fn progress_clamps_out_of_range_values() {
    let s = surface(doc(
        json!([{"id": "root", "component": "Progress", "value": 140}]),
        Value::Null,
    ));
    assert_eq!(progress_item(&s), (Some(1.0), "100%".to_string()));
}

#[test]
fn progress_defaults_to_a_hundred_and_accepts_the_community_names() {
    for name in ["Progress", "ProgressBar", "MaterialProgressBar"] {
        let s = surface(doc(
            json!([{"id": "root", "component": name, "value": 50}]),
            Value::Null,
        ));
        assert_eq!(progress_item(&s), (Some(0.5), "50%".to_string()), "{name} must be understood");
        assert!(scene::layout(&s, W, H).unsupported.is_empty(), "{name} is not unsupported");
    }
}

#[test]
fn indeterminate_progress_has_no_fraction() {
    let s = surface(doc(
        json!([{"id": "root", "component": "Progress", "mode": "indeterminate", "label": "Syncing"}]),
        Value::Null,
    ));
    let (f, readout) = progress_item(&s);
    assert_eq!(f, None);
    assert_eq!(readout, "…");
}

#[test]
fn progress_with_no_value_is_a_diagnostic_not_a_crash() {
    let s = surface(doc(
        json!([{"id": "root", "component": "Progress", "value": {"path": "/pct"}}]),
        json!({}),
    ));
    let (f, readout) = progress_item(&s);
    assert_eq!(f, None);
    assert_eq!(readout, "");
    assert!(scene::layout(&s, W, H).diagnostics.iter().any(|d| d.contains("no value yet")));
}

// ---- images --------------------------------------------------------------

#[test]
fn image_is_a_placeholder_box_with_its_description() {
    let s = surface(doc(
        json!([
            {"id": "root", "component": "Image", "url": "https://example/x.png", "description": "Thermal map", "variant": "largeFeature"}
        ]),
        Value::Null,
    ));
    let sc = scene::layout(&s, W, H);
    let it = sc.items.iter().find(|i| matches!(i.kind, ItemKind::Image { .. })).expect("image box");
    assert_eq!(it.rect.h, 200, "largeFeature reserves a real block of screen");
    let ItemKind::Image { description, url } = &it.kind else { unreachable!() };
    assert_eq!(description, "Thermal map");
    assert_eq!(url, "https://example/x.png");
    assert!(texts(&s).iter().any(|(l, st)| l == &vec!["Thermal map".to_string()] && *st == Style::Caption));
}

// ---- text fields ---------------------------------------------------------

#[test]
fn field_shows_the_model_value_and_remembers_its_path() {
    let s = surface(doc(
        json!([
            {"id": "root", "component": "Column", "children": ["f"]},
            {"id": "f", "component": "TextField", "label": "Room", "value": {"path": "/room"}}
        ]),
        json!({"room": "server closet"}),
    ));
    let sc = scene::layout(&s, W, H);
    let it = sc.hit_test(W / 2, sc.items[0].rect.y + sc.items[0].rect.h - 1).expect("field is tappable");
    let ItemKind::Field { id, label, value, path, focused, obscured } = &it.kind else {
        panic!("expected a field, got {:?}", it.kind)
    };
    assert_eq!(id, "f");
    assert_eq!(label, "Room");
    assert_eq!(value, "server closet");
    assert_eq!(path.as_deref(), Some("/room"));
    assert!(!focused);
    assert!(!obscured);
    // The touch box covers the input row, not just the label.
    assert!(it.rect.h >= scene::LINE_H + scene::TOUCH_H);
}

#[test]
fn typed_text_wins_over_the_model_and_writes_back() {
    let mut s = surface(doc(
        json!([
            {"id": "root", "component": "TextField", "label": "Room", "value": {"path": "/room"}}
        ]),
        json!({"room": "old"}),
    ));
    let written = s.set_field_value("root", "new").expect("field is bound");
    assert_eq!(written, "/room");
    let sc = scene::layout(&s, W, H);
    let ItemKind::Field { value, .. } = &sc.items[0].kind else { panic!("no field") };
    assert_eq!(value, "new");
    assert_eq!(s.data["room"], json!("new"), "the model is updated too");
}

#[test]
fn focus_marks_exactly_one_field() {
    let s = surface(doc(
        json!([
            {"id": "root", "component": "Column", "children": ["a", "b"]},
            {"id": "a", "component": "TextField", "label": "One", "value": "1"},
            {"id": "b", "component": "TextField", "label": "Two", "value": "2"}
        ]),
        Value::Null,
    ));
    let sc = scene::layout_with(&s, W, H, &scene::Options { focus: Some("b") });
    let focused: Vec<String> = sc
        .items
        .iter()
        .filter_map(|i| match &i.kind {
            ItemKind::Field { id, focused, .. } if *focused => Some(id.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(focused, vec!["b".to_string()]);
    assert_eq!(sc.focused_field().map(|i| i.rect.y), sc.items.last().map(|i| i.rect.y));
    assert!(scene::layout(&s, W, H).focused_field().is_none(), "no focus by default");
}

#[test]
fn obscured_field_shows_dots_not_the_secret() {
    let s = surface(doc(
        json!([
            {"id": "root", "component": "TextField", "label": "Pass", "value": "hunter2", "variant": "obscured"}
        ]),
        Value::Null,
    ));
    let sc = scene::layout(&s, W, H);
    let ItemKind::Field { value, obscured, .. } = &sc.items[0].kind else { panic!("no field") };
    assert_eq!(value, "•••••••");
    assert!(*obscured);
    assert!(!value.contains('h'));
}

// ---- containers and overflow --------------------------------------------

#[test]
fn card_frame_encloses_its_children() {
    let s = surface(doc(
        json!([
            {"id": "root", "component": "Card", "children": ["t"]},
            {"id": "t", "component": "Text", "text": "inside"}
        ]),
        Value::Null,
    ));
    let sc = scene::layout(&s, W, H);
    let frame = sc.items.iter().find(|i| i.kind == ItemKind::Frame).expect("card outline");
    let text = sc.items.iter().find(|i| matches!(i.kind, ItemKind::Text { .. })).unwrap();
    assert!(frame.rect.x <= text.rect.x && frame.rect.y <= text.rect.y);
    assert!(frame.rect.x + frame.rect.w >= text.rect.x + text.rect.w);
    assert!(frame.rect.y + frame.rect.h >= text.rect.y + text.rect.h);
    assert!(frame.rect.h >= text.rect.h + 2 * scene::CARD_PAD);
}

#[test]
fn divider_is_a_line_that_takes_a_little_room() {
    let s = surface(doc(
        json!([
            {"id": "root", "component": "Column", "children": ["a", "d", "b"]},
            {"id": "a", "component": "Text", "text": "above"},
            {"id": "d", "component": "Divider"},
            {"id": "b", "component": "Text", "text": "below"}
        ]),
        Value::Null,
    ));
    let sc = scene::layout(&s, W, H);
    let d = sc.items.iter().find(|i| i.kind == ItemKind::Divider).expect("divider drawn");
    assert_eq!(d.rect.w, W - 2 * scene::MARGIN);
    let ys: Vec<i32> = sc.items.iter().filter(|i| matches!(i.kind, ItemKind::Text { .. })).map(|i| i.rect.y).collect();
    assert_eq!(ys.len(), 2);
    assert!(ys[0] < d.rect.y && d.rect.y < ys[1]);
}

#[test]
fn content_taller_than_the_screen_is_flagged_not_dropped() {
    let mut components = vec![json!({"id": "root", "component": "Column", "children": []})];
    let mut ids = Vec::new();
    for i in 0..60 {
        ids.push(json!(format!("t{i}")));
        components.push(json!({"id": format!("t{i}"), "component": "Text", "text": format!("row {i}")}));
    }
    components[0]["children"] = Value::Array(ids);
    let s = surface(doc(Value::Array(components), Value::Null));
    let sc = scene::layout(&s, W, 200);
    assert!(sc.overflow, "60 rows do not fit in 200 px");
    assert!(sc.content_height > 200);
    assert_eq!(sc.items.len(), 60, "everything is still laid out and drawable");
}

#[test]
fn a_broken_document_leaves_the_renderer_alive() {
    // Every pathological shape at once: no root, unknown type, missing child,
    // a template over a scalar, and a button whose child is absent.
    let s = surface(doc(
        json!([
            {"id": "root", "component": "Column", "children": ["gone", "btn", "lst", "x"]},
            {"id": "x", "component": "Tabs"},
            {"id": "btn", "component": "Button", "child": "gone", "action": {"event": {"name": "n"}}},
            {"id": "lst", "component": "List", "children": {"componentId": "row", "path": "/scalar"}},
            {"id": "row", "component": "Text", "text": "r"}
        ]),
        json!({"scalar": 7}),
    ));
    let sc = scene::layout(&s, W, H);
    let labels: Vec<String> = chips(&s).into_iter().map(|(l, _)| l).collect();
    assert!(labels.iter().any(|l| l == "unsupported: Tabs"));
    assert!(labels.iter().any(|l| l == "waiting: gone"));
    assert!(labels.iter().any(|l| l == "not a list: /scalar"));
    assert_eq!(sc.unsupported, vec!["Tabs".to_string()]);
}
