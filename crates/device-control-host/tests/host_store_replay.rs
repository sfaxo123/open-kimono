//! DC-103 — replay & failure-semantics RED tests, written BEFORE the fix.
//!
//! Each named test reproduces a defect the root review confirmed in
//! `src/replay.rs` / `src/store.rs` at 01:37 PT. They compile against the
//! pre-fix API and FAIL against it; the fix in this packet makes them pass.
//! The tests are never weakened afterwards, and replay never repairs or
//! truncates corrupt input — the byte-identity assertions below pin that.
//!
//! Defect map:
//! 1. `failing_callback_aborts_and_propagates` — an `on_record` Err was
//!    swallowed: the failing record was counted as delivered and the replay
//!    returned Ok with `issue: None`. Correct: the Err ABORTS the pass, is
//!    propagated as a failure, and the failing record is NOT counted.
//! 2. `replay_after_interleaved_cursor_moves_starts_at_zero_with_absolute_offsets`
//!    — `replay_file` started at the current file cursor while stats claimed
//!    offsets from 0. Correct: replay always starts at the true file start;
//!    every reported offset is absolute.
//! 3. `budget_exhaustion_returns_typed_budget_exhausted_mid_file` — there
//!    was no byte budget at all. Correct: a TOTAL byte budget checked
//!    BEFORE reading each frame (against the remaining budget) and enforced
//!    DURING reads, with typed `BudgetExhausted` and the stats gathered so
//!    far.
//! 4. `duplicate_outcome_reports_this_calls_gaps_not_the_standing_total` —
//!    `AppendOutcome.sequence_gaps` meant per-record gaps on the stored
//!    path but the standing cumulative total on the duplicate path.
//!    Correct: the field ALWAYS means gaps created/observed by THIS call
//!    (0 on refusals); the standing total stays on `store.sequence_gaps()`.
//! 5. `truncated_tail_is_partial_plus_issue_and_never_repaired` — the
//!    documented partial+issue contract for a truncated tail, and the file
//!    bytes byte-identical afterwards (never repaired). This one passes
//!    before and after the fix by design: it freezes the contract the fix
//!    must not break.

use std::fs;
use std::io::{Seek, SeekFrom, Write};
use std::path::PathBuf;

use device_control::log_record::{FramedLog, LogRecord, Severity, MAX_LOG_BODY};
use device_control::{Policy, HEADER_LEN};
use device_control_host::{
    replay_file, AppendOutcome, FileIssue, FileIssueKind, HostLogStore, ReplayFailure,
    ReplayReport, ReplayStats, StoreConfig,
};

const BOOT: [u8; 16] = [0xB0; 16];
const BUILD: [u8; 16] = [0xBB; 16];

struct TmpDir(PathBuf);

impl TmpDir {
    fn new(tag: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "dch-replay-{}-{}-{}",
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
    fn file(&self) -> PathBuf {
        self.0.join("host.log")
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

fn write_file(tmp: &TmpDir, bytes: &[u8]) {
    let mut f = fs::File::create(tmp.file()).unwrap();
    f.write_all(bytes).unwrap();
    f.sync_all().unwrap();
}

/// A three-record file: sequences 1..=3, exact framed bytes concatenated.
fn three_records() -> (Vec<u8>, Vec<usize>) {
    let msgs = ["alpha-one", "bravo-two", "charlie-three"];
    let mut bytes = Vec::new();
    let mut sizes = Vec::new();
    for (i, m) in msgs.iter().enumerate() {
        let env = envelope((i as u64) + 1, m);
        sizes.push(env.len());
        bytes.extend_from_slice(&env);
    }
    (bytes, sizes)
}

// ============ defect 1 — a failing callback must ABORT and PROPAGATE =======

/// An `on_record` Err is a caller failure, not a "caller stopped early"
/// signal: it must abort the pass, be propagated as a failure, and the
/// failing record must NOT be counted as delivered. Pre-fix, replay
/// counted the failing record and returned Ok with `issue: None`.
#[test]
fn failing_callback_aborts_and_propagates() {
    let tmp = TmpDir::new("cb-fail");
    let (bytes, sizes) = three_records();
    write_file(&tmp, &bytes);

    let mut seen: Vec<u64> = Vec::new();
    let rep = replay_file(
        &mut fs::File::open(tmp.file()).unwrap(),
        &BOOT,
        &BUILD,
        |r| {
            seen.push(r.record().sequence());
            if r.record().sequence() == 2 {
                Err(()) // the caller's storage fails on the second record
            } else {
                Ok(())
            }
        },
        u64::MAX,
    )
    .expect("io itself is fine; the failure is the callback's");

    // The pass reports FAILURE, typed, not Ok-with-no-issue — at the
    // absolute offset of the record whose callback failed (record 2).
    let good = sizes[0] as u64;
    assert_eq!(
        rep.failure,
        Some(ReplayFailure::CallbackAborted { offset: good }),
        "a failing callback must be propagated as a failure, not swallowed"
    );
    // Only record 1 was actually delivered; record 2 failed and is NOT
    // counted as delivered.
    assert_eq!(seen, vec![1, 2], "the callback saw 1 and 2 (2 failed)");
    assert_eq!(
        rep.stats.records, 1,
        "the failing record is never counted as delivered"
    );
    // The good prefix is still reported honestly: stats cover record 1 only.
    assert_eq!(rep.stats.good_bytes, good);
    assert_eq!(rep.stats.last_good_offset, good);
    assert_eq!(rep.stats.file_len, bytes.len() as u64);
    // A file issue is a different thing from a caller failure: none here.
    assert_eq!(rep.issue, None);
    // And the file itself is untouched — a callback failure rewrites nothing.
    assert_eq!(fs::read(tmp.file()).unwrap(), bytes);
}

// ============ defect 2 — replay starts at 0; offsets are absolute =========

/// No matter where the caller left the file cursor, `replay_file` must read
/// from the TRUE file start and report absolute offsets. Pre-fix it started
/// at the cursor while stats claimed offsets from 0.
#[test]
fn replay_after_interleaved_cursor_moves_starts_at_zero_with_absolute_offsets() {
    let tmp = TmpDir::new("cursor");
    let (bytes, sizes) = three_records();
    write_file(&tmp, &bytes);

    let mut f = fs::File::open(tmp.file()).unwrap();
    // Interleaved cursor moves: end, middle, start-1 — none of them may
    // change what replay reads or how it numbers offsets.
    f.seek(SeekFrom::End(0)).unwrap();
    f.seek(SeekFrom::Start(sizes[0] as u64)).unwrap();
    f.seek(SeekFrom::Start(1)).unwrap();

    let mut seqs: Vec<u64> = Vec::new();
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

    // ALL THREE records, in order, even though the cursor was mid-file.
    assert_eq!(
        seqs,
        vec![1, 2, 3],
        "replay must always start at the true file start, not the cursor"
    );
    // Every reported offset is absolute from 0.
    assert_eq!(rep.stats.records, 3);
    assert_eq!(rep.stats.last_good_offset, bytes.len() as u64);
    assert_eq!(rep.stats.good_bytes, bytes.len() as u64);
    assert_eq!(rep.stats.file_len, bytes.len() as u64);
    assert_eq!(rep.issue, None);

    // Absolute-offset claims hold on a fresh handle too.
    let mut g = fs::File::open(tmp.file()).unwrap();
    let rep2 = replay_file(&mut g, &BOOT, &BUILD, |_| Ok(()), u64::MAX).expect("replay io");
    assert_eq!(
        rep2.stats, rep.stats,
        "cursor position must not change stats"
    );
}

// ============ defect 3 — TOTAL byte budget, typed BudgetExhausted ==========

/// A budget that cannot fit the first whole frame must be caught BEFORE any
/// read of that frame: zero records, zero bytes consumed, and the file
/// cursor must not have advanced.
#[test]
fn budget_below_first_frame_is_refused_before_any_read() {
    let tmp = TmpDir::new("budget-zero");
    let (bytes, sizes) = three_records();
    write_file(&tmp, &bytes);

    let mut f = fs::File::open(tmp.file()).unwrap();
    let budget = (sizes[0] - 1) as u64;
    let rep = replay_file(&mut f, &BOOT, &BUILD, |_| Ok(()), budget).expect("replay io");

    let be = rep
        .budget
        .expect("a budget smaller than the first frame must stop replay");
    assert_eq!(be.budget_bytes, budget);
    assert_eq!(
        be.consumed_bytes, 0,
        "nothing may be booked past the budget"
    );
    assert_eq!(
        be.offset, 0,
        "the stop is at the start of the unfittable frame"
    );
    assert_eq!(rep.stats.records, 0);
    assert_eq!(rep.stats.good_bytes, 0);
    assert_eq!(rep.stats.last_good_offset, 0);
    // Checked BEFORE reading the frame: nothing was BOOKED against the
    // budget. At most a fixed header-sized peek was taken — never body
    // bytes, and no allocation or accounting follows.
    let pos = f.stream_position().unwrap();
    assert!(pos <= HEADER_LEN as u64, "no body bytes may be read at all");
    // Not misreported as a file issue — the file itself is intact.
    assert_eq!(rep.issue, None);
    assert_eq!(fs::read(tmp.file()).unwrap(), bytes);
}

/// A budget that fits whole frames up to a mid-file boundary must deliver
/// exactly the whole frames it can pay for, stop at the next frame start
/// (never mid-record), and report typed `BudgetExhausted` with the partial
/// stats.
#[test]
fn budget_exhaustion_returns_typed_budget_exhausted_mid_file() {
    let tmp = TmpDir::new("budget-mid");
    let (bytes, sizes) = three_records();
    write_file(&tmp, &bytes);

    let mut f = fs::File::open(tmp.file()).unwrap();
    let budget = (sizes[0] + sizes[1]) as u64; // exactly records 1 and 2
    let rep = replay_file(&mut f, &BOOT, &BUILD, |_| Ok(()), budget).expect("replay io");

    let be = rep
        .budget
        .expect("the third frame does not fit the remaining budget");
    assert_eq!(be.budget_bytes, budget);
    // Booked accounting: exactly the two whole frames; only a fixed header
    // peek of frame 3 was physically read, never booked.
    assert_eq!(be.consumed_bytes, budget, "exactly the budget was booked");
    assert_eq!(
        be.consumed_bytes, rep.stats.good_bytes,
        "consumed == booked"
    );
    assert_eq!(be.offset, budget, "the stop is at the next frame's start");
    assert_eq!(
        rep.stats.records, 2,
        "only whole frames the budget paid for"
    );
    assert_eq!(rep.stats.good_bytes, budget);
    assert_eq!(rep.stats.last_good_offset, budget);
    assert_eq!(rep.stats.file_len, bytes.len() as u64);
    assert_eq!(rep.issue, None);
    assert_eq!(fs::read(tmp.file()).unwrap(), bytes);
}

/// The budget is enforced DURING reads: a header that fits the remaining
/// budget but announces a body that overruns it must be refused before the
/// body is read, with the budget stop at that frame's start.
#[test]
fn budget_is_enforced_during_reads_not_only_before_frames() {
    let tmp = TmpDir::new("budget-midread");
    let (bytes, sizes) = three_records();
    write_file(&tmp, &bytes);

    let mut f = fs::File::open(tmp.file()).unwrap();
    // Enough for record 1 plus part of record 2's header: the header fits
    // the remaining budget, its announced body does not.
    let budget = (sizes[0] + HEADER_LEN - 1) as u64;
    let rep = replay_file(&mut f, &BOOT, &BUILD, |_| Ok(()), budget).expect("replay io");

    let be = rep
        .budget
        .expect("record 2's announced frame overruns the remaining budget");
    assert_eq!(be.budget_bytes, budget);
    assert_eq!(be.offset, sizes[0] as u64, "stop at record 2's start");
    // Booked: only record 1. Physically read: record 1 plus at most one
    // fixed header peek — never the body that would have overrun.
    assert_eq!(be.consumed_bytes, sizes[0] as u64);
    assert_eq!(rep.stats.records, 1);
    assert_eq!(rep.stats.good_bytes, sizes[0] as u64);
    assert_eq!(rep.stats.last_good_offset, sizes[0] as u64);
    // The header probe that overran was never booked as good data, and the
    // body was never read: the cursor sits at most at the header probe.
    let pos = f.stream_position().unwrap();
    assert!(
        pos <= sizes[0] as u64 + HEADER_LEN as u64,
        "no body bytes may be read once the announced frame overruns the budget"
    );
    assert_eq!(rep.issue, None);
    assert_eq!(fs::read(tmp.file()).unwrap(), bytes);
}

/// A budget of zero is a legitimate total budget: nothing is read at all.
#[test]
fn zero_budget_reads_nothing() {
    let tmp = TmpDir::new("budget-none");
    let (bytes, _) = three_records();
    write_file(&tmp, &bytes);

    let mut f = fs::File::open(tmp.file()).unwrap();
    let rep = replay_file(&mut f, &BOOT, &BUILD, |_| Ok(()), 0).expect("replay io");
    let be = rep.budget.expect("zero budget stops immediately");
    assert_eq!(be.budget_bytes, 0);
    assert_eq!(be.consumed_bytes, 0);
    assert_eq!(be.offset, 0);
    assert_eq!(rep.stats.records, 0);
    // A zero total budget reads nothing at all.
    assert_eq!(f.stream_position().unwrap(), 0);
    assert_eq!(rep.issue, None);
}

/// The `u64::MAX` sentinel means "no practical bound": the pass behaves
/// exactly like an unbounded read — full stats, no budget stop. A finite
/// budget that fits every frame delivers everything and stops cleanly at
/// EOF with only the exact whole frames booked against it.
#[test]
fn budget_at_least_file_len_replays_everything() {
    let tmp = TmpDir::new("budget-full");
    let (bytes, _) = three_records();
    write_file(&tmp, &bytes);

    let mut f = fs::File::open(tmp.file()).unwrap();
    let rep = replay_file(&mut f, &BOOT, &BUILD, |_| Ok(()), u64::MAX).expect("replay io");
    assert_eq!(
        rep.budget, None,
        "the unbounded sentinel never stops a pass"
    );
    assert_eq!(rep.stats.records, 3);
    assert_eq!(rep.stats.last_good_offset, bytes.len() as u64);
    assert_eq!(rep.issue, None);

    // A finite budget >= file length: everything delivered; the only stop
    // is EOF at the true end of file, with exactly the whole frames booked.
    let mut g = fs::File::open(tmp.file()).unwrap();
    let rep2 =
        replay_file(&mut g, &BOOT, &BUILD, |_| Ok(()), bytes.len() as u64).expect("replay io");
    assert_eq!(rep2.stats.records, 3);
    assert_eq!(rep2.stats.good_bytes, bytes.len() as u64);
    if let Some(be) = rep2.budget {
        assert_eq!(be.offset, bytes.len() as u64, "only at true EOF");
        assert_eq!(
            be.consumed_bytes,
            bytes.len() as u64,
            "exactly the whole frames booked"
        );
    }
    assert_eq!(rep2.issue, None);
}

// ============ defect 4 — sequence_gaps: THIS call, always =================

/// `AppendOutcome.sequence_gaps` must mean the gaps created/observed by
/// THIS call on every path. Pre-fix the duplicate path reported the
/// standing cumulative total instead. The standing total stays available
/// via `store.sequence_gaps()`.
#[test]
fn duplicate_outcome_reports_this_calls_gaps_not_the_standing_total() {
    let tmp = TmpDir::new("gaps-freeze");
    let mut store = HostLogStore::create(&tmp.0, "logs", "host.log", config()).unwrap();

    // Stored path: seq 9 first — THIS call created 8 gaps.
    let first = store.append(&envelope(9, "nine")).unwrap();
    assert!(first.stored);
    assert_eq!(first.sequence_gaps, 8, "this call created 8 gaps");
    assert_eq!(store.sequence_gaps(), 8, "standing total after this call");

    // Duplicate path: THIS call created no gaps, so the field is 0 — NOT
    // the standing total of 8. Pre-fix this reported 8 (cumulative), which
    // is the inconsistency this packet freezes away.
    let dup = store.append(&envelope(9, "nine")).unwrap();
    assert_eq!(
        dup,
        AppendOutcome {
            stored: false,
            stored_bytes: 0,
            sequence_gaps: 0,
            skipped_duplicates: 1,
            rejected_backwards: 0,
        },
        "sequence_gaps must mean THIS call on the duplicate path too"
    );
    // The standing total is unchanged and stays readable from the store.
    assert_eq!(store.sequence_gaps(), 8, "a refusal erases nothing");

    // A second stored jump reports only ITS gaps.
    let jump = store.append(&envelope(12, "twelve")).unwrap();
    assert!(jump.stored);
    assert_eq!(jump.sequence_gaps, 2, "12 - 9 loses 10 and 11");
    assert_eq!(store.sequence_gaps(), 10, "standing total accumulates");

    // A duplicate of the newest record again: 0 on this call.
    let dup2 = store.append(&envelope(12, "twelve")).unwrap();
    assert_eq!(dup2.sequence_gaps, 0);
    assert_eq!(store.sequence_gaps(), 10);
}

// ==== defect 5 (contract freeze) — truncated tail: partial + issue, =======
// ==== file bytes never repaired. Passes before AND after the fix. =========

#[test]
fn truncated_tail_is_partial_plus_issue_and_never_repaired() {
    let tmp = TmpDir::new("trunc");
    let (bytes, sizes) = three_records();
    // Records 1 and 2 whole, then the first half of record 3's bytes.
    let full = sizes[0] + sizes[1];
    let truncated = &bytes[..full + sizes[2] / 2];
    write_file(&tmp, truncated);
    let on_disk = fs::read(tmp.file()).unwrap();

    let mut f = fs::File::open(tmp.file()).unwrap();
    let rep: ReplayReport =
        replay_file(&mut f, &BOOT, &BUILD, |_| Ok(()), u64::MAX).expect("replay io");

    // Documented partial+issue result: the whole prefix is delivered, the
    // incomplete tail frame is reported, never repaired.
    assert_eq!(
        rep.issue.unwrap(),
        FileIssue {
            offset: full as u64,
            kind: FileIssueKind::TruncatedFrame,
        },
        "a half-written tail frame is a TruncatedFrame at its start offset"
    );
    let stats: ReplayStats = rep.stats;
    assert_eq!(stats.records, 2, "the whole prefix is still delivered");
    assert_eq!(stats.good_bytes, full as u64);
    assert_eq!(stats.last_good_offset, full as u64);
    assert_eq!(stats.file_len, truncated.len() as u64);
    assert_eq!(rep.budget, None, "the budget was not the limiter here");

    // The file bytes are byte-identical afterwards: replay never repairs,
    // truncates, or rewrites the corrupt tail.
    assert_eq!(fs::read(tmp.file()).unwrap(), on_disk);
}
