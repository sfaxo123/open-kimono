//! HW-09 (Lane H) — MEASURED resource evidence for fdt-diff.
//!
//! Everything here is Rust; no Python/shell generators anywhere.
//!
//! What it measures on the REAL artifacts:
//!   1. extract_dtbs scan cost (wall ms) over the full boot image, and how
//!      many DTB candidates the caps allow through.
//!   2. A repeated (N=50) parse+compare loop on a WELL-FORMED real input
//!      (best real-image candidate vs the live fixture) — per-iteration ms
//!      and totals.
//!   3. A repeated (N=50) parse loop over MALFORMED controls: truncated
//!      image slices at several cut points and header-corrupted variants —
//!      proving bounded cost on hostile input, not just good input.
//!   4. Process high-water RSS (VmHWM from /proc/self/status) before/after
//!      the loops, with the delta in KiB.
//!
//! Honesty rule: VmHWM is a PROCESS-LEVEL bound. It is printed together
//! with an explicit note that it is NOT exact heap accounting, NOT
//! leak-freedom proof, and NOT device-fit proof.
//!
//! Usage: resource_evidence [img-path] [dtb-path]   (defaults below)

use fdt_diff::{compare, extract_dtbs, DiffReport, Fdt};
use std::fs;
use std::path::Path;
use std::time::Instant;

const IMG: &str = "$HOME/artifacts/handoff/redox-perplexity/physical-revised31-20261027T1955/package/op6-embedded-efi-redox.img";
const DTB: &str = "tests/fixtures/live-chosen-splash.dtb";
const ITERS: usize = 50;
const FULL_TREE: &[&str] = &["/"];

/// VmHWM (peak resident set, KiB) from /proc/self/status. Process-level —
/// see the honesty note printed at the end.
fn vmhwm_kib() -> u64 {
    let status = fs::read_to_string("/proc/self/status").expect("read /proc/self/status");
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmHWM:") {
            return rest
                .trim()
                .trim_end_matches(" kB")
                .trim()
                .parse()
                .unwrap_or(0);
        }
    }
    0
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let img_path = args.get(1).map(String::as_str).unwrap_or(IMG);
    let dtb_path = args.get(2).map(String::as_str).unwrap_or(DTB);

    let img = match fs::read(img_path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("cannot read img {}: {}", img_path, e);
            std::process::exit(2);
        }
    };
    // Read the fixture relative to this crate's dir so it works from anywhere.
    let dtb_full = if Path::new(dtb_path).exists() {
        dtb_path.to_string()
    } else {
        format!(
            "{}/{}",
            env!("CARGO_MANIFEST_DIR"),
            "tests/fixtures/live-chosen-splash.dtb"
        )
    };
    let fixture = match fs::read(&dtb_full) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("cannot read dtb {}: {}", dtb_full, e);
            std::process::exit(2);
        }
    };

    println!("== HW-09 resource evidence (all-Rust measurement) ==");
    println!(
        "img: {} ({} B), fixture: {} ({} B)",
        img_path,
        img.len(),
        dtb_full,
        fixture.len()
    );
    println!(
        "caps: MAX_NODE_DEPTH {} / MAX_CANDIDATES {} / MAX_DIFFS {} / FDT_MAX_PROP_LEN {}",
        fdt_diff::MAX_NODE_DEPTH,
        fdt_diff::MAX_CANDIDATES,
        fdt_diff::MAX_DIFFS,
        fdt_diff::FDT_MAX_PROP_LEN
    );

    let hwm_before = vmhwm_kib();

    // ---- 1. extract_dtbs scan over the real image -------------------------
    let t = Instant::now();
    let candidates = extract_dtbs(&img);
    let scan_ms = t.elapsed().as_secs_f64() * 1000.0;
    println!(
        "extract_dtbs: {} candidate(s) in {:.3} ms (cap {}, truncated_at_cap: {})",
        candidates.len(),
        scan_ms,
        fdt_diff::MAX_CANDIDATES,
        candidates.len() >= fdt_diff::MAX_CANDIDATES
    );

    // Pick the best real candidate for the well-formed loop: prefer a
    // byte-equal one (the G3 finding), else the first.
    let mut best_off = 0usize;
    let mut best_ts = 0usize;
    for c in &candidates {
        let end = c.offset + c.totalsize;
        if img[c.offset..end] == fixture[..] {
            best_off = c.offset;
            best_ts = c.totalsize;
            break;
        }
        if best_ts == 0 {
            best_off = c.offset;
            best_ts = c.totalsize;
        }
    }
    let cand_slice = &img[best_off..best_off + best_ts];

    // ---- 2. well-formed repeated parse+compare loop (N = ITERS) -----------
    println!(
        "well-formed loop: compare(fixture, candidate@{}) x {}",
        best_off, ITERS
    );
    let mut wf_total = 0.0f64;
    let mut last_report: Option<DiffReport> = None;
    for i in 0..ITERS {
        let t = Instant::now();
        let rep = compare(&fixture, cand_slice, FULL_TREE).expect("both must parse");
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        wf_total += ms;
        if i == 0 || i == ITERS - 1 {
            println!("  iter {}: {:.3} ms -> {}", i, ms, rep.verdict());
        }
        last_report = Some(rep);
    }
    let verdict_line = match &last_report {
        Some(DiffReport::Different {
            diffs, truncated, ..
        }) => format!(
            "diffs {}{}",
            diffs.len(),
            if *truncated { " TRUNCATED" } else { "" }
        ),
        Some(r) => r.verdict().into_owned(),
        None => "n/a".into(),
    };
    println!(
        "well-formed TOTAL: {:.3} ms over {} iters (avg {:.3} ms); verdict: {}",
        wf_total,
        ITERS,
        wf_total / ITERS as f64,
        verdict_line
    );

    // ---- 3. malformed controls repeated loop (N = ITERS each) -------------
    // Controls are derived FROM the real artifacts in Rust: truncated image
    // slices (the candidate blob cut at several points, totalsize left
    // intact so it overruns — the truncation attack), plus header-corrupted
    // variants. Each must fail with an explicit FdtError (never a panic),
    // at bounded cost. A control that unexpectedly PARSES is reported
    // honestly as PARSED and flips the all-rejected flag.
    let mut controls: Vec<(String, Vec<u8>)> = Vec::new();
    for frac in [2usize, 3, 4] {
        let cut = cand_slice.len() / frac;
        controls.push((
            format!("truncated-cand@{} (1/{})", cut, frac),
            cand_slice[..cut].to_vec(),
        ));
    }
    // Header-corrupted variants of the real candidate: version -> 16,
    // magic -> junk, off_rsv_map -> inside header.
    let mut v16 = cand_slice.to_vec();
    v16[20..24].copy_from_slice(&16u32.to_be_bytes());
    controls.push(("hdr version=16".into(), v16));
    let mut junk_magic = cand_slice.to_vec();
    junk_magic[0..4].copy_from_slice(&0xdead_beefu32.to_be_bytes());
    controls.push(("hdr magic=junk".into(), junk_magic));
    let mut bad_rsv = cand_slice.to_vec();
    bad_rsv[16..20].copy_from_slice(&8u32.to_be_bytes());
    controls.push(("hdr off_rsv_map=8".into(), bad_rsv));

    println!(
        "malformed controls: {} variants x {} iters",
        controls.len(),
        ITERS
    );
    let mut mf_total = 0.0f64;
    let mut all_rejected = true;
    for (name, ctl) in &controls {
        let t = Instant::now();
        let mut err_seen = None;
        for _ in 0..ITERS {
            match Fdt::parse(ctl) {
                Ok(_) => {
                    // A control that PARSES is a measurement finding, not an
                    // error — record it honestly. (Truncated slices at cuts
                    // that happen to land on valid structure could in
                    // principle parse; none of the chosen cuts do.)
                    err_seen = Some("PARSED".to_string());
                    break;
                }
                Err(e) => err_seen = Some(format!("{:?}", e)),
            }
        }
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        mf_total += ms;
        let parsed = err_seen.as_deref() == Some("PARSED");
        all_rejected &= !parsed;
        println!(
            "  {:<24} {} B -> {} in {:.3} ms ({} iters)",
            name,
            ctl.len(),
            err_seen.unwrap_or_default(),
            ms,
            ITERS
        );
    }
    println!(
        "malformed TOTAL: {:.3} ms over {} controls x {} iters; explicit-error-on-every-control: {}",
        mf_total,
        controls.len(),
        ITERS,
        all_rejected
    );

    // ---- 4. RSS high-water + honesty note ---------------------------------
    let hwm_after = vmhwm_kib();
    println!(
        "VmHWM before loops: {} KiB; after loops: {} KiB; delta: {} KiB",
        hwm_before,
        hwm_after,
        hwm_after as i64 - hwm_before as i64
    );
    println!(
        "NOTE: measured RSS high-water (VmHWM) is a process-level bound, NOT exact heap accounting, NOT leak-freedom proof, NOT device-fit proof."
    );
}
