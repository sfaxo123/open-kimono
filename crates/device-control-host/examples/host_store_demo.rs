//! DC-103 demo: the replay path's four outcomes, run end to end and
//! printed — not asserted by panic. The store writes THREE records, then a
//! deliberately corrupt tail is appended (a half-written frame), and the
//! file is replayed THREE ways: whole-file with a corrupt tail, bounded by
//! a byte budget, and with a callback that fails on the second record.
//! Every outcome is printed as its typed field — never a panic, never a
//! silent Ok.
//!
//! Run it with:
//! ```text
//! cargo run --offline --example host_store_demo
//! ```
//!
//! It writes into a fresh temporary directory under the system temp dir and
//! removes that directory at the end; nothing here touches a real store,
//! and replay never repairs or truncates the corrupt file it reads.

use std::fs;
use std::io::Write;
use std::path::PathBuf;

use device_control::log_record::{FramedLog, LogRecord, Severity, MAX_LOG_BODY};
use device_control::{Policy, HEADER_LEN};
use device_control_host::{replay_file, HostLogStore, ReplayFailure, StoreConfig};

const BOOT: [u8; 16] = [0xB0; 16];
const BUILD: [u8; 16] = [0xBB; 16];

fn envelope(seq: u64, msg: &str) -> Vec<u8> {
    let rec = LogRecord::new(BUILD, seq, seq * 1000, Severity::Info, "sensor", msg).unwrap();
    let mut out = vec![0u8; HEADER_LEN + MAX_LOG_BODY];
    let n = rec
        .encode_envelope(seq, &BOOT, &mut out, &Policy::default())
        .unwrap();
    out.truncate(n);
    out
}

struct Arena(PathBuf);

impl Arena {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "dch-replay-demo-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        Arena(root)
    }
    fn parent(&self) -> PathBuf {
        let p = self.0.join("parent");
        fs::create_dir(&p).unwrap();
        p
    }
}

impl Drop for Arena {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn main() {
    let arena = Arena::new();
    let parent = arena.parent();
    let mut store = HostLogStore::create(
        &parent,
        "logs",
        "host.log",
        StoreConfig {
            expected_boot_id: BOOT,
            expected_build_id: BUILD,
            quota_bytes: 64 * 1024,
        },
    )
    .expect("the demo store is created under a temp arena");

    // THREE records stored: sequences 1, 2 and 3.
    let mut frame_sizes = Vec::new();
    for seq in 1..=3u64 {
        let out = store.append(&envelope(seq, &format!("msg-{seq}"))).unwrap();
        assert!(out.stored);
        frame_sizes.push(out.stored_bytes);
        println!("stored seq {seq}: {} bytes", out.stored_bytes);
    }

    // Append a CORRUPT TAIL directly to the file: half of a fourth frame.
    // (The store itself never writes partial frames; this simulates what a
    // power-lost writer could leave behind. Replay must report it, never
    // repair it.)
    let file = parent.join("logs").join("host.log");
    let bytes_before = fs::read(&file).unwrap();
    let tail = envelope(4, "half-written");
    {
        let mut f = fs::OpenOptions::new().append(true).open(&file).unwrap();
        f.write_all(&tail[..tail.len() / 2]).unwrap();
        f.sync_all().unwrap();
    }
    println!(
        "corrupt tail appended: {} of {} bytes of a fourth frame",
        tail.len() / 2,
        tail.len()
    );

    // OUTCOME 1 — corrupt tail: partial + typed issue; file never repaired.
    let mut f = fs::File::open(&file).unwrap();
    let mut seqs = Vec::new();
    let rep = replay_file(
        &mut f,
        &BOOT,
        &BUILD,
        |r: &FramedLog<'_>| {
            seqs.push(r.record().sequence());
            Ok(())
        },
        u64::MAX,
    )
    .expect("replay io");
    println!(
        "OUTCOME 1 — corrupt tail: delivered [{}], {} good bytes, stop at \
         offset {} with issue {:?}; file bytes afterwards: {}",
        seqs.iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
            .join(", "),
        rep.stats.good_bytes,
        rep.issue.map(|i| i.offset).unwrap_or(0),
        rep.issue.map(|i| i.kind),
        if fs::read(&file).unwrap() == bytes_before_then_all(&bytes_before, &tail) {
            "UNCHANGED (never repaired)"
        } else {
            "CHANGED — BUG"
        },
    );

    // OUTCOME 2 — byte budget: a total budget that fits exactly the first
    // two frames stops at frame 3's start with typed BudgetExhausted and
    // the partial stats.
    let mut f = fs::File::open(&file).unwrap();
    let budget = frame_sizes[0] + frame_sizes[1];
    let rep = replay_file(&mut f, &BOOT, &BUILD, |_| Ok(()), budget).expect("replay io");
    let be = rep.budget.expect("the demo budget stops mid-file");
    println!(
        "OUTCOME 2 — byte budget {budget}: delivered {}, booked {} bytes, \
         typed BudgetExhausted at offset {} (resume point for the next pass)",
        rep.stats.records, be.consumed_bytes, be.offset,
    );

    // OUTCOME 3 — failing callback: the consumer fails on record 2; the
    // pass ABORTS with a typed ReplayFailure and record 2 is NOT counted.
    let mut f = fs::File::open(&file).unwrap();
    let mut seen = Vec::new();
    let rep = replay_file(
        &mut f,
        &BOOT,
        &BUILD,
        |r: &FramedLog<'_>| {
            seen.push(r.record().sequence());
            if r.record().sequence() == 2 {
                Err(()) // pretend the consumer's sink failed on this record
            } else {
                Ok(())
            }
        },
        u64::MAX,
    )
    .expect("replay io");
    match rep.failure {
        Some(ReplayFailure::CallbackAborted { offset }) => println!(
            "OUTCOME 3 — failing callback: aborted at absolute offset {offset}; \
             callback saw [{}], but only {} delivered (the failing record is \
             never counted)",
            seen.iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
                .join(", "),
            rep.stats.records,
        ),
        None => println!("UNPROVEN — a failing callback was swallowed; BUG"),
    }

    // OUTCOME 4 (the freeze, printed for the record) — sequence_gaps means
    // THIS call everywhere: a refusal reports 0 on the outcome while the
    // standing total stays on the store. Seed a known gap first (seq 12 ->
    // 15 loses 13 and 14) so the standing total is visibly distinct from
    // this call's zero.
    store.append(&envelope(12, "msg-12")).unwrap();
    let jump = store.append(&envelope(15, "msg-15")).unwrap();
    println!(
        "stored seq 15 after a jump: this-call sequence_gaps={} (13, 14 lost)",
        jump.sequence_gaps,
    );
    let dup = store.append(&envelope(15, "msg-15")).unwrap();
    println!(
        "OUTCOME 4 — duplicate refusal is TYPED (Ok + stored={}), this-call \
         sequence_gaps={}, standing total={} (unchanged by the refusal)",
        dup.stored,
        dup.sequence_gaps,
        store.sequence_gaps(),
    );

    drop(store);
    drop(arena);
    println!("temp arena removed: demo wrote nothing outside it");
}

/// The full expected file bytes: the three stored frames plus the half tail.
fn bytes_before_then_all(stored: &[u8], tail: &[u8]) -> Vec<u8> {
    let mut all = stored.to_vec();
    all.extend_from_slice(&tail[..tail.len() / 2]);
    all
}
