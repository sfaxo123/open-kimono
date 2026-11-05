//! Sequence-gap semantics of the host log store (DC-101).
//!
//! The gap rules are stated by the preserved semantic RED probes in
//! `crates/.dch-red-probe/tests/baseline_probe.rs` (finding 2): a store
//! that loses sequence numbers must say so, and a refused duplicate must
//! not be booked as a stored record. Four named cases, each pinned here:
//!
//! - **before-first**: the first accepted record starts above seq 1 — the
//!   numbers below it never existed in this store, and the gap is counted;
//! - **between**: an accepted jump `last -> got` loses exactly the numbers
//!   strictly between them (`3 -> 7` skips 4, 5, 6 → three);
//! - **after-last**: a gap is only what sits *behind* an accepted record —
//!   nothing after the last sequence can be counted, and a refused
//!   duplicate neither creates nor erases a gap;
//! - **empty**: a store that has accepted nothing reports zero gaps, and a
//!   plain consecutive stream adds none.

use std::fs;
use std::path::PathBuf;

use device_control::log_record::{LogRecord, Severity, MAX_LOG_BODY};
use device_control::{Policy, HEADER_LEN};
use device_control_host::{HostLogStore, StoreConfig};

const BOOT: [u8; 16] = [0xB0; 16];
const BUILD: [u8; 16] = [0xBB; 16];

struct TmpDir(PathBuf);

impl TmpDir {
    fn new(tag: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "dch-gap-{}-{}-{}",
            tag,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        TmpDir(root)
    }
}

impl Drop for TmpDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn config() -> StoreConfig {
    StoreConfig {
        expected_boot_id: BOOT,
        expected_build_id: BUILD,
        quota_bytes: 64 * 1024,
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

fn create_store(tmp: &TmpDir) -> HostLogStore {
    HostLogStore::create(&tmp.0, "logs", "host.log", config()).unwrap()
}

/// Case 1 — before-first. The first record the host ever sees is seq 3.
/// Sequences 1 and 2 never existed in this store, and the outcome for that
/// first append must report them (the RED probe: "seq 1->3 ... lose
/// sequence numbers and AppendOutcome reports nothing about it").
#[test]
fn gap_before_first_record_is_reported_on_the_first_append() {
    let tmp = TmpDir::new("before-first");
    let mut store = create_store(&tmp);

    let first = store.append(&envelope(3, "three")).unwrap();
    assert!(first.stored, "the record after a gap is still stored");
    assert_eq!(
        first.sequence_gaps, 2,
        "seq starting at 3 loses sequences 1 and 2 before the first record"
    );
    assert_eq!(store.sequence_gaps(), 2);

    // A first record at seq 1 has nothing before it to lose.
    let tmp2 = TmpDir::new("first-one");
    let mut clean = create_store(&tmp2);
    let out = clean.append(&envelope(1, "one")).unwrap();
    assert_eq!(out.sequence_gaps, 0);
    assert_eq!(clean.sequence_gaps(), 0);
}

/// Case 2 — between. An accepted jump loses exactly the numbers strictly
/// between the last accepted sequence and this one: 3 -> 7 skips 4, 5, 6.
#[test]
fn gap_between_accepted_sequences_is_counted_and_record_still_stored() {
    let tmp = TmpDir::new("between");
    let mut store = create_store(&tmp);

    store.append(&envelope(3, "three")).unwrap();
    let jump = store.append(&envelope(7, "seven")).unwrap();
    assert!(jump.stored, "a gap is reported, never a reason to discard");
    assert_eq!(
        jump.sequence_gaps, 3,
        "3 -> 7 loses exactly sequences 4, 5 and 6"
    );
    // The total accumulates: 2 lost before the first, 3 between.
    assert_eq!(store.sequence_gaps(), 5);

    // Consecutive appends after the jump add nothing.
    let next = store.append(&envelope(8, "eight")).unwrap();
    assert_eq!(next.sequence_gaps, 0);
    assert_eq!(store.sequence_gaps(), 5);
}

/// Case 3 — after-last. Nothing after the last accepted sequence can be a
/// gap: the store reports what it lost behind it, never what might come.
/// A refused duplicate is not stored and must leave the gap total exactly
/// as it was — before this call (the RED probe's fake-success half: one
/// event cannot be simultaneously a store and a skip).
#[test]
fn nothing_after_the_last_accepted_sequence_counts_as_a_gap() {
    let tmp = TmpDir::new("after-last");
    let mut store = create_store(&tmp);

    // Last accepted is 9; the store cannot know 10 will never arrive, so
    // there is no after-last gap to report.
    store.append(&envelope(9, "nine")).unwrap();
    assert_eq!(store.sequence_gaps(), 8, "only the eight before first");

    // A duplicate at the last sequence is refused: stored = false, zero
    // bytes, and the gap total is unchanged by this call. Frozen DC-103
    // semantics (brief item 4): AppendOutcome.sequence_gaps ALWAYS means
    // the gaps created/observed by THIS call — a refusal creates none, so
    // it reports 0 — and the standing total stays on store.sequence_gaps()
    // exactly as it stood before. The intent of this assertion is unchanged:
    // one event cannot be simultaneously a store and a skip, and a refusal
    // neither creates nor erases a gap.
    let dup = store.append(&envelope(9, "nine")).unwrap();
    assert!(!dup.stored, "a refused duplicate is never booked as stored");
    assert_eq!(dup.stored_bytes, 0);
    assert_eq!(
        dup.sequence_gaps, 0,
        "this call created no gaps; the standing total lives on the store"
    );
    assert_eq!(store.sequence_gaps(), 8, "a duplicate erases nothing");
    assert_eq!(dup.skipped_duplicates, 1);

    // A backwards sequence is refused outright and changes nothing.
    let back = store.append(&envelope(4, "back"));
    assert!(back.is_err());
    assert_eq!(store.sequence_gaps(), 8);
}

/// Case 4 — empty. A store that has accepted no records has no sequences at
/// all, so it reports zero gaps; and a fully consecutive stream from seq 1
/// adds none at any point.
#[test]
fn empty_store_and_consecutive_stream_report_no_gaps() {
    let tmp = TmpDir::new("empty");
    let store = create_store(&tmp);

    // Nothing appended yet: no last sequence, therefore no gap anywhere.
    assert_eq!(store.last_seq(), 0);
    assert_eq!(store.sequence_gaps(), 0);
    assert_eq!(store.used_bytes(), 0);

    let mut store = store;
    for seq in 1..=5 {
        let out = store.append(&envelope(seq, &format!("m{seq}"))).unwrap();
        assert!(out.stored);
        assert_eq!(out.sequence_gaps, 0, "{seq}-onwards is consecutive");
    }
    assert_eq!(store.sequence_gaps(), 0);
}
