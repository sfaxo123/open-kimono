//! `kimono-fx` (Track A2): generates the recorded JSONL fixtures the kimono
//! view is developed and tested against.
//!
//! The fixtures are NOT hand-written JSON: they are built with the real
//! `trace-events` types and serialized with `to_jsonl`, so the UI can never
//! drift from the wire format. Run `cargo run` in this directory to rewrite
//! `fixtures/*.jsonl`; `tests/fixtures_valid.rs` re-parses them and asserts
//! the vocabulary coverage.
//!
//! Usage:
//!   cargo run                 # rewrite fixtures/
//!   cargo run -- --check      # fail if the checked-in fixtures are stale

mod ev;
mod journal;
mod scene_failure;
mod scene_vocab;
mod scene_voice;
mod scene_water;

use std::path::PathBuf;

use journal::Journal;

/// Every scene the kimono view must render, in the order the tests read them.
fn scenes() -> Vec<(&'static str, Journal)> {
    vec![
        ("water_reminder", scene_water::build()),
        ("fallback_failure", scene_failure::build()),
        ("voice_mic_power", scene_voice::build()),
        ("vocab_coverage", scene_vocab::build()),
    ]
}

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

fn main() -> Result<(), String> {
    let check = std::env::args().any(|a| a == "--check");
    let dir = fixtures_dir();
    if !check {
        std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    }
    let mut stale = Vec::new();
    for (name, journal) in scenes() {
        assert!(!journal.is_empty(), "scene {name} produced no events");
        let text = journal.jsonl().map_err(|e| format!("{name}: {e}"))?;
        let path = dir.join(format!("{name}.jsonl"));
        if check {
            match std::fs::read_to_string(&path) {
                Ok(have) if have == text => println!("fresh  {}", path.display()),
                Ok(_) => stale.push(name.to_string()),
                Err(e) => return Err(format!("read {}: {e}", path.display())),
            }
        } else {
            std::fs::write(&path, &text).map_err(|e| format!("write {}: {e}", path.display()))?;
            println!("wrote {} ({} events)", path.display(), journal.len());
        }
    }
    if !stale.is_empty() {
        return Err(format!("stale fixtures: {} — run `cargo run`", stale.join(", ")));
    }
    Ok(())
}
