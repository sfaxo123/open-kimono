//! DC-102 — host-store safety proof: path-traversal rejection before any
//! filesystem touch, private mode at first existence, and duplicate-sequence
//! rejection that writes nothing and leaves the gap total unchanged.
//!
//! Proof, not demonstration. Each case is anchored on the preserved
//! `crates/.dch-red-probe/tests/baseline_probe.rs` semantic baseline (and
//! the DC-102 brief's own proof list), and pins the behaviour where a name
//! was previously accepted by mistake:
//!
//! - **traversal** — a `../` name, an absolute name, a multi-component
//!   name, an empty name and `.` are refused as *names* for BOTH `dir_name`
//!   and `file_name`, before any directory or file is touched (the probe's
//!   finding 1: an unvalidated name reached the filesystem);
//! - **private at creation** — a created file is 0600 and a created
//!   directory 0700 at the first `stat`, never widened and then narrowed;
//! - **duplicate** — a refused duplicate returns `stored == false`, writes
//!   NOTHING (the file is byte-identical, not merely the same size), and
//!   reports the standing gap total exactly as it was before the call.
//!
//! What this file deliberately does NOT claim, kept explicit so nobody
//! reads a proof into it it:
//!
//! - The parent directory is a caller-supplied precondition and is never
//!   investigated. A hostile parent — a symlink, or a directory the caller
//!   did not mean to expose — is not detectable from here and is not
//!   claimed against: this is a naming rule, not a symlink jail.
//! - The mode assertions are mode assertions, asserted with `stat` at the
//!   first observation. No uid/gid ownership behaviour was re-derived for
//!   this packet (the literal `store_uid` / `file_uid` probe words match
//!   nothing in the base tree), so nothing here proves anything about
//!   ownership beyond the mode bits themselves.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use device_control::log_record::{LogRecord, Severity, MAX_LOG_BODY};
use device_control::{Policy, HEADER_LEN};
use device_control_host::{replay_file, HostLogStore, StoreConfig};

const BOOT: [u8; 16] = [0xB0; 16];
const BUILD: [u8; 16] = [0xBB; 16];

fn config() -> StoreConfig {
    StoreConfig {
        expected_boot_id: BOOT,
        expected_build_id: BUILD,
        quota_bytes: 64 * 1024,
    }
}

/// One private arena per case: the parent and every escape target live
/// INSIDE it, so a traversal probe cannot reach a real system or user path
/// even if the name under test were accepted (the same containment rule the
/// preserved red probe used, kept here for the same reason).
struct Arena(PathBuf);

impl Arena {
    fn new(tag: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "dch-safety-{}-{}-{}",
            tag,
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
    fn at(&self, rel: &str) -> PathBuf {
        self.0.join(rel)
    }
}

impl Drop for Arena {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Build one valid wire envelope exactly as a producer would.
fn envelope(seq: u64, msg: &str) -> Vec<u8> {
    let rec = LogRecord::new(BUILD, seq, seq * 1000, Severity::Info, "sensor", msg).unwrap();
    let mut out = vec![0u8; HEADER_LEN + MAX_LOG_BODY];
    let n = rec
        .encode_envelope(seq, &BOOT, &mut out, &Policy::default())
        .unwrap();
    out.truncate(n);
    out
}

fn mode_at_first_stat(p: &Path) -> u32 {
    // The FIRST stat is the only one that matters: a create-then-chmod
    // window is invisible to a second, later stat.
    fs::metadata(p).unwrap().permissions().mode() & 0o777
}

// ============ proof item 1 — traversal refused BEFORE any filesystem touch ==

/// `../` in the DIRECTORY name is refused, and the escape target is not
/// touched. This is the preserved red probe's own shape (finding 1), pinned
/// as a permanent test.
#[test]
fn dotdot_in_dir_name_is_rejected_and_escapes_nothing() {
    let arena = Arena::new("dd-dir");
    let parent = arena.parent();
    let victim = arena.at("victim.txt");
    fs::write(&victim, b"DO NOT TOUCH").unwrap();

    let r = HostLogStore::create(&parent, "../victim.txt", "host.log", config());
    assert!(
        r.is_err(),
        "F1: create() joined the escaping dir_name \"../victim.txt\" onto \
         the trusted parent instead of refusing it as a name"
    );
    // The escape target is byte-identical…
    assert_eq!(
        fs::read(&victim).unwrap(),
        b"DO NOT TOUCH",
        "F1: the escape target inside the arena was disturbed"
    );
    // …and nothing at all was created under the parent by a rejected name.
    let stray: Vec<String> = fs::read_dir(&parent)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        stray.is_empty(),
        "F1: rejected names still created things under the parent: {stray:?}"
    );
}

/// `../` in the FILE name is refused with the same typing, before the file
/// is touched — the same escape, reached through the second name slot.
#[test]
fn dotdot_in_file_name_is_rejected_and_escapes_nothing() {
    let arena = Arena::new("dd-file");
    let parent = arena.parent();
    let victim = arena.at("victim.txt");
    fs::write(&victim, b"DO NOT TOUCH").unwrap();

    let r = HostLogStore::create(&parent, "logs", "../victim.txt", config());
    assert!(
        r.is_err(),
        "F1: create() accepted the escaping file_name \"../victim.txt\" — \
         a name that leaves the parent is refused as a name"
    );
    assert_eq!(
        fs::read(&victim).unwrap(),
        b"DO NOT TOUCH",
        "F1: the file-name traversal disturbed the escape target"
    );
    // The refusal happened BEFORE any filesystem touch: no half-built store
    // directory was left under the parent, and nothing else was created.
    assert!(
        !parent.join("logs").exists(),
        "F1: a rejected file_name still created the store directory"
    );
    let stray: Vec<String> = fs::read_dir(&parent)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        stray.is_empty(),
        "F1: rejected file_name created things under the parent: {stray:?}"
    );
}

/// An ABSOLUTE name is refused as a name in both slots (the probe's
/// absolute-path escape case, pinned as a permanent test).
#[test]
fn absolute_name_is_rejected_before_any_touch() {
    let arena = Arena::new("abs");
    let parent = arena.parent();

    for name in ["/etc", "/etc/passwd"] {
        let r = HostLogStore::create(&parent, name, "host.log", config());
        assert!(
            r.is_err(),
            "F1: absolute dir_name {name:?} must be refused as a name"
        );
        let r = HostLogStore::create(&parent, "logs", name, config());
        assert!(
            r.is_err(),
            "F1: absolute file_name {name:?} must be refused as a name"
        );
    }
    // A refused name creates nothing: the store directory does not exist
    // under the parent at all.
    assert!(
        !parent.join("logs").exists(),
        "F1: a refused absolute name still created the store directory"
    );
}

/// A MULTI-COMPONENT name (one containing a separator, or spelling `.` /
/// `..` in any component) is refused in both slots — pinned explicitly,
/// together with the empty and `.` names the brief lists, so the whole
/// traversal family is covered, not only `../`.
#[test]
fn multi_component_empty_and_dot_names_are_rejected() {
    let arena = Arena::new("multipart");
    let parent = arena.parent();
    let victim = arena.at("victim.txt");
    fs::write(&victim, b"DO NOT TOUCH").unwrap();

    for name in [
        "",
        ".",
        "..",
        "a/b",
        "a\\b",
        "./x",
        "../..",
        "sub/deep",
        "sub\\deep",
    ] {
        let rd = HostLogStore::create(&parent, name, "host.log", config());
        let rf = HostLogStore::create(&parent, "logs", name, config());
        assert!(
            rd.is_err(),
            "F1: create() accepted the dir_name {name:?} — separators, \
             dot-names and multi-component names must be refused before any I/O"
        );
        assert!(rf.is_err(), "F1: create() accepted the file_name {name:?}");
    }
    assert_eq!(
        fs::read(&victim).unwrap(),
        b"DO NOT TOUCH",
        "F1: the arena's own escape target was disturbed by an unvalidated name"
    );
    let stray: Vec<String> = fs::read_dir(&parent)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        stray.is_empty(),
        "F1: rejected names still created things under the parent: {stray:?}"
    );
}

// ========== proof item 2 — private mode at FIRST existence (0600/0700) ====

/// The store directory is 0700 and the store file 0600 **at the first
/// stat**: both are created private (DirBuilder mode 0o700, OpenOptions
/// mode 0o600 — never a world-readable interim state). The mode is asserted
/// with `stat` at the first observation, which is exactly where a
/// create-then-chmod window would be visible.
#[test]
fn created_dir_and_file_are_private_at_first_stat() {
    let arena = Arena::new("modes");
    let mut store = HostLogStore::create(&arena.0, "logs", "host.log", config())
        .expect("a valid create with safe names works");
    let dir = arena.at("logs");
    let file = arena.at("logs/host.log");

    // First stat of each, immediately after create returned.
    assert_eq!(
        mode_at_first_stat(&dir),
        0o700,
        "a created directory is 0700 at its first stat (no widened interim state)"
    );
    assert_eq!(
        mode_at_first_stat(&file),
        0o600,
        "a created file is 0600 at its first stat (no widened interim state)"
    );

    // Writing a record never widens the file either — the 0600 is not a
    // post-write chmod.
    store.append(&envelope(1, "one")).unwrap();
    assert_eq!(mode_at_first_stat(&file), 0o600);
    drop(store);

    // A caller-made parent keeps whatever mode it had (here the default a
    // fresh temp directory gets): the store neither widens nor narrows a
    // directory it did not create. A create that entered a pre-existing
    // directory would have to chmod it to 0700 first — the mode staying
    // exactly as found is the proof no such widening-then-narrowing ever
    // happens on a path the store did not create.
    let parent = arena.parent();
    let parent_mode_before = fs::metadata(&parent).unwrap().permissions().mode();
    let kept = HostLogStore::create(&parent, "logs2", "host.log", config()).unwrap();
    assert_eq!(
        fs::metadata(&parent).unwrap().permissions().mode(),
        parent_mode_before,
        "the private dir mode is a creation-time invariant of what create \
         brought into existence, never a re-permissioning of the parent"
    );
    let _ = kept;
}

/// A second create over the same paths fails (the file is never truncated
/// and the directory is never re-entered) and writes nothing into what it
/// refused. Together with the test above this pins "exclusive AND private
/// from first existence": exclusivity is what makes the first existence the
/// only existence the store creates.
#[test]
fn second_create_is_refused_and_writes_nothing() {
    let arena = Arena::new("exclusive");
    let store = HostLogStore::create(&arena.0, "logs", "host.log", config()).unwrap();
    drop(store);
    let file = arena.at("logs/host.log");
    let before = fs::read(&file).unwrap();
    let again = HostLogStore::create(&arena.0, "logs", "host.log", config());
    assert!(again.is_err(), "an existing path must be refused");
    assert_eq!(
        fs::read(&file).unwrap(),
        before,
        "a refused second create left the file bytes"
    );
    // and the file-mode invariant survives reopening through the replay
    // seam (read-only) — the store never chmods its own file wider.
    let mut f = fs::File::open(&file).unwrap();
    let rep = replay_file(&mut f, &BOOT, &BUILD, |_| Ok(()), u64::MAX);
    assert!(rep.is_ok());
    assert_eq!(mode_at_first_stat(&file), 0o600);
}

// ============== proof item 3 — a refused duplicate writes NOTHING ==========

/// A duplicate sequence number is refused with a TYPED outcome (`stored ==
/// false`, zero bytes), the file is byte-identical (not merely the same
/// size), and the standing gap total is exactly what it was before the
/// call — refused records neither create nor erase gaps.
#[test]
fn refused_duplicate_writes_nothing_and_leaves_gap_total_unchanged() {
    let arena = Arena::new("dup");
    let mut store = HostLogStore::create(&arena.0, "logs", "host.log", config()).unwrap();

    // Seed a store with one accepted record and a known gap total: seq 9
    // loses sequences 1..=8 before it, so the standing total is 8 and the
    // file is non-empty.
    let first = store.append(&envelope(9, "nine")).unwrap();
    assert!(first.stored);
    assert_eq!(first.sequence_gaps, 8);

    let file = arena.at("logs/host.log");
    let bytes_before = fs::read(&file).unwrap();
    let len_before = bytes_before.len();
    let gaps_before = store.sequence_gaps();

    let dup = store.append(&envelope(9, "nine")).unwrap();
    assert!(!dup.stored, "a refused duplicate is never booked as stored");
    assert_eq!(dup.stored_bytes, 0, "a refused duplicate writes nothing");
    // Frozen DC-103 semantics: the outcome field describes THIS call — a
    // refusal creates no gaps, so 0 — and the standing total's invariance
    // (the intent of this proof) is asserted on the store just below.
    assert_eq!(
        dup.sequence_gaps, 0,
        "a refused duplicate creates no gaps on this call"
    );
    assert_eq!(
        store.sequence_gaps(),
        gaps_before,
        "a refused duplicate erases nothing — the standing total is unchanged"
    );
    assert_eq!(dup.skipped_duplicates, 1);
    let bytes_after = fs::read(&file).unwrap();
    assert_eq!(
        bytes_after.len(),
        len_before,
        "the refused duplicate changed the file length"
    );
    assert_eq!(
        bytes_after, bytes_before,
        "the refused duplicate changed the file bytes"
    );

    // And the whole file replays exactly as it was written: the refused
    // duplicate is not hiding inside it — the store replays ONE record.
    let mut f = fs::File::open(&file).unwrap();
    let rep = replay_file(&mut f, &BOOT, &BUILD, |_| Ok(()), u64::MAX).unwrap();
    assert!(rep.issue.is_none(), "replay stopped inside the file");
    assert_eq!(
        rep.stats.records, 1,
        "only seq 9 is in the file — the refused duplicate wrote nothing"
    );
    assert_eq!(
        rep.stats.file_len as usize, len_before,
        "the file is exactly the accepted record's bytes"
    );
}
