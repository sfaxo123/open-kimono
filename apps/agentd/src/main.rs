//! agentd entry point.
//!
//! ```text
//! agentd [--config PATH] [--trace-out FILE]     # the daemon: POST /request, GET /events
//! agentd [--config PATH] --once "text"          # one request, print the answer, exit
//! ```
//!
//! `--trace-out` appends every journal record to a JSONL file as it happens,
//! which is how a phone run leaves evidence without exposing a port: the phone
//! binds loopback only, and the file is copied back afterwards.

use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

use agentd::agent::Agent;
use agentd::config::{self, Config};
use agentd::journal::{Journal, DEFAULT_CAPACITY};
use agentd::server::Server;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut config_path = default_config_path();
    let mut trace_out: Option<PathBuf> = None;
    let mut once: Option<String> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--config" if i + 1 < args.len() => {
                config_path = PathBuf::from(&args[i + 1]);
                i += 2;
            }
            "--trace-out" if i + 1 < args.len() => {
                trace_out = Some(PathBuf::from(&args[i + 1]));
                i += 2;
            }
            "--once" if i + 1 < args.len() => {
                once = Some(args[i + 1].clone());
                i += 2;
            }
            "--help" | "-h" => {
                println!("usage: agentd [--config PATH] [--trace-out FILE] [--once \"text\"]");
                return;
            }
            other => {
                eprintln!("agentd: unrecognised argument `{other}` (try --help)");
                std::process::exit(2);
            }
        }
    }

    let text = match std::fs::read_to_string(&config_path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!(
                "agentd: cannot read config {}: {e}\n\
                 Copy apps/agentd/agentd.example.toml to that path and fill it in.",
                config_path.display()
            );
            std::process::exit(2);
        }
    };
    let cfg: Config = match config::load(&text) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("agentd: {e}");
            std::process::exit(2);
        }
    };

    let journal = Journal::new(DEFAULT_CAPACITY);
    if let Some(path) = &trace_out {
        if let Err(e) = spawn_trace_writer(&journal, path) {
            eprintln!("agentd: cannot write trace file {}: {e}", path.display());
            std::process::exit(2);
        }
    }
    let agent = Arc::new(Agent::new(cfg.clone(), journal.clone()));

    if let Some(request) = once {
        let summary = agent.handle(&request);
        let out = serde_json::json!({
            "run_id": summary.run_id,
            "answer": summary.answer,
            "error": summary.error,
            "brain": summary.brain,
            "events": journal.len(),
        });
        println!("{}", out);
        std::process::exit(if summary.error.is_some() { 1 } else { 0 });
    }

    let server = Server { config: cfg.clone(), agent, journal };
    eprintln!(
        "agentd: listening on {} ({} brain(s): {})",
        cfg.listen,
        cfg.brains.len(),
        cfg.brains.iter().map(|b| b.name.as_str()).collect::<Vec<_>>().join(" -> ")
    );
    if let Err(e) = server.serve() {
        eprintln!("agentd: serve failed: {e}");
        std::process::exit(1);
    }
}

fn default_config_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/home/user".to_string());
    PathBuf::from(home).join("agentic/agentd.toml")
}

/// Drain the journal into a JSONL file forever (append mode, flushed per line).
fn spawn_trace_writer(journal: &Journal, path: &PathBuf) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let file = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
    let rx = journal.subscribe();
    std::thread::spawn(move || {
        let mut file = file;
        while let Ok(event) = rx.recv() {
            if let Ok(line) = trace_events::to_json_line(&event) {
                let _ = writeln!(file, "{line}");
                let _ = file.flush();
            }
        }
    });
    Ok(())
}
