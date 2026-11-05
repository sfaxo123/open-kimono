//! `svcd` command line (Linux dev host):
//!
//! - `svcd validate <ir.json>...` — load + validate, print precise errors.
//! - `svcd simulate <ir.json> [--trace <file>]` — run the IR's embedded tests
//!   on the virtual clock (`svc:simulate`), print a verdict per test, and
//!   optionally write the journal as trace-events JSONL.
//!
//! The Redox `svc:` scheme daemon is not written yet (see README).

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.split_first() {
        Some((cmd, files)) if cmd == "validate" && !files.is_empty() => validate(files),
        Some((cmd, rest)) if cmd == "simulate" && !rest.is_empty() => simulate(rest),
        _ => {
            eprintln!("usage: svcd validate <ir.json>...\n       svcd simulate <ir.json> [--trace <out.trace>]");
            ExitCode::from(2)
        }
    }
}

fn read(f: &str) -> Option<String> {
    std::fs::read_to_string(f).map_err(|e| eprintln!("{f}: {e}")).ok()
}

fn validate(files: &[String]) -> ExitCode {
    let mut ok = true;
    for f in files {
        let Some(text) = read(f) else {
            ok = false;
            continue;
        };
        match svcd::load(&text) {
            Ok(ir) => println!("{f}: OK ({} v{}, {} triggers, {} actions)", ir.id, ir.version, ir.triggers.len(), ir.actions.len()),
            Err(errs) => {
                ok = false;
                print!("{f}: {errs}");
            }
        }
    }
    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn simulate(args: &[String]) -> ExitCode {
    let file = &args[0];
    let trace_out = args.iter().position(|a| a == "--trace").and_then(|i| args.get(i + 1));
    let Some(text) = read(file) else { return ExitCode::FAILURE };
    let ir = match svcd::load(&text) {
        Ok(ir) => ir,
        Err(errs) => {
            print!("{file}: {errs}");
            return ExitCode::FAILURE;
        }
    };
    let t0 = std::time::Instant::now();
    let reports = svcd::sim::simulate(&ir);
    let host_us = t0.elapsed().as_micros();
    let mut journal = String::new();
    let mut ok = true;
    for r in &reports {
        let verdict = if r.passed() { "PASS" } else { "FAIL" };
        println!("{verdict} {} :: {} (virtual end {} ms, {} journal events, {} bus messages)", r.service, r.test, r.end_ms, r.journal.len(), r.bus.len());
        for m in &r.bus {
            println!("  bus @{:>7} ms  {:<32} {}", m.at_ms, m.msg_type, m.data);
        }
        println!(
            "  final state {}\n  peak memory {} B, network {} B, active schemes after {:?}",
            r.final_state, r.peak_memory_bytes, r.network_bytes, r.active_schemes
        );
        for f in &r.failures {
            println!("  FAILED: {f}");
        }
        ok &= r.passed();
        journal.push_str(&trace_events::to_jsonl(&r.journal).expect("journal serializes"));
    }
    println!("{} test(s), host time {host_us} µs", reports.len());
    if let Some(out) = trace_out {
        if let Err(e) = std::fs::write(out, journal) {
            eprintln!("{out}: {e}");
            return ExitCode::FAILURE;
        }
    }
    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
