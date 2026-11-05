//! op6-boot CLI — verify / plan / attempt. See `lib.rs` for the safety
//! argument. This binary NEVER runs a state-changing fastboot verb; the
//! attempt path is consent-double-gated and verb-whitelisted by construction.

use std::env;
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use op6_boot::{
    attempt_run, check_consent, plan_text, AttemptOutcome, ConsentGate, ImagePin, RealFastboot,
    choose_pin, CARRIER_IMG, CARRIER_V2_IMG, CARRIER_V3_IMG, REFUSAL_TEXT, STAGED_IMAGES, V0_IMG,
};

const USAGE: &str = "\
op6-boot — Rust replacement for BOOT-ATTEMPT.sh (HW-20). NEVER run here.

USAGE:
  op6-boot verify [--carrier <path>] [--v2 <path>] [--v0 <path>]
        Streamed sha256 + size check of ALL staged images (STAGED_IMAGES
        order) against embedded pins; prints a receipt table. A missing
        image is a FAIL, never a silent skip. Read-only, no phone contact.

  op6-boot plan
        Prints the exact operator-hands checklist and the single permitted
        command line (`fastboot boot <carrier img>`, v0 fallback documented).
        Runs NOTHING.

  op6-boot attempt --operator-consent <token>   (AND env OP6_BOOT_CONSENT=<token>)
        HARD-GATED. Refuses unless BOTH consent inputs carry the SAME token.
        When open: pin-verify, `fastboot devices` (selection contract: exactly
        one line in state `fastboot`; optional --expect-serial must match),
        re-verify, then `fastboot boot <chosen img>`. No other verb is
        expressible. Exit codes: 0 boot-ok, 1 pre-flight/consent/devices or
        selection abort, 4 boot process FAILED (distinct from aborts).

Default image paths are the 3090 staging kit: ~/op6-staging-20261028/.";

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let sub = args.first().map(String::as_str).unwrap_or("");
    let code = match sub {
        "verify" => cmd_verify(&args[1..]),
        "plan" => {
            print!("{}", plan_text());
            0
        }
        "attempt" => cmd_attempt(&args[1..]),
        "--help" | "-h" | "help" | "" => {
            println!("{USAGE}");
            if sub.is_empty() {
                2
            } else {
                0
            }
        }
        other => {
            eprintln!("unknown subcommand: {other}\n\n{USAGE}");
            2
        }
    };
    std::process::exit(code);
}

// ---------------------------------------------------------------------------
// verify
// ---------------------------------------------------------------------------

/// Result of checking one file against one pin.
struct Check {
    pin: ImagePin,
    path: String,
    found_size: Option<u64>,
    found_sha: Option<String>,
    pass: bool,
    note: String,
}

fn hash_file(path: &Path) -> io::Result<(u64, [u8; 32])> {
    let mut f = File::open(path)?;
    let mut ctx = op6_boot::sha256::Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    let mut total: u64 = 0;
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        ctx.update(&buf[..n]);
        total += n as u64;
    }
    Ok((total, ctx.finalize()))
}

fn check_path(pin: &ImagePin, path: &str) -> Check {
    match hash_file(Path::new(path)) {
        Err(e) => Check {
            pin: *pin,
            path: path.to_string(),
            found_size: None,
            found_sha: None,
            pass: false,
            note: format!("cannot read: {e}"),
        },
        Ok((size, digest)) => {
            let sha = op6_boot::sha256::hex(&digest);
            let size_ok = size == pin.size;
            let sha_ok = sha == pin.sha256;
            let mut note = String::new();
            if !size_ok {
                note.push_str(&format!("size {size} != pinned {}", pin.size));
            }
            if !sha_ok {
                if !note.is_empty() {
                    note.push_str("; ");
                }
                note.push_str("sha256 MISMATCH — do NOT boot this file");
            }
            Check {
                pin: *pin,
                path: path.to_string(),
                found_size: Some(size),
                found_sha: Some(sha),
                pass: size_ok && sha_ok,
                note,
            }
        }
    }
}

/// Resolve the default staging paths (the 3090 kit from HW-PREP-BOOT §9).
fn default_paths() -> (String, String, String, String) {
    let home = env::var("HOME").unwrap_or_else(|_| "$HOME".to_string());
    let dir = format!("{home}/op6-staging-20261028");
    // The v2 carrier is not part of the staging kit; its default is the
    // repo out/attempt2 path under HOME, overridable with OP6_BOOT_V2_IMG
    // for any other checkout layout (Wonder diff bb6b111).
    let v2 = env::var("OP6_BOOT_V2_IMG").unwrap_or_else(|_| {
        format!(
            "{home}/Developer/experiments/redox-oneplus6/out/attempt2/{}",
            CARRIER_V2_IMG.name
        )
    });
    // The v3 carrier likewise defaults to the repo out/probe2 path,
    // overridable with OP6_BOOT_V3_IMG.
    let v3 = env::var("OP6_BOOT_V3_IMG").unwrap_or_else(|_| {
        format!(
            "{home}/Developer/experiments/redox-oneplus6/out/probe2/{}",
            CARRIER_V3_IMG.name
        )
    });
    (
        format!("{dir}/{}", CARRIER_IMG.name),
        v2,
        v3,
        format!("{dir}/{}", V0_IMG.name),
    )
}

fn cmd_verify(args: &[String]) -> i32 {
    let (mut carrier, mut v2, mut v3, mut v0) = default_paths();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--carrier" if i + 1 < args.len() => {
                carrier = args[i + 1].clone();
                i += 2;
            }
            "--v2" if i + 1 < args.len() => {
                v2 = args[i + 1].clone();
                i += 2;
            }
            "--v3" if i + 1 < args.len() => {
                v3 = args[i + 1].clone();
                i += 2;
            }
            "--v0" if i + 1 < args.len() => {
                v0 = args[i + 1].clone();
                i += 2;
            }
            other => {
                eprintln!("verify: unexpected argument: {other}");
                return 2;
            }
        }
    }

    // The receipt list is DERIVED from STAGED_IMAGES (Wonder diff
    // bb6b111): reordering or extending the constant changes verify
    // with it. Paths resolve per pin name; an unknown future staged
    // image defaults to the staging-kit directory (the carrier's
    // parent). A missing file FAILs in check_path, never skips.
    let kit_dir = carrier
        .rsplit_once('/')
        .map(|(d, _)| d.to_string())
        .unwrap_or_default();
    let path_for = |pin: &ImagePin| -> String {
        if pin.name == CARRIER_IMG.name {
            carrier.clone()
        } else if pin.name == CARRIER_V2_IMG.name {
            v2.clone()
        } else if pin.name == CARRIER_V3_IMG.name {
            v3.clone()
        } else if pin.name == V0_IMG.name {
            v0.clone()
        } else {
            format!("{kit_dir}/{}", pin.name)
        }
    };
    let checks: Vec<Check> = STAGED_IMAGES
        .iter()
        .map(|pin| check_path(pin, &path_for(pin)))
        .collect();

    println!("op6-boot verify — receipt table (pins: HW-PREP-BOOT §2/§9)");
    println!(
        "{:<28} {:>10}  {:<8}  {}",
        "IMAGE", "SIZE", "SHA256", "VERDICT"
    );
    println!("{}", "-".repeat(72));
    let mut all_pass = true;
    for c in &checks {
        let size_s = match c.found_size {
            Some(s) => format!("{s}"),
            None => "-".to_string(),
        };
        let sha_s = if c.pass {
            "MATCH".to_string()
        } else if c.found_sha.is_some() {
            "DRIFT".to_string()
        } else {
            "-".to_string()
        };
        println!(
            "{:<28} {:>10}  {:<8}  {}",
            c.pin.name,
            size_s,
            sha_s,
            if c.pass { "PASS" } else { "FAIL" }
        );
        println!("    path: {}", c.path);
        println!("    pin : {} ({} B)", c.pin.sha256, c.pin.size);
        if let Some(sha) = &c.found_sha {
            println!("    now : {sha}");
        }
        if !c.note.is_empty() {
            println!("    note: {}", c.note);
        }
        all_pass &= c.pass;
    }
    println!("{}", "-".repeat(72));
    println!(
        "drift count: {} — {}",
        checks.iter().filter(|c| !c.pass).count(),
        if all_pass {
            "ALL PASS: images match the pins."
        } else {
            "FAIL: do NOT proceed to any phone step on a FAIL row."
        }
    );
    if all_pass {
        0
    } else {
        1
    }
}

// ---------------------------------------------------------------------------
// attempt (hard-gated)
// ---------------------------------------------------------------------------

fn cmd_attempt(args: &[String]) -> i32 {
    let mut flag_token: Option<String> = None;
    let mut img: Option<String> = None;
    let mut expect_serial: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--operator-consent" if i + 1 < args.len() => {
                flag_token = Some(args[i + 1].clone());
                i += 2;
            }
            "--img" if i + 1 < args.len() => {
                img = Some(args[i + 1].clone());
                i += 2;
            }
            "--expect-serial" if i + 1 < args.len() => {
                expect_serial = Some(args[i + 1].clone());
                i += 2;
            }
            other => {
                eprintln!("attempt: unexpected argument: {other}");
                return 2;
            }
        }
    }

    let env_token = env::var("OP6_BOOT_CONSENT").ok();
    match check_consent(flag_token.as_deref(), env_token.as_deref()) {
        ConsentGate::Refused { reasons } => {
            eprintln!("{REFUSAL_TEXT}");
            for r in &reasons {
                eprintln!("  - {r}");
            }
            1
        }
        ConsentGate::Open { .. } => {
            let (carrier, _v2, _v3, _v0) = default_paths();
            let chosen = img.unwrap_or(carrier);

            // Thin CLI (HW-27): all decision logic lives in lib.rs
            // `attempt_run` behind the FastbootRunner seam. The pin verify is
            // passed as a closure so attempt_run can run it TWICE — once
            // pre-flight, once immediately before boot (TOCTOU mitigation;
            // a residual reopen race remains, documented in lib.rs).
            // Pin selection is by image file name: the v2 carrier is
            // checked against the v2 pin, everything else against the
            // full31 carrier pin (the pre-v2 behaviour). A file whose
            // bytes drift from its named pin fails the check either way.
            let chosen_pin: &ImagePin =
                choose_pin(Path::new(&chosen).file_name().and_then(|n| n.to_str()));
            let chosen_path = Path::new(&chosen);
            let outcome = attempt_run(&RealFastboot, chosen_path, expect_serial.as_deref(), || {
                check_path(chosen_pin, &chosen).pass
            });

            match &outcome {
                AttemptOutcome::Booted => {
                    eprintln!("attempt: `fastboot boot` reported SUCCESS. Capture the panel NOW.");
                }
                other => {
                    eprintln!("attempt: ABORT — {}", other.reason());
                }
            }
            outcome.exit_code()
        }
    }
}
