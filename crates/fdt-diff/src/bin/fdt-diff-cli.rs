//! HW-25: `fdt-diff-cli` — command-line diff of two real DTB files.
//!
//! A THIN wrapper over the already-proven library (HW-07…HW-22): it reads two
//! blobs, calls the crate's public compare entry points, renders the verdict
//! via `DiffReport::verdict()` (no duplicated verdict logic), prints up to 20
//! diff lines, and maps the outcome onto a stable exit-code contract:
//!
//!   0 = EQUAL            (byte-exact inputs)
//!   1 = DIFFERENT        (structural diffs found; complete or truncated walk)
//!   2 = INCOMPLETE       (walk/budget truncation with zero emitted diffs)
//!   3 = usage/parse error (bad args, unreadable file, malformed DTB)
//!
//! Usage: fdt-diff-cli <A.dtb> <B.dtb> [--cap N]
//! `--cap N` routes through `compare_capped` (the test-visible cap wrapper);
//! without it the comparison runs uncapped via `compare`.

use std::process::ExitCode;

const EXIT_EQUAL: u8 = 0;
const EXIT_DIFFERENT: u8 = 1;
const EXIT_INCOMPLETE: u8 = 2;
const EXIT_USAGE: u8 = 3;

/// The subtree the CLI diffs. "/" is the whole tree — the same scope HW-22
/// used on the real pair.
const SUBTREE: &str = "/";

fn usage() -> ! {
    eprintln!("usage: fdt-diff-cli <A.dtb> <B.dtb> [--cap N]");
    eprintln!("exit codes: 0 EQUAL, 1 DIFFERENT, 2 INCOMPLETE, 3 usage/parse error");
    std::process::exit(EXIT_USAGE as i32)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    // Parse: exactly two positionals, optional --cap N. Any deviation is a
    // usage error (exit 3) — including unknown flags and unparsable caps.
    let mut positional: Vec<String> = Vec::new();
    let mut cap: Option<usize> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--cap" => {
                if cap.is_some() || i + 1 >= args.len() {
                    usage();
                }
                match args[i + 1].parse::<usize>() {
                    Ok(n) => cap = Some(n),
                    Err(_) => usage(),
                }
                i += 2;
            }
            "-h" | "--help" => usage(),
            s if s.starts_with('-') && s.len() > 1 => usage(),
            s => {
                if positional.len() >= 2 {
                    usage();
                }
                positional.push(s.to_string());
                i += 1;
            }
        }
    }
    if positional.len() != 2 {
        usage();
    }

    // File reads: any IO failure is a parse-side error (exit 3).
    let a = match std::fs::read(&positional[0]) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: cannot read {}: {e}", positional[0]);
            return ExitCode::from(EXIT_USAGE);
        }
    };
    let b = match std::fs::read(&positional[1]) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: cannot read {}: {e}", positional[1]);
            return ExitCode::from(EXIT_USAGE);
        }
    };

    // The comparison itself — uncapped by default, capped with --cap.
    let subtrees = [SUBTREE];
    let report = match cap {
        None => fdt_diff::compare(&a, &b, &subtrees),
        Some(n) => fdt_diff::compare_capped(&a, &b, &subtrees, n),
    };
    let report = match report {
        Ok(r) => r,
        Err((side, e)) => {
            eprintln!(
                "error: side {side} ({}) did not parse: {e:?}",
                positional[side]
            );
            return ExitCode::from(EXIT_USAGE);
        }
    };

    // Verdict line comes from the crate's own rendering — never re-derived.
    println!("{}", report.verdict());

    match &report {
        // The exit-code contract names three verdicts. The crate has a
        // fourth — `StructurallyEqual` (bytes differ, complete walk finds
        // no structural diff). It is NOT Equal and NOT budget-blind, but
        // the contract has no code for it; 2 (not-proven-equal) is the
        // closest honest mapping. Documented in HW-DTB.md.
        fdt_diff::DiffReport::StructurallyEqual { .. } => ExitCode::from(EXIT_INCOMPLETE),
        fdt_diff::DiffReport::Equal => ExitCode::from(EXIT_EQUAL),
        fdt_diff::DiffReport::Incomplete { .. } => ExitCode::from(EXIT_INCOMPLETE),
        fdt_diff::DiffReport::Different { diffs, .. } => {
            for d in diffs.iter().take(20) {
                println!("{}", render_diff(d));
            }
            ExitCode::from(EXIT_DIFFERENT)
        }
    }
}

/// Render one `Diff` as `kind path [name]`. Field names mirror the enum so
/// the output is greppable against the library's semantics.
fn render_diff(d: &fdt_diff::Diff) -> String {
    use fdt_diff::Diff::*;
    match d {
        MissingInB { path } => format!("MissingInB {path}"),
        MissingInA { path } => format!("MissingInA {path}"),
        PropMissingInB { path, name } => format!("PropMissingInB {path} {name}"),
        PropMissingInA { path, name } => format!("PropMissingInA {path} {name}"),
        ValueMismatch { path, name, .. } => format!("ValueMismatch {path} {name}"),
        NameMismatch {
            parent,
            a_name,
            b_name,
        } => format!("NameMismatch {parent} {a_name}|{b_name}"),
    }
}
