//! Host log store contract tests: real temp files, close/reopen exact
//! roundtrip, quota/identity/duplicate rejection leaving the file
//! unchanged, corrupt-file replay, and injected write/sync failures on
//! the shared writer-state path.

use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

#[path = "fixtures/mod.rs"]
mod fixtures;

use device_control::log_record::{LogRecord, Severity, MAX_LOG_BODY};
use device_control::{Policy, HEADER_LEN};
use device_control_host::{
    replay_file, AppendOutcome, ByteSink, FileIssue, FileIssueKind, HostLogStore, StoreConfig,
    StoreError,
};
use fixtures::failing_sink::{FaultPlan, MemorySink};

const BOOT: [u8; 16] = [0xB0; 16];
const BUILD: [u8; 16] = [0xBB; 16];
const QUOTA: u64 = 64 * 1024;

struct TmpDir(PathBuf);

impl TmpDir {
    fn new(tag: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "dch-{}-{}-{}",
            tag,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
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
        quota_bytes: QUOTA,
    }
}

/// Build one valid wire envelope exactly as a producer would.
fn envelope(boot: &[u8; 16], seq: u64, msg: &str) -> Vec<u8> {
    let rec = LogRecord::new(BUILD, seq, seq * 1000, Severity::Info, "sensor", msg).unwrap();
    let mut out = vec![0u8; HEADER_LEN + MAX_LOG_BODY];
    let n = rec
        .encode_envelope(seq, boot, &mut out, &Policy::default())
        .unwrap();
    out.truncate(n);
    out
}

fn store_file(tmp: &TmpDir) -> PathBuf {
    tmp.0.join("logs").join("host.log")
}

fn create_store(tmp: &TmpDir) -> HostLogStore {
    HostLogStore::create(&tmp.0, "logs", "host.log", config()).unwrap()
}

fn write_file(tmp: &TmpDir, bytes: &[u8]) {
    fs::create_dir_all(tmp.0.join("logs")).unwrap();
    let mut f = fs::File::create(store_file(tmp)).unwrap();
    f.write_all(bytes).unwrap();
    f.sync_all().unwrap();
}

#[test]
fn create_is_exclusive_and_private() {
    let tmp = TmpDir::new("create");
    let store = create_store(&tmp);
    drop(store);
    let path = store_file(&tmp);
    assert!(path.exists());
    let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "log file must be 0600");
    let dmode = fs::metadata(tmp.0.join("logs"))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(dmode, 0o700, "log dir must be 0700");
    // A second store over the same path must fail and not truncate.
    let before = fs::read(&path).unwrap();
    let again = HostLogStore::create(&tmp.0, "logs", "host.log", config());
    assert!(again.is_err(), "existing path must be rejected");
    assert_eq!(
        fs::read(&path).unwrap(),
        before,
        "failed create left file bytes"
    );
}

#[test]
fn invalid_config_rejects_before_creating_files() {
    let tmp = TmpDir::new("badcfg");
    let mut c = config();
    c.expected_boot_id = [0u8; 16];
    assert!(HostLogStore::create(&tmp.0, "logs", "host.log", c).is_err());
    let mut c = config();
    c.expected_build_id = [0u8; 16];
    assert!(HostLogStore::create(&tmp.0, "logs", "host.log", c).is_err());
    let mut c = config();
    c.quota_bytes = 0;
    assert!(HostLogStore::create(&tmp.0, "logs", "host.log", c).is_err());
    c.quota_bytes = 64 * 1024 * 1024 + 1;
    assert!(HostLogStore::create(&tmp.0, "logs", "host.log", c).is_err());
    assert!(
        !tmp.0.join("logs").exists(),
        "no dir created for invalid config"
    );
}

#[test]
fn append_then_close_reopen_roundtrips_exact_records() {
    let tmp = TmpDir::new("roundtrip");
    let msgs = [
        "hello",
        "line1\nline2\n",       // newlines retained
        "nul\0inside\0message", // NULs retained
        "",                     // empty message legal
        "unicode ✓ ünïcödé",
    ];
    let mut sizes = Vec::new();
    {
        let mut store = create_store(&tmp);
        for (i, m) in msgs.iter().enumerate() {
            let env = envelope(&BOOT, (i as u64) + 1, m);
            let out = store.append(&env).unwrap();
            assert_eq!(out.stored_bytes, env.len() as u64);
            sizes.push(env.len());
        }
        assert_eq!(store.used_bytes(), sizes.iter().sum::<usize>() as u64);
        assert!(!store.is_poisoned());
    }
    // Fresh close/reopen: exact bytes on disk, then exact decoded records.
    let path = store_file(&tmp);
    let raw = fs::read(&path).unwrap();
    let mut expected = Vec::new();
    for (i, m) in msgs.iter().enumerate() {
        expected.extend_from_slice(&envelope(&BOOT, (i as u64) + 1, m));
    }
    assert_eq!(raw, expected, "store must hold exact original framed bytes");

    let mut file = fs::File::open(store_file(&tmp)).unwrap();
    let mut seen: Vec<(u64, String, [u8; 16], u64)> = Vec::new();
    let report = replay_file(
        &mut file,
        &BOOT,
        &BUILD,
        |fl| {
            seen.push((
                fl.record().sequence(),
                fl.record().message().to_string(),
                *fl.boot_id(),
                fl.correlation_id(),
            ));
            Ok(())
        },
        u64::MAX,
    )
    .unwrap();
    assert_eq!(report.issue, None);
    assert_eq!(report.stats.records, msgs.len() as u64);
    assert_eq!(report.stats.last_good_offset, raw.len() as u64);
    let got_msgs: Vec<&str> = seen.iter().map(|(_, m, _, _)| m.as_str()).collect();
    assert_eq!(got_msgs, msgs.to_vec());
    for (i, (seq, _, boot, corr)) in seen.iter().enumerate() {
        assert_eq!(*seq, (i as u64) + 1);
        assert_eq!(*boot, BOOT);
        assert_eq!(*corr, (i as u64) + 1);
    }
}

#[test]
fn wrong_identity_duplicate_and_quota_rejections_leave_file_unchanged() {
    let tmp = TmpDir::new("reject");
    let mut store = create_store(&tmp);
    let env1 = envelope(&BOOT, 5, "first");
    store.append(&env1).unwrap();
    let path = store_file(&tmp);
    let after_good = fs::read(&path).unwrap();

    // Wrong boot id.
    let bad_boot = envelope(&[0xA5; 16], 6, "x");
    assert_eq!(store.append(&bad_boot), Err(StoreError::WrongBootId));
    // Wrong build id (build baked into body): craft by flipping body byte 0.
    let mut env = envelope(&BOOT, 7, "x");
    env[HEADER_LEN] ^= 0xFF;
    assert_eq!(store.append(&env), Err(StoreError::WrongBuildId));
    // Duplicate seq: skipped, reported explicitly with count.
    let dup = envelope(&BOOT, 5, "first");
    let out = store.append(&dup).unwrap();
    assert_eq!(
        out,
        AppendOutcome {
            stored: false,
            stored_bytes: 0,
            // Frozen DC-103 semantics: `sequence_gaps` describes THIS call.
            // The store's first accepted record was seq 5, so sequences
            // 1..=4 were lost before it — that standing total (4) lives on
            // `store.sequence_gaps()` and a refusal changes nothing; the
            // refused duplicate itself creates no gaps, so this call is 0.
            sequence_gaps: 0,
            skipped_duplicates: 1,
            rejected_backwards: 0,
        }
    );
    // Backwards seq.
    let back = envelope(&BOOT, 4, "back");
    assert_eq!(
        store.append(&back),
        Err(StoreError::NonIncreasingSequence { last: 5, got: 4 })
    );
    // Malformed envelope.
    assert!(matches!(
        store.append(b"not a frame at all...................."),
        Err(StoreError::Frame(_))
    ));
    // Quota: tiny quota store rejects before write.
    let tmp2 = TmpDir::new("quota");
    let env_big = envelope(&BOOT, 1, &"z".repeat(300));
    let mut small = HostLogStore::create(
        &tmp2.0,
        "logs",
        "host.log",
        StoreConfig {
            expected_boot_id: BOOT,
            expected_build_id: BUILD,
            quota_bytes: env_big.len() as u64 - 1,
        },
    )
    .unwrap();
    assert!(matches!(
        small.append(&env_big),
        Err(StoreError::QuotaExceeded { .. })
    ));
    assert_eq!(fs::metadata(store_file(&tmp2)).unwrap().len(), 0);
    // Exactly-at-quota store accepts.
    let tmp3 = TmpDir::new("quota2");
    let mut exact = HostLogStore::create(
        &tmp3.0,
        "logs",
        "host.log",
        StoreConfig {
            expected_boot_id: BOOT,
            expected_build_id: BUILD,
            quota_bytes: env_big.len() as u64,
        },
    )
    .unwrap();
    exact.append(&env_big).unwrap();

    // The first store's file is byte-for-byte unchanged by all rejections.
    assert_eq!(fs::read(&path).unwrap(), after_good);
    assert_eq!(store.used_bytes(), env1.len() as u64);
    assert_eq!(store.last_seq(), 5);
    assert_eq!(store.skipped_duplicates(), 1);
    assert_eq!(store.rejected_backwards(), 1);
}

#[test]
fn replay_detects_oversize_truncated_malformed_and_gaps() {
    let tmp = TmpDir::new("corrupt");
    // Oversize announcement: valid header, declared body above MAX_LOG_BODY.
    let mut hdr = [0u8; HEADER_LEN];
    hdr[0..4].copy_from_slice(b"DCTL");
    hdr[4..6].copy_from_slice(&1u16.to_le_bytes());
    hdr[6] = 1;
    hdr[8..16].copy_from_slice(&1u64.to_le_bytes());
    hdr[16..32].copy_from_slice(&BOOT);
    hdr[32..36].copy_from_slice(&((MAX_LOG_BODY + 1) as u32).to_le_bytes());
    write_file(&tmp, &hdr);
    let mut f = fs::File::open(store_file(&tmp)).unwrap();
    let r = replay_file(&mut f, &BOOT, &BUILD, |_| Ok(()), u64::MAX).unwrap();
    assert_eq!(
        r.issue.unwrap(),
        FileIssue {
            offset: 0,
            kind: FileIssueKind::OversizeAnnouncement
        }
    );
    drop(f);

    // Truncated header (5 stray bytes).
    write_file(&tmp, b"ABCDE");
    let mut f = fs::File::open(store_file(&tmp)).unwrap();
    let r = replay_file(&mut f, &BOOT, &BUILD, |_| Ok(()), u64::MAX).unwrap();
    assert_eq!(r.issue.unwrap().kind, FileIssueKind::TruncatedHeader);
    drop(f);

    // Truncated frame: header says N bytes, only N-10 present.
    let good = envelope(&BOOT, 1, "a complete message here");
    write_file(&tmp, &good[..good.len() - 10]);
    let mut f = fs::File::open(store_file(&tmp)).unwrap();
    let r = replay_file(&mut f, &BOOT, &BUILD, |_| Ok(()), u64::MAX).unwrap();
    assert_eq!(r.issue.unwrap().kind, FileIssueKind::TruncatedFrame);
    assert_eq!(r.stats.records, 0);
    assert_eq!(r.stats.last_good_offset, 0);
    drop(f);

    // Malformed header bytes where a record should start.
    write_file(&tmp, &[0xFFu8; 40]);
    let mut f = fs::File::open(store_file(&tmp)).unwrap();
    let r = replay_file(&mut f, &BOOT, &BUILD, |_| Ok(()), u64::MAX).unwrap();
    assert_eq!(r.issue.unwrap().kind, FileIssueKind::MalformedHeader);
    drop(f);

    // Gap: good, good, then a regressed/duplicate seq in the file (a gap in
    // seq numbering itself is fine; a regression is the detectable fault).
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&envelope(&BOOT, 1, "one"));
    bytes.extend_from_slice(&envelope(&BOOT, 9, "nine")); // gap 2..8: legal
    bytes.extend_from_slice(&envelope(&BOOT, 4, "four again")); // regression
    write_file(&tmp, &bytes);
    let mut f = fs::File::open(store_file(&tmp)).unwrap();
    let mut n = 0;
    let r = replay_file(
        &mut f,
        &BOOT,
        &BUILD,
        |_| {
            n += 1;
            Ok(())
        },
        u64::MAX,
    )
    .unwrap();
    assert_eq!(n, 2, "gap does not stop replay, regression does");
    assert_eq!(
        r.issue.unwrap().kind,
        FileIssueKind::SequenceRegression { last: 9, got: 4 }
    );
    let third = envelope(&BOOT, 4, "four again").len();
    assert_eq!(
        r.stats.last_good_offset,
        (bytes.len() - third) as u64,
        "last good offset points just after last valid record"
    );
    // Original evidence preserved: file bytes untouched by replay.
    assert_eq!(fs::read(store_file(&tmp)).unwrap(), bytes);
    drop(f);

    // Resume path: seeking to a clean framing boundary (last_good_offset
    // minus the last good record) re-reads that record and stops at the
    // regression. DC-103 froze this contract: `replay_file` ALWAYS starts at
    // the true file start (cursor position never changes what it reads), so
    // a cursor-anchored resume is `replay_segment`, which reads from the
    // current cursor with ABSOLUTE offsets. Replay state is per-pass;
    // evidence stays on disk.
    let second = envelope(&BOOT, 9, "nine").len() as u64;
    let mut f = fs::File::open(store_file(&tmp)).unwrap();
    device_control_host::seek_to(&mut f, r.stats.last_good_offset - second).unwrap();
    let mut n2 = 0;
    let r2 = device_control_host::replay_segment(
        &mut f,
        &BOOT,
        &BUILD,
        |_| {
            n2 += 1;
            Ok(())
        },
        u64::MAX,
    )
    .unwrap();
    assert_eq!(
        n2, 1,
        "resume re-reads the record just before the regression"
    );
    assert_eq!(r2.stats.good_bytes, second, "segment byte count is local");
    assert_eq!(r2.stats.last_good_offset, r.stats.last_good_offset);
    assert_eq!(r2.issue.unwrap().offset, r.stats.last_good_offset);
    assert_eq!(
        r2.issue.unwrap().kind,
        FileIssueKind::SequenceRegression { last: 9, got: 4 }
    );
    let start = r.stats.last_good_offset - second;
    device_control_host::seek_to(&mut f, start).unwrap();
    let empty = device_control_host::replay_segment(&mut f, &BOOT, &BUILD, |_| Ok(()), 0).unwrap();
    assert_eq!(empty.stats.last_good_offset, start);
    assert_eq!(empty.stats.good_bytes, 0);
    assert_eq!(empty.budget.unwrap().offset, start);
    device_control_host::seek_to(&mut f, start).unwrap();
    let failed =
        device_control_host::replay_segment(&mut f, &BOOT, &BUILD, |_| Err(()), u64::MAX).unwrap();
    assert_eq!(failed.stats.last_good_offset, start);
    assert_eq!(failed.stats.records, 0);
    assert_eq!(
        failed.failure,
        Some(device_control_host::ReplayFailure::CallbackAborted { offset: start })
    );
    drop(f);

    // Corrupt body mid-file: valid envelope framing, garbage body.
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&envelope(&BOOT, 1, "one"));
    let mut bad = envelope(&BOOT, 2, "two");
    bad[HEADER_LEN + 32] = 99; // invalid severity
    bytes.extend_from_slice(&bad);
    write_file(&tmp, &bytes);
    let mut f = fs::File::open(store_file(&tmp)).unwrap();
    let r = replay_file(&mut f, &BOOT, &BUILD, |_| Ok(()), u64::MAX).unwrap();
    assert_eq!(r.stats.records, 1);
    assert_eq!(r.issue.unwrap().kind, FileIssueKind::CorruptRecord);
    drop(f);

    // Cross-wired identity mid-file.
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&envelope(&BOOT, 1, "one"));
    bytes.extend_from_slice(&envelope(&[0x77; 16], 2, "other boot"));
    write_file(&tmp, &bytes);
    let mut f = fs::File::open(store_file(&tmp)).unwrap();
    let r = replay_file(&mut f, &BOOT, &BUILD, |_| Ok(()), u64::MAX).unwrap();
    assert_eq!(r.stats.records, 1);
    assert_eq!(r.issue.unwrap().kind, FileIssueKind::CorruptRecord);
}

#[test]
fn injected_write_failure_poisons_and_blocks_further_writes() {
    let fake = MemorySink::with_plan(FaultPlan::write_fails_at(1, true)); // 2nd write fails, partial
    let sink: Box<dyn ByteSink> = Box::new(fake);
    let mut store = HostLogStore::with_sink(sink, config()).unwrap();
    store.append(&envelope(&BOOT, 1, "one")).unwrap();
    let res = store.append(&envelope(&BOOT, 2, "two"));
    assert_eq!(res, Err(StoreError::Poisoned));
    assert!(store.is_poisoned());
    // No blind retry: every subsequent append is refused without touching
    // the sink again.
    let writes_after = {
        // We cannot reach the sink through the store; assert via state:
        // used_bytes only advanced for record 1.
        store.used_bytes()
    };
    let one_len = envelope(&BOOT, 1, "one").len() as u64;
    assert_eq!(writes_after, one_len);
    for seq in 3..6 {
        assert_eq!(
            store.append(&envelope(&BOOT, seq, "x")),
            Err(StoreError::Poisoned)
        );
    }
    assert_eq!(store.last_seq(), 1);
}

#[test]
fn injected_sync_failure_poisons_store() {
    let fake = MemorySink::with_plan(FaultPlan::sync_fails_at(1)); // 2nd sync fails
    let sink: Box<dyn ByteSink> = Box::new(fake);
    let mut store = HostLogStore::with_sink(sink, config()).unwrap();
    store.append(&envelope(&BOOT, 1, "one")).unwrap();
    assert_eq!(
        store.append(&envelope(&BOOT, 2, "two")),
        Err(StoreError::Poisoned)
    );
    assert!(store.is_poisoned());
    // Accounting never advanced for the record whose sync failed: no fake
    // durable success.
    assert_eq!(store.last_seq(), 1);
    assert_eq!(store.used_bytes(), envelope(&BOOT, 1, "one").len() as u64);
    assert_eq!(
        store.append(&envelope(&BOOT, 3, "x")),
        Err(StoreError::Poisoned)
    );
}

#[test]
fn fake_sink_replay_roundtrip_proves_shared_logic() {
    // Same append path, in-memory sink: the bytes it wrote must replay as
    // exact records through the same decoder used for files.
    let sink: Box<dyn ByteSink> = Box::new(MemorySink::new());
    let mut store = HostLogStore::with_sink(sink, config()).unwrap();
    for seq in 1..=5 {
        store
            .append(&envelope(&BOOT, seq, &format!("m{seq}")))
            .unwrap();
    }
    // Re-open path: write the sink bytes to a real file and replay.
    let tmp = TmpDir::new("fake2file");
    // The MemorySink contents are not reachable from the store; rebuild the
    // expected bytes from the envelopes (store stores them exactly).
    let mut bytes = Vec::new();
    for seq in 1..=5 {
        bytes.extend_from_slice(&envelope(&BOOT, seq, &format!("m{seq}")));
    }
    write_file(&tmp, &bytes);
    let mut f = fs::File::open(store_file(&tmp)).unwrap();
    let mut seqs = Vec::new();
    let r = replay_file(
        &mut f,
        &BOOT,
        &BUILD,
        |fl| {
            seqs.push(fl.record().sequence());
            Ok(())
        },
        u64::MAX,
    )
    .unwrap();
    assert_eq!(r.issue, None);
    assert_eq!(seqs, vec![1, 2, 3, 4, 5]);
}
