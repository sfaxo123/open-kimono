//! A0 phone smoke: print exactly one valid `trace-events` JSONL line, and
//! (with `--validate`) prove that lines like it parse back into the schema.
//!
//! Run on the OnePlus 6 (`~/agentic/bin/smoke`) after the cross-build in
//! `docs/track-a/BUILD.md`. Piping the phone's line back through
//! `smoke --validate` on the dev host is the evidence that the record is a
//! real journal entry, not just plausible-looking JSON.

use std::io::{self, BufRead, Write};

use agentd::smoke::{self, HostInfo};

fn main() {
    let validate = std::env::args().skip(1).any(|a| a == "--validate");
    if validate {
        std::process::exit(validate_stdin());
    }
    let host = HostInfo::probe();
    match smoke::smoke_line(smoke::now_us(), &host) {
        Ok(line) => println!("{line}"),
        Err(e) => {
            eprintln!("smoke: journal line serialization failed: {e}");
            std::process::exit(1);
        }
    }
}

/// Read JSONL from stdin; exit 0 if every line parses as a journal `Event`.
fn validate_stdin() -> i32 {
    let stdin = io::stdin();
    let mut count = 0usize;
    for (i, line) in stdin.lock().lines().enumerate() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                eprintln!("smoke --validate: stdin read failed on line {}: {e}", i + 1);
                return 1;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        if let Err(e) = trace_events::from_json_line(&line) {
            eprintln!("smoke --validate: line {} is not a valid journal event: {e}", i + 1);
            return 1;
        }
        count += 1;
    }
    if count == 0 {
        eprintln!("smoke --validate: no journal lines on stdin");
        return 1;
    }
    let mut out = io::stdout();
    let _ = writeln!(out, "valid: {count} journal line(s)");
    0
}
