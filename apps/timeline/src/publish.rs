//! Publish the C4 fixture into the Redox trace journal for the E1 demo.
use std::io::Write;

const DEMO: &str = include_str!("../fixtures/demo.jsonl");

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut journal = std::fs::OpenOptions::new().write(true).open("trace:publish")?;
    let mut count = 0;
    for line in DEMO.lines().filter(|line| !line.is_empty()) {
        trace_events::from_json_line(line)?;
        writeln!(journal, "{line}")?;
        count += 1;
    }
    eprintln!("timeline-demo-publish: sent {count} events to trace:publish");
    Ok(())
}
