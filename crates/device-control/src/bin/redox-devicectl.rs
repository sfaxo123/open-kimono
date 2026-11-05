//! `redox-devicectl` — bounded host CLI consumer of the scripted fake
//! transport (DC-CLI-01E observe slice).
//!
//! Subcommands:
//! - `logs tail`: serve ALL scripted frames from a caller-provided
//!   [`Script`], decode every `Op::Logs` body through `log_record`, and
//!   print one decoded record per line in a fixed text format.
//! - `screenshot capture`: serve the scripted `Op::Screenshot` frame,
//!   decode it through `image_record`, and print one deterministic summary
//!   line (dimensions, sizes, FNV-1a-64 checksum of the pixel payload).
//! - `--transport usb`: `fake_transport::open` with `TransportKind::Usb`
//!   prints an explicit `UNSUPPORTED-TRANSPORT` line and exits 2 — no
//!   fallback to the scripted transport.
//!
//! The output is fully deterministic: no wall-clock, no randomness, no
//! real device, USB or phone code anywhere. Two consecutive runs of the
//! same invocation are byte-identical. This CLI talks to the scripted fake
//! transport only; it makes no authenticity claim and reaches no hardware.

use device_control::fake_transport::{open, Script, TransportKind};
use device_control::image_record::ImageRecord;
use device_control::log_record::LogRecord;
use device_control::{Op, Policy};

/// Fixed exit code for the unsupported-transport condition.
pub const EXIT_UNSUPPORTED_TRANSPORT: i32 = 2;

/// Deterministic FNV-1a 64-bit checksum of a byte payload (lowercase hex
/// when printed with `{:016x}`). Pure function of the bytes — no
/// wall-clock, no randomness.
pub fn payload_checksum(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// `logs tail`: serve every scripted frame, decode each `Logs` body, and
/// print one fixed-format line per record. All frames are served (the
/// screenshot frame is consumed too), but only log records are printed.
/// Returns the number of printed lines.
pub fn run_logs_tail<W: core::fmt::Write>(script: &Script, out: &mut W) -> Result<usize, ()> {
    let policy = Policy::default();
    let mut transport = open(TransportKind::Scripted, script).map_err(|_| ())?;
    let mut printed = 0usize;
    loop {
        let frame = match transport.serve_frame(&policy) {
            Ok(None) => break,
            Ok(Some(f)) => f,
            Err(_) => return Err(()),
        };
        if frame.op() != Op::Logs {
            continue;
        }
        let logged = LogRecord::from_frame(&frame).map_err(|_| ())?;
        let rec = logged.record();
        writeln!(
            out,
            "seq={} mono_us={} sev={:?} sub={} msg={}",
            rec.sequence(),
            rec.monotonic_us(),
            rec.severity(),
            rec.subsystem(),
            rec.message(),
        )
        .map_err(|_| ())?;
        printed += 1;
    }
    Ok(printed)
}

/// `screenshot capture`: serve every scripted frame, decode the
/// `Screenshot` body, and print one deterministic summary line. Returns
/// the number of printed lines (0 or 1).
pub fn run_screenshot_capture<W: core::fmt::Write>(
    script: &Script,
    out: &mut W,
) -> Result<usize, ()> {
    let policy = Policy::default();
    let mut transport = open(TransportKind::Scripted, script).map_err(|_| ())?;
    let mut printed = 0usize;
    loop {
        let frame = match transport.serve_frame(&policy) {
            Ok(None) => break,
            Ok(Some(f)) => f,
            Err(_) => return Err(()),
        };
        if frame.op() != Op::Screenshot {
            continue;
        }
        let img = ImageRecord::from_frame(&frame).map_err(|_| ())?;
        let rec = img.record();
        writeln!(
            out,
            "screenshot frame={} {}x{} stride={} fmt={:?} rot={:?} pixels={} checksum_fnv1a64={:016x}",
            rec.frame_seq(),
            rec.width(),
            rec.height(),
            rec.stride(),
            rec.pixel_format(),
            rec.rotation(),
            rec.pixels().len(),
            payload_checksum(rec.pixels()),
        )
        .map_err(|_| ())?;
        printed += 1;
    }
    Ok(printed)
}

/// Ask `fake_transport::open` for `kind`. On the unsupported variant this
/// prints the explicit `UNSUPPORTED-TRANSPORT` line and returns the
/// nonzero exit code — never a fallback to the scripted transport.
pub fn run_unsupported_transport<W: core::fmt::Write>(
    kind: TransportKind,
    script: &Script,
    out: &mut W,
) -> Result<i32, ()> {
    match open(kind, script) {
        Ok(_) => {
            writeln!(out, "TRANSPORT-OK").map_err(|_| ())?;
            Ok(0)
        }
        Err(e) => {
            // `e` displays as exactly "UNSUPPORTED-TRANSPORT".
            writeln!(out, "{e}").map_err(|_| ())?;
            Ok(EXIT_UNSUPPORTED_TRANSPORT)
        }
    }
}

/// Render the output of one subcommand key into a String, with the exit
/// code `main` would return. The integration tests and demo drive the
/// same `run_*` functions directly; this only maps argv keys.
fn render(sub: &str, script: &Script) -> Result<(String, i32), ()> {
    let mut out = String::new();
    match sub {
        "logs tail" => {
            run_logs_tail(script, &mut out)?;
            Ok((out, 0))
        }
        "screenshot capture" => {
            run_screenshot_capture(script, &mut out)?;
            Ok((out, 0))
        }
        "usb" => {
            run_unsupported_transport(TransportKind::Usb, script, &mut out).map(|code| (out, code))
        }
        other => {
            out.push_str("unknown subcommand or transport: ");
            out.push_str(other);
            out.push('\n');
            Ok((out, 1))
        }
    }
}

fn main() {
    let script = Script::new();
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let sub = match argv.len() {
        0 => "help".to_string(),
        // `--transport usb` names the transport kind as the subcommand key.
        1 if argv[0] == "--transport" => "help".to_string(),
        2 if argv[0] == "--transport" => argv[1].clone(),
        _ => argv.join(" "),
    };
    match render(&sub, &script) {
        Ok((text, code)) => {
            print!("{text}");
            std::process::exit(code);
        }
        Err(()) => {
            eprintln!("redox-devicectl: scripted transport decode failure");
            std::process::exit(1);
        }
    }
}
