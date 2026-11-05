//! The timeline binary (task C4): one open-kimono screen fed by trace-events
//! JSONL.
//!
//! ```text
//! timeline --demo                        replay the fixture stream in a window
//! timeline --stream <path|->             tail a live journal (file or stdin)
//! timeline --screenshot <png>            render the final frame to a PNG
//! timeline --exit-after <ms>             close the window after ms (evidence runs)
//! timeline --speed <f> / --instant       demo replay pacing
//! timeline --width <n> --height <n>      window size (default 480x800)
//! ```
//!
//! With no display available the window open fails and the run degrades to a
//! headless pass: apply every event, render to an off-screen `Surface`, write
//! the PNG — the same draw code, so the evidence image is the app's frame.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use orbclient::{EventOption, Renderer, Surface, Window};
use trace_events::Event;

use timeline::{feed::Feed, model::TimelineModel, png::write_png, view};

const DEMO_FIXTURE: &str = include_str!("../fixtures/demo.jsonl");

struct Args {
    demo: bool,
    stream: Option<String>,
    screenshot: Option<PathBuf>,
    exit_after: Option<Duration>,
    speed: f64,
    instant: bool,
    width: u32,
    height: u32,
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args {
        demo: false,
        stream: None,
        screenshot: None,
        exit_after: None,
        speed: 1.0,
        instant: false,
        width: 480,
        height: 800,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut need = |name: &str| -> Result<String, String> {
            it.next().ok_or_else(|| format!("{name} needs a value"))
        };
        match arg.as_str() {
            "--demo" => a.demo = true,
            "--stream" => a.stream = Some(need("--stream")?),
            "--screenshot" => a.screenshot = Some(PathBuf::from(need("--screenshot")?)),
            "--exit-after" => {
                let ms: u64 = need("--exit-after")?.parse().map_err(|_| "--exit-after wants ms")?;
                a.exit_after = Some(Duration::from_millis(ms));
            }
            "--speed" => {
                a.speed = need("--speed")?.parse().map_err(|_| "--speed wants a number")?;
                if a.speed.is_nan() || a.speed <= 0.0 {
                    return Err("--speed must be > 0".into());
                }
            }
            "--instant" => a.instant = true,
            "--width" => a.width = need("--width")?.parse().map_err(|_| "--width wants px")?,
            "--height" => a.height = need("--height")?.parse().map_err(|_| "--height wants px")?,
            "--help" | "-h" => {
                println!("usage: timeline [--demo | --stream <path|->] [--screenshot <png>]\n\
                          \t      [--exit-after <ms>] [--speed <f> | --instant] [--width <n>] [--height <n>]");
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument {other} (try --help)")),
        }
    }
    if a.stream.is_none() && !a.demo {
        a.demo = true; // no source named: show the demo, the app must never be blank
    }
    if a.stream.is_some() && a.demo {
        return Err("pick --demo or --stream, not both".into());
    }
    Ok(a)
}

/// Replay pacing: the gap between two journal timestamps, scaled and capped.
fn replay_gap(prev_ts_us: u64, next_ts_us: u64, speed: f64) -> Duration {
    let delta_us = next_ts_us.saturating_sub(prev_ts_us);
    let ms = (delta_us as f64 / 1000.0 / speed).min(400.0);
    Duration::from_millis(ms as u64)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = parse_args()?;
    let mut model = TimelineModel::default();
    let mut bad_lines: u64 = 0;

    // Demo events (parsed up front so replay can pace on ts_us).
    let demo_events: Vec<Event> = if args.demo {
        let batch = timeline::feed::parse_chunk(DEMO_FIXTURE);
        bad_lines += batch.bad_lines;
        batch.events
    } else {
        Vec::new()
    };

    let mut feed = match &args.stream {
        Some(p) if p == "-" => Some(Feed::stdin()),
        Some(p) if p.starts_with("trace:") => Some(Feed::scheme(p)?),
        Some(p) => Some(Feed::file(p)?),
        None => None,
    };

    // Window, or a headless pass when no display exists.
    let window = Window::new(64, 64, args.width, args.height, "timeline");
    let mut window = match window {
        Some(w) => w,
        None if (args.screenshot.is_some() || args.stream.is_none()) && !cfg!(target_os = "redox") => {
            // Headless: apply everything now, render off-screen, done.
            for e in &demo_events {
                model.apply(e);
            }
            if let Some(f) = feed.as_mut() {
                let batch = f.poll()?;
                bad_lines += batch.bad_lines;
                for e in batch.events {
                    model.apply(&e);
                }
            }
            return finish_headless(&args, &model, bad_lines);
        }
        None => return Err("cannot open a window and no --screenshot asked".into()),
    };
    // orbclient's events() BLOCKS in sync mode (wait_event) — the frame loop
    // needs the polling variant.
    window.set_async(true);

    let start = Instant::now();
    let mut demo_idx = 0usize;
    let mut due = start;
    let mut quit = false;

    while !quit {
        for event in window.events() {
            if matches!(event.to_option(), EventOption::Quit(_)) {
                quit = true;
            }
        }

        if args.demo {
            let now = Instant::now();
            while demo_idx < demo_events.len() && (args.instant || now >= due) {
                let e = &demo_events[demo_idx];
                model.apply(e);
                demo_idx += 1;
                if demo_idx < demo_events.len() && !args.instant {
                    let gap = replay_gap(demo_events[demo_idx - 1].ts_us, demo_events[demo_idx].ts_us, args.speed);
                    // Anchor to the schedule, not the wall clock: frame
                    // overhead must not accumulate into replay drift.
                    due += gap;
                } else {
                    due = Instant::now();
                }
            }
        } else if let Some(f) = feed.as_mut() {
            let batch = f.poll()?;
            bad_lines += batch.bad_lines;
            for e in batch.events {
                model.apply(&e);
            }
        }

        view::render(&mut window, &model);
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
        let mut surface = Surface::new(w, h).ok_or("cannot create off-screen surface")?;
        view::render(&mut surface, &model);
        write_png(path, w, h, surface.data())?;
        eprintln!("screenshot: {}", path.display());
    }
    eprintln!("events applied: {}  bad lines: {}", model.events_applied, bad_lines);
    // orbclient's Window spawns an event thread and its Drop joins it; under
    // SDL's dummy driver that join never returns, so exit the process here.
    std::process::exit(0);
}

fn finish_headless(args: &Args, model: &TimelineModel, bad_lines: u64) -> Result<(), Box<dyn std::error::Error>> {
    let path = args
        .screenshot
        .clone()
        .unwrap_or_else(|| PathBuf::from("timeline-screenshot.png"));
    let mut surface = Surface::new(args.width, args.height).ok_or("cannot create off-screen surface")?;
    view::render(&mut surface, model);
    write_png(&path, args.width, args.height, surface.data())?;
    eprintln!(
        "headless: events applied {}  bad lines {}  screenshot {}",
        model.events_applied,
        bad_lines,
        path.display()
    );
    Ok(())
}
