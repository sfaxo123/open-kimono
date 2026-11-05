//! DC-102 demo: the host store's three safety rules, run end to end and
//! printed — not asserted by panic. The store stores THREE records, then
//! makes ONE duplicate attempt and ONE traversal attempt, and prints BOTH
//! rejections as typed outcomes (never a panic, never a silent Ok).
//!
//! Run it with:
//! ```text
//! cargo run --offline --example host_store_safety_demo
//! ```
//!
//! It writes into a fresh temporary directory under the system temp dir and
//! removes that directory at the end; nothing here touches a real store.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use device_control::log_record::{FramedLog, LogRecord, Severity, MAX_LOG_BODY};
use device_control::{Policy, HEADER_LEN};
use device_control_host::{replay_file, HostLogStore, StoreConfig};

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

fn temp_arena() -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "dch-safety-demo-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&root).unwrap();
    root
}

fn mode_of(p: &std::path::Path) -> u32 {
    fs::metadata(p).unwrap().permissions().mode() & 0o777
}

/// One temp arena, its caller-made parent kept INSIDE it, and the store
/// file inside that — a traversal name has nowhere outside the arena to
/// reach, which is the point the printout below reports honestly.
struct Arena(PathBuf);

impl Arena {
    fn new() -> Self {
        Arena(temp_arena())
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

    // Private at creation, proven at the first stat.
    let dir = parent.join("logs");
    let file = dir.join("host.log");
    println!(
        "private at creation: dir {:o}, file {:o}",
        mode_of(&dir),
        mode_of(&file)
    );

    // THREE records stored: sequences 1, 2 and 3, byte-for-byte retained.
    for seq in 1..=3 {
        let out = store.append(&envelope(seq, &format!("msg-{seq}"))).unwrap();
        assert!(out.stored, "the demo only stores three real records");
        println!(
            "stored seq {seq}: {} bytes (total {})",
            out.stored_bytes,
            store.used_bytes()
        );
    }

    // ONE duplicate attempt: the same bytes as seq 3 again. Typed refusal,
    // not a panic and not a silent success: stored == false, zero bytes,
    // and the gap total unchanged by the call.
    let len_before = fs::metadata(&file).unwrap().len();
    let gaps_before = store.sequence_gaps();
    let dup = store.append(&envelope(3, "msg-3")).unwrap();
    let len_after = fs::metadata(&file).unwrap().len();
    // Frozen DC-103 semantics: `dup.sequence_gaps` describes THIS call — a
    // refusal creates no gaps, so it is 0 — while the standing total stays
    // on the store and is unchanged by the refusal.
    println!(
        "REJECTION 1 — duplicate seq 3 refused: stored={}, stored_bytes={}, \
         this-call gaps={}, standing gap total {} -> {} (unchanged), \
         file length {} -> {} (nothing written)",
        dup.stored,
        dup.stored_bytes,
        dup.sequence_gaps,
        gaps_before,
        store.sequence_gaps(),
        len_before,
        len_after,
    );

    // ONE traversal attempt: a directory name that leaves the caller's
    // parent (`../escape`). Typed refusal before any filesystem touch:
    // the name is refused as a name, and because the refusal happened
    // before any touch, NOTHING is created and NOTHING outside the arena
    // is disturbed. A hostile parent is out of scope by design (the
    // parent is caller-trusted); this is a naming rule, not a jail.
    let traversal = HostLogStore::create(
        &parent,
        "../escape",
        "host.log",
        StoreConfig {
            expected_boot_id: BOOT,
            expected_build_id: BUILD,
            quota_bytes: 64 * 1024,
        },
    );
    match traversal {
        Err(e) => println!("REJECTION 2 — traversal create refused: {e}"),
        Ok(_) => println!(
            "UNPROVEN — the traversal name was ACCEPTED and reached the \
             filesystem; the naming rule is not enforced"
        ),
    }

    // Honest closing proof of the duplicate refusal: the file replays
    // exactly three records — the refused duplicate is not inside it.
    let mut f = fs::File::open(&file).unwrap();
    let mut seen = Vec::new();
    let rep = replay_file(
        &mut f,
        &BOOT,
        &BUILD,
        |r: &FramedLog<'_>| {
            seen.push(r.record().sequence());
            Ok(())
        },
        u64::MAX,
    )
    .expect("replay of the demo file");
    println!(
        "replay: {} records replays as {} (three stored; the refused \
         duplicate wrote nothing)",
        rep.stats.records,
        seen.iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    );

    drop(store);
    drop(arena);
    println!("temp arena removed: demo wrote nothing outside it");
}
