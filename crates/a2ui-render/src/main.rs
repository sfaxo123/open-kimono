//! The A2UI front end (task C6): render a declarative UI document on the
//! phone in the OrbClient black+orange style, and journal every interaction.
//!
//! ```text
//! a2ui-render --doc <path|->             render one A2UI document
//! a2ui-render --demo                     render the bundled deploy fixture
//! a2ui-render --screenshot <png>         render off-screen and write a PNG
//! a2ui-render --click <x,y>              press at a point (repeatable, scripted)
//! a2ui-render --focus <id>               put the keyboard in a field
//! a2ui-render --text <chars>             type into the focused field (repeatable)
//! a2ui-render --events <path|->          write the journal as JSONL
//! a2ui-render --trace <32 hex>           tag the journal with a trace id
//! a2ui-render --exit-after <ms>          close the window after ms (evidence runs)
//! a2ui-render --width <n> --height <n>   window size (default 480x800)
//! ```
//!
//! With no display the window open fails and the run degrades to a headless
//! pass: lay out, draw to an off-screen `Surface`, write the PNG. Same draw
//! code either way, so an evidence image is the app's actual frame.

use std::io::Read as _;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use orbclient::{EventOption, Renderer, Surface, Window};
use trace_events::{to_jsonl, EventRing, TraceId};

use a2ui_render::{doc, events, png, scene, surface::Surface as Model, view};

const DEMO_DOC: &str = include_str!("../fixtures/deploy.json");

/// One scripted interaction, kept in command-line order so a run reads as a
/// story: focus this field, type, then press.
#[derive(Clone, Debug)]
enum Op {
    Click(i32, i32),
    Focus(String),
    Text(String),
}

struct Args {
    source: Option<String>,
    screenshot: Option<PathBuf>,
    script: Vec<Op>,
    events_out: Option<String>,
    trace: Option<TraceId>,
    exit_after: Option<Duration>,
    width: u32,
    height: u32,
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args {
        source: None,
        screenshot: None,
        script: Vec::new(),
        events_out: None,
        trace: None,
        exit_after: None,
        width: 480,
        height: 800,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut need = |name: &str| -> Result<String, String> {
            it.next().ok_or_else(|| format!("{name} needs a value"))
        };
        match arg.as_str() {
            "--doc" => a.source = Some(need("--doc")?),
            "--demo" => a.source = Some("--demo".into()),
            "--screenshot" => a.screenshot = Some(PathBuf::from(need("--screenshot")?)),
            "--click" => {
                let v = need("--click")?;
                let (x, y) = v.split_once(',').ok_or("--click wants x,y")?;
                let (x, y) = (x.parse().map_err(|_| "--click wants x,y")?, y.parse().map_err(|_| "--click wants x,y")?);
                a.script.push(Op::Click(x, y));
            }
            "--focus" => a.script.push(Op::Focus(need("--focus")?)),
            "--text" => a.script.push(Op::Text(need("--text")?)),
            "--events" => a.events_out = Some(need("--events")?),
            "--trace" => {
                let v = need("--trace")?;
                a.trace = Some(TraceId::from_hex(&v).ok_or("--trace wants 32 hex characters")?);
            }
            "--exit-after" => {
                let ms: u64 = need("--exit-after")?.parse().map_err(|_| "--exit-after wants ms")?;
                a.exit_after = Some(Duration::from_millis(ms));
            }
            "--width" => a.width = need("--width")?.parse().map_err(|_| "--width wants px")?,
            "--height" => a.height = need("--height")?.parse().map_err(|_| "--height wants px")?,
            "--help" | "-h" => {
                println!(
                    "usage: a2ui-render [--doc <path|-> | --demo] [--screenshot <png>]\n\
                     \t       [--click <x,y>] [--focus <id>] [--text <chars>] [--events <path|->]\n\
                     \t       [--trace <hex32>] [--exit-after <ms>] [--width <n>] [--height <n>]"
                );
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument {other} (try --help)")),
        }
    }
    if a.source.is_none() {
        a.source = Some("--demo".into()); // the app must never open blank
    }
    Ok(a)
}

/// Is there a display to open? An explicit SDL_VIDEODRIVER (notably `dummy`,
/// the evidence driver) counts as one: it runs the real event loop off-screen.
fn display_available() -> bool {
    std::env::var_os("SDL_VIDEODRIVER").is_some_and(|v| !v.is_empty())
        || std::env::var_os("DISPLAY").is_some_and(|v| !v.is_empty())
        || std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty())
}

fn now_us() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_micros() as u64).unwrap_or(0)
}

/// Read the document: a JSON document (object, array, or `{"messages": []}`)
/// or a stream of one message per line.
fn load(source: &str) -> Result<String, Box<dyn std::error::Error>> {
    if source == "--demo" {
        return Ok(DEMO_DOC.to_string());
    }
    if source == "-" {
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf)?;
        return Ok(buf);
    }
    Ok(std::fs::read_to_string(source)?)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = parse_args()?;
    let text = load(args.source.as_deref().unwrap_or("--demo"))?;

    // A document first; if that yields nothing, treat the input as JSONL.
    let parsed = doc::parse_document(&text);
    let parsed = if parsed.messages.is_empty() && !parsed.errors.is_empty() {
        let j = doc::parse_jsonl(&text);
        if j.messages.len() >= parsed.messages.len() {
            j
        } else {
            parsed
        }
    } else {
        parsed
    };
    for e in &parsed.errors {
        match e.index {
            Some(i) => eprintln!("message {i} rejected: {}", e.message),
            None => eprintln!("document rejected: {}", e.message),
        }
    }
    if parsed.messages.is_empty() {
        return Err("no usable A2UI messages in the input".into());
    }

    let mut model = Model::default();
    let mut surface_id = String::from("default");
    for m in &parsed.messages {
        if let doc::Message::CreateSurface { surface_id: id, .. } = m {
            surface_id = id.clone();
        }
        model.apply(m)?;
    }

    let mut ring = EventRing::new(512);
    let mut emitter = events::Emitter::new(surface_id.clone());
    if let Some(t) = args.trace {
        emitter = emitter.trace(t);
    }
    let mut focus: Option<String> = None;

    // Say out loud what the document asked for that this renderer cannot do.
    report(&layout(&model, &focus, &args));

    // Scripted interactions, in the order they were given, so a run is
    // reproducible from the command line alone.
    for op in &args.script {
        let sc = layout(&model, &focus, &args);
        match op {
            Op::Click(x, y) => {
                if let events::Tap::Focus { component_id } = emitter.tap(&sc, *x, *y, now_us(), &mut ring) {
                    focus = Some(component_id);
                }
            }
            Op::Focus(id) => focus = Some(id.clone()),
            Op::Text(t) => {
                if let Some(id) = typing_target(&sc, &focus) {
                    let merged = format!("{}{t}", field_value(&sc, &id));
                    model.set_field_value(&id, &merged);
                    emitter.typed(&sc, &id, &merged, now_us(), &mut ring);
                }
            }
        }
    }

    // orbclient panics inside SDL_Init when there is no display to open, so
    // decide before calling it: no display means the headless pass below.
    let window = if display_available() {
        Window::new(64, 64, args.width, args.height, &format!("a2ui · {surface_id}"))
    } else {
        None
    };
    let mut window = match window {
        Some(w) => w,
        None => {
            // No display: draw off-screen and finish.
            let sc = layout(&model, &focus, &args);
            let path = args.screenshot.clone().unwrap_or_else(|| PathBuf::from("a2ui-screenshot.png"));
            let mut canvas = Surface::new(args.width, args.height).ok_or("cannot create off-screen surface")?;
            view::render(&mut canvas, &sc);
            png::write_png(&path, args.width, args.height, canvas.data())?;
            eprintln!(
                "headless: {} items, content {} px in {} px, screenshot {}",
                sc.items.len(),
                sc.content_height,
                args.height,
                path.display(),
            );
            return finish(&args, &ring);
        }
    };
    // orbclient's events() BLOCKS in sync mode — the frame loop needs polling.
    window.set_async(true);

    let start = Instant::now();
    let mut mouse = (0i32, 0i32);
    let mut quit = false;
    while !quit {
        for event in window.events() {
            match event.to_option() {
                EventOption::Quit(_) => quit = true,
                EventOption::Mouse(m) => mouse = (m.x, m.y),
                EventOption::Button(button) if button.left => {
                    let sc = layout(&model, &focus, &args);
                    let tap = emitter.tap(&sc, mouse.0, mouse.1, now_us(), &mut ring);
                    if let events::Tap::Focus { component_id } = tap {
                        focus = Some(component_id);
                    }
                }
                EventOption::TextInput(input) => {
                    let sc = layout(&model, &focus, &args);
                    if let Some(id) = typing_target(&sc, &focus) {
                        let merged = format!("{}{}", field_value(&sc, &id), input.character);
                        model.set_field_value(&id, &merged);
                        emitter.typed(&sc, &id, &merged, now_us(), &mut ring);
                    }
                }
                _ => {}
            }
        }

        let sc = layout(&model, &focus, &args);
        view::render(&mut window, &sc);
        window.sync();

        if let Some(limit) = args.exit_after {
            if start.elapsed() >= limit {
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(16));
    }

    if let Some(path) = &args.screenshot {
        let (w, h) = (window.width(), window.height());
        let mut canvas = Surface::new(w, h).ok_or("cannot create off-screen surface")?;
        let sc = layout(&model, &focus, &args);
        view::render(&mut canvas, &sc);
        png::write_png(path, w, h, canvas.data())?;
        eprintln!("screenshot: {}", path.display());
    }
    finish(&args, &ring)?;
    // orbclient's Window Drop joins an event thread; under SDL's dummy driver
    // that join never returns, so leave the process here.
    std::process::exit(0);
}

fn layout(model: &Model, focus: &Option<String>, args: &Args) -> scene::Scene {
    let opts = scene::Options { focus: focus.as_deref() };
    scene::layout_with(model, args.width as i32, args.height as i32, &opts)
}

/// The field the keyboard belongs to: the focused one, else the one named by
/// `--focus` (so a scripted run can type without a press).
fn typing_target(sc: &scene::Scene, focus: &Option<String>) -> Option<String> {
    if let Some(item) = sc.focused_field() {
        if let scene::ItemKind::Field { id, .. } = &item.kind {
            return Some(id.clone());
        }
    }
    focus.clone()
}

/// The text a field currently shows, as laid out (model value or local edit).
fn field_value(sc: &scene::Scene, id: &str) -> String {
    sc.items
        .iter()
        .find_map(|it| match &it.kind {
            scene::ItemKind::Field { id: fid, value, .. } if fid == id => Some(value.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

/// What the document asked for that this renderer cannot do, once per run.
fn report(sc: &scene::Scene) {
    for u in &sc.unsupported {
        eprintln!("unsupported component: {u}");
    }
    for d in &sc.diagnostics {
        eprintln!("diagnostic: {d}");
    }
}

fn finish(args: &Args, ring: &EventRing) -> Result<(), Box<dyn std::error::Error>> {
    let text = to_jsonl(&ring.snapshot())?;
    match args.events_out.as_deref() {
        Some("-") => print!("{text}"),
        Some(p) => std::fs::write(p, text)?,
        None => {}
    }
    eprintln!("journal: {} events", ring.len());
    Ok(())
}
