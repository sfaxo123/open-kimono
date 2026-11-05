//! Deterministic observe demo for the `redox-devicectl` CLI slice.
//!
//! Runs BOTH observe subcommands (`logs tail`, `screenshot capture`)
//! against a caller-provided fake-transport [`Script`] through the same
//! public codec paths the binary uses, and prints their outputs. No
//! wall-clock, no randomness: `cargo run --example cli_observe_demo`
//! twice produces byte-identical stdout.

use device_control::fake_transport::{open, Script, TransportKind};
use device_control::image_record::ImageRecord;
use device_control::log_record::LogRecord;
use device_control::{Op, Policy};

/// Same FNV-1a-64 payload checksum the binary prints.
fn payload_checksum(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn main() {
    let script = Script::new();
    let policy = Policy::default();

    println!("== logs tail ==");
    let mut t = open(TransportKind::Scripted, &script).expect("scripted transport opens");
    while let Some(frame) = t.serve_frame(&policy).expect("script frames decode") {
        if frame.op() != Op::Logs {
            continue;
        }
        let logged = LogRecord::from_frame(&frame).expect("log decodes");
        let rec = logged.record();
        println!(
            "seq={} mono_us={} sev={:?} sub={} msg={}",
            rec.sequence(),
            rec.monotonic_us(),
            rec.severity(),
            rec.subsystem(),
            rec.message(),
        );
    }

    println!("== screenshot capture ==");
    let mut t = open(TransportKind::Scripted, &script).expect("scripted transport opens");
    while let Some(frame) = t.serve_frame(&policy).expect("script frames decode") {
        if frame.op() != Op::Screenshot {
            continue;
        }
        let img = ImageRecord::from_frame(&frame).expect("image decodes");
        let rec = img.record();
        println!(
            "screenshot frame={} {}x{} stride={} fmt={:?} rot={:?} pixels={} checksum_fnv1a64={:016x}",
            rec.frame_seq(),
            rec.width(),
            rec.height(),
            rec.stride(),
            rec.pixel_format(),
            rec.rotation(),
            rec.pixels().len(),
            payload_checksum(rec.pixels()),
        );
    }
}
