//! Golden-image tests: four sample A2UI documents are rendered exactly as the
//! app renders them, and compared pixel-for-pixel against committed PNGs.
//!
//! The comparison is on **decoded pixels**, not PNG bytes — a change in the
//! zlib version must not fail the suite, a change in what is drawn must.
//! Regenerate the goldens with `A2UI_BLESS=1 cargo test --test golden` after
//! an intentional change, and look at the diff before committing it.

use std::path::PathBuf;

use orbclient::{Color, Renderer as _, Surface};

use a2ui_render::{doc, png, scene, surface::Surface as Model, view};

const W: u32 = 480;
const H: u32 = 800;
const FIXTURES: [&str; 4] = ["deploy", "booking", "progress", "broken"];

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("fixtures/{name}.json"))
}

fn fixture(name: &str) -> String {
    let path = fixture_path(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// Load a document into a model, exactly as the binary does.
fn model_for(name: &str) -> (Model, String) {
    let parsed = doc::parse_document(&fixture(name));
    assert!(parsed.errors.is_empty(), "{name}: parse errors: {:?}", parsed.errors);
    let mut m = Model::default();
    let mut id = String::new();
    for msg in &parsed.messages {
        if let doc::Message::CreateSurface { surface_id, .. } = msg {
            id = surface_id.clone();
        }
        m.apply(msg).unwrap_or_else(|e| panic!("{name}: apply failed: {e}"));
    }
    (m, id)
}

/// The frame the app would show for this document.
fn frame(name: &str) -> (Vec<Color>, scene::Scene) {
    let (m, _) = model_for(name);
    let sc = scene::layout(&m, W as i32, H as i32);
    let mut canvas = Surface::new(W, H).expect("off-screen surface");
    view::render(&mut canvas, &sc);
    (canvas.data().to_vec(), sc)
}

fn golden_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("fixtures/golden/{name}.png"))
}

#[test]
fn the_four_documents_match_their_golden_pixels() {
    let bless = std::env::var("A2UI_BLESS").is_ok();
    for name in FIXTURES {
        let (pixels, _) = frame(name);
        let path = golden_path(name);
        if bless {
            png::write_png(&path, W, H, &pixels).expect("write golden");
            continue;
        }
        let golden = png::read_png(&path)
            .unwrap_or_else(|e| panic!("no golden for {name} ({}): {e}\n  run: A2UI_BLESS=1 cargo test --test golden", path.display()));
        assert_eq!((golden.width, golden.height), (W, H), "{name}: golden is the wrong size");
        let diffs: Vec<(usize, Color, Color)> = pixels
            .iter()
            .zip(golden.pixels.iter())
            .enumerate()
            .filter(|(_, (a, b))| a.data != b.data)
            .map(|(i, (a, b))| (i, *a, *b))
            .collect();
        assert!(
            diffs.is_empty(),
            "{name}: {} pixels differ from the golden, first few: {:?} (x = idx % {W})",
            diffs.len(),
            diffs.iter().take(4).map(|(i, a, b)| (i % W as usize, i / W as usize, *a, *b)).collect::<Vec<_>>()
        );
    }
    if bless {
        // A bless run must not silently pass as a comparison.
        panic!("A2UI_BLESS is set: the goldens were rewritten, re-run without it to compare");
    }
}

#[test]
fn the_goldens_are_real_images_not_blank_frames() {
    for name in FIXTURES {
        let img = png::read_png(&golden_path(name)).expect("golden readable");
        let lit = img.pixels.iter().filter(|c| c.data != 0xFF000000).count();
        assert!(lit > 2_000, "{name}: only {lit} non-background pixels — the frame is nearly empty");
        let orange = img.pixels.iter().filter(|c| c.r() == 255 && c.g() == 140 && c.b() == 0).count();
        assert!(orange > 200, "{name}: only {orange} orange pixels — nothing is being emphasised");
    }
}

#[test]
fn deploy_renders_a_whole_surface_with_no_complaints() {
    let (m, id) = model_for("deploy");
    assert_eq!(id, "deploy");
    let sc = scene::layout(&m, W as i32, H as i32);
    assert!(sc.unsupported.is_empty(), "deploy should use only supported types, got {:?}", sc.unsupported);
    assert!(sc.diagnostics.is_empty(), "unexpected diagnostics: {:?}", sc.diagnostics);
    assert!(!sc.overflow, "deploy fits the screen (content {} px in {H})", sc.content_height);

    let texts: Vec<String> = sc
        .items
        .iter()
        .filter_map(|it| match &it.kind {
            scene::ItemKind::Text { lines, .. } => Some(lines.join(" ")),
            _ => None,
        })
        .collect();
    // The h1 is uppercased because unifont cannot scale.
    assert!(texts.iter().any(|t| t.contains("DEPLOY REDOX-ORB")), "heading: {texts:?}");
    // The template list read the data model: three steps, relative paths resolved.
    assert!(texts.iter().any(|t| t.contains("CHECKOUT - DONE") || t.contains("checkout - done")), "steps: {texts:?}");
    assert!(texts.iter().any(|t| t.contains("smoke test - queued")), "third step: {texts:?}");

    let bar = sc.items.iter().find_map(|it| match &it.kind {
        scene::ItemKind::Progress { fraction, readout } => Some((*fraction, readout.clone())),
        _ => None,
    });
    assert_eq!(bar.as_ref().map(|b| b.0), Some(Some(0.62)), "the bar reads /percent");

    let buttons: Vec<(&str, &str)> = sc
        .items
        .iter()
        .filter_map(|it| match &it.kind {
            scene::ItemKind::Button { id, label, .. } => Some((id.as_str(), label.as_str())),
            _ => None,
        })
        .collect();
    assert_eq!(buttons, vec![("go", "Deploy now"), ("cancel", "Cancel")]);
    // Every touch target is at least 48 px tall.
    for it in sc.items.iter().filter(|it| matches!(it.kind, scene::ItemKind::Button { .. })) {
        assert!(it.rect.h >= 48, "button {} is {} px tall", it.rect.y, it.rect.h);
    }
}

#[test]
fn booking_renders_fields_a_card_and_an_image() {
    let (m, _) = model_for("booking");
    let sc = scene::layout(&m, W as i32, H as i32);
    assert!(sc.unsupported.is_empty(), "{:?}", sc.unsupported);
    let fields: Vec<(&str, &str)> = sc
        .items
        .iter()
        .filter_map(|it| match &it.kind {
            scene::ItemKind::Field { label, value, .. } => Some((label.as_str(), value.as_str())),
            _ => None,
        })
        .collect();
    assert_eq!(fields, vec![("Date", "2026-11-02"), ("Guests", "4"), ("Room", "101")]);
    assert!(sc.items.iter().any(|it| matches!(&it.kind, scene::ItemKind::Image { description, .. } if description == "Map to the venue")));
    assert!(sc.items.iter().any(|it| matches!(&it.kind, scene::ItemKind::Frame)), "the Card drew its frame");
    assert!(!sc.overflow, "booking fits ({} px)", sc.content_height);
}

#[test]
fn progress_renders_both_kinds_of_bar_and_wraps_its_paragraph() {
    let (m, _) = model_for("progress");
    let sc = scene::layout(&m, W as i32, H as i32);
    assert!(sc.unsupported.is_empty(), "{:?}", sc.unsupported);
    let bars: Vec<(Option<f64>, String)> = sc
        .items
        .iter()
        .filter_map(|it| match &it.kind {
            scene::ItemKind::Progress { fraction, readout } => Some((*fraction, readout.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(bars.len(), 2);
    assert_eq!(bars[0].0, Some(0.25), "the determinate bar reads /pct");
    assert_eq!(bars[1].0, None, "the indeterminate bar has no fraction");

    let para = sc
        .items
        .iter()
        .find_map(|it| match &it.kind {
            scene::ItemKind::Text { lines, style } if lines.join(" ").contains("homelab link") => Some((lines.clone(), *style)),
            _ => None,
        })
        .expect("the paragraph is on screen");
    assert!(para.0.len() >= 3, "the paragraph wrapped to {} lines", para.0.len());
    for line in &para.0 {
        assert!((line.chars().count() as i32) * 8 <= (W as i32 - 16), "line overruns the column: {line:?}");
    }
    // The file list resolved relative paths inside the template.
    let captions: Vec<String> = sc
        .items
        .iter()
        .filter_map(|it| match &it.kind {
            scene::ItemKind::Text { lines, style } if *style == scene::Style::Caption => Some(lines.join(" ")),
            _ => None,
        })
        .collect();
    assert!(captions.iter().any(|c| c.contains("vendor.img - 512 KB")), "file list: {captions:?}");
}

#[test]
fn broken_shows_everything_it_could_not_do() {
    let (m, _) = model_for("broken");
    let sc = scene::layout(&m, W as i32, H as i32);
    assert_eq!(sc.unsupported, vec!["Calendar"], "one unknown type, named");
    let joined = sc.diagnostics.join("\n");
    assert!(joined.contains("Calendar"), "the unknown type is explained, not silent");
    assert!(joined.contains("not_yet"), "a child id that has not arrived is reported");
    assert!(joined.contains("cycle"), "the cycle is reported and did not hang the layout");
    assert!(joined.contains("inert"), "a button with no action is reported");
    assert!(sc.overflow, "twenty-six notes overflow the screen ({} px of content)", sc.content_height);

    // The chips are drawn on screen, in words, in orange.
    let chips: Vec<String> = sc
        .items
        .iter()
        .filter_map(|it| match &it.kind {
            scene::ItemKind::Chip { label, .. } => Some(label.clone()),
            _ => None,
        })
        .collect();
    assert!(chips.iter().any(|c| c == "unsupported: Calendar"), "chips: {chips:?}");
    assert!(chips.iter().any(|c| c.starts_with("waiting:")), "chips: {chips:?}");
    assert!(chips.iter().any(|c| c.starts_with("cycle:")), "chips: {chips:?}");

    // The inert button is still a hit target, so a tap on it is reported.
    let inert = sc.items.iter().find(|it| matches!(&it.kind, scene::ItemKind::Button { id, .. } if id == "inert")).expect("inert button");
    assert!(matches!(inert.kind, scene::ItemKind::Button { action: scene::Action::None, .. }));
    assert!(sc.hit_test(inert.rect.x + 4, inert.rect.y + 4).is_some());
}

/// Run the binary with a clean display environment (headless pass) or the
/// dummy driver (the real event loop, off-screen).
fn run(bin: &str, args: &[&str], dummy: bool) -> String {
    let out = std::process::Command::new(bin)
        .env("SDL_VIDEODRIVER", if dummy { "dummy" } else { "" })
        .env("DISPLAY", "")
        .env("WAYLAND_DISPLAY", "")
        .args(args)
        .output()
        .expect("run the binary");
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn the_binary_draws_exactly_the_frame_the_library_describes() {
    let bin = env!("CARGO_BIN_EXE_a2ui-render");
    let dir = std::env::temp_dir().join(format!("a2ui-golden-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let shot = dir.join("deploy.png");

    run(bin, &["--doc", &fixture_path("deploy").to_string_lossy(), "--screenshot", &shot.to_string_lossy()], false);

    let shot_img = png::read_png(&shot).expect("the binary wrote a PNG");
    assert_eq!((shot_img.width, shot_img.height), (W, H));
    let (library, _) = frame("deploy");
    let diff = shot_img.pixels.iter().zip(library.iter()).filter(|(a, b)| a.data != b.data).count();
    assert_eq!(diff, 0, "the binary's frame differs from the library's by {diff} pixels");
    // And it matches the committed golden, so the golden is the app's frame.
    let golden = png::read_png(&golden_path("deploy")).expect("golden readable");
    let diff = shot_img.pixels.iter().zip(golden.pixels.iter()).filter(|(a, b)| a.data != b.data).count();
    assert_eq!(diff, 0, "the golden is not what the binary draws ({diff} pixels differ)");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_typed_note_and_a_press_become_journal_records() {
    let bin = env!("CARGO_BIN_EXE_a2ui-render");
    let dir = std::env::temp_dir().join(format!("a2ui-tap-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let journal = dir.join("deploy.jsonl");

    // Where the primary button is, from the layout the binary itself uses.
    let (m, _) = model_for("deploy");
    let sc = scene::layout(&m, W as i32, H as i32);
    let go = sc.items.iter().find(|it| matches!(&it.kind, scene::ItemKind::Button { id, .. } if id == "go")).expect("go button");
    let (cx, cy) = (go.rect.x + go.rect.w / 2, go.rect.y + go.rect.h / 2);

    run(
        bin,
        &[
            "--doc",
            &fixture_path("deploy").to_string_lossy(),
            "--focus",
            "note",
            "--text",
            "ship it",
            "--click",
            &format!("{cx},{cy}"),
            "--events",
            &journal.to_string_lossy(),
            "--exit-after",
            "150",
        ],
        true,
    );

    let text = std::fs::read_to_string(&journal).expect("the binary wrote the journal");
    let events: Vec<serde_json::Value> = text.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert_eq!(events.len(), 2, "one edit, one tap, in the order given: {text}");
    assert_eq!(events[0]["payload"]["name"], serde_json::json!("a2ui.input"));
    assert_eq!(events[0]["payload"]["value"]["value"], serde_json::json!("ship it"));
    assert_eq!(events[1]["payload"]["name"], serde_json::json!("a2ui.action"));
    assert_eq!(events[1]["payload"]["value"]["action"]["name"], serde_json::json!("deploy.confirm"));
    // The typed note reached the agent through the action's resolved context.
    assert_eq!(events[1]["payload"]["value"]["action"]["context"]["note"], serde_json::json!("ship it"));
    assert_eq!(events[1]["payload"]["value"]["action"]["context"]["env"], serde_json::json!("staging"));
    assert_eq!(events[1]["seq"], serde_json::json!(1), "the journal is gapless from 0");
    let _ = std::fs::remove_dir_all(&dir);
}
