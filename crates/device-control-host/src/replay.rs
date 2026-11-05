//! Bounded streaming read/replay of a host log file.
//!
//! This is the actual file-read path, not just a complete-buffer parser:
//! records are streamed from disk through a fixed buffer and delivered to a
//! caller callback with their envelope IDs. Rules:
//! - replay ALWAYS starts at the true start of the file (the handle's
//!   cursor is forced to 0 internally): what a caller left the cursor at
//!   never changes what is read, and every reported offset is ABSOLUTE
//!   from 0;
//! - a TOTAL byte budget bounds the pass: before reading each frame the
//!   header is probed (a fixed `HEADER_LEN` peek) so the announced frame
//!   size is known against the REMAINING budget, and reads are enforced
//!   during the body read — exhaustion stops at a frame boundary with a
//!   typed [`BudgetStop`] and the stats gathered so far. No allocation ever
//!   follows an announced length: one fixed buffer, sized to the protocol
//!   cap, exists for the whole pass;
//! - the announced body length in the frame header is checked against
//!   `MAX_LOG_BODY` BEFORE any allocation or body read (fixed buffer);
//! - every record is fully re-parsed (frame + log body) and identity-checked;
//! - a truncated tail, corrupt bytes, an oversize announcement, or a
//!   non-increasing sequence are REPORTED as issues; the original bytes on
//!   disk are never repaired, rewritten, or dropped;
//! - a callback that returns Err ABORTS the pass: the failure is propagated
//!   as a typed [`ReplayFailure`] and the failing record is NOT counted as
//!   delivered — a delivery attempt that failed is not a delivery;
//! - `last_good_offset` is the file offset just after the last fully valid
//!   AND accepted record. It is an absolute position, so a caller can seek
//!   a fresh handle there with [`seek_to`] to re-read only what follows it.
//!   It does not mean "where the next append goes" — appending is the
//!   store's job ([`crate::HostLogStore`]), never this module's.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};

use device_control::log_record::{FramedLog, LogRecord, MAX_LOG_BODY};
use device_control::{Frame, Policy, HEADER_LEN};

/// Why replay stopped at a given offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileIssueKind {
    /// Not enough bytes remain for a full frame header: truncated tail.
    TruncatedHeader,
    /// Announced body length exceeds `MAX_LOG_BODY` (or the hard cap).
    OversizeAnnouncement,
    /// Header magic/version/op/reserved/correlation/boot checks failed.
    MalformedHeader,
    /// Header OK but the full frame bytes are not all present: truncated tail.
    TruncatedFrame,
    /// Full frame present but envelope/body decode failed.
    CorruptRecord,
    /// Valid record whose sequence is not strictly increasing.
    SequenceRegression { last: u64, got: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileIssue {
    pub offset: u64,
    pub kind: FileIssueKind,
}

/// A caller-side failure during replay: the `on_record` callback returned
/// Err at a record. This is NOT a file issue — the bytes were fine, the
/// consumer failed. The pass aborts at that record; the failing record is
/// never counted as delivered and never re-delivered by this pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplayFailure {
    CallbackAborted {
        /// Absolute offset of the record whose callback failed. It was not
        /// delivered, so a retry resumes at exactly this offset.
        offset: u64,
    },
}

/// The byte budget ran out before the next whole frame could be read.
/// Typed and reported with the stats gathered so far; it is NOT a file
/// issue — nothing about the file is wrong, the pass was simply bounded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BudgetStop {
    /// The total budget this pass was given.
    pub budget_bytes: u64,
    /// Frame bytes BOOKED against the budget when it stopped — whole
    /// accepted frames only (a partial header peek is read but never
    /// booked). Always <= `budget_bytes`, and equal to `stats.good_bytes`.
    pub consumed_bytes: u64,
    /// Absolute offset of the frame that did not fit: where a later pass
    /// with more budget resumes.
    pub offset: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReplayStats {
    /// Records delivered to the callback (a record whose callback returned
    /// Err is never counted).
    pub records: u64,
    /// Bytes of valid record data consumed.
    pub good_bytes: u64,
    /// File offset just after the last fully valid AND accepted record —
    /// absolute from 0; resume a fresh handle here with [`seek_to`].
    pub last_good_offset: u64,
    /// Total file length observed.
    pub file_len: u64,
}

/// Outcome of one replay pass. Replay STOPS at the first stopper — a file
/// issue, a failing callback, or budget exhaustion — because the file is an
/// append-only concatenation and anything after a corrupt point is not
/// trustworthy framing; the original bytes are left untouched either way.
/// The three stop reasons are separate typed fields so a caller can never
/// confuse "the file was bad" with "my consumer failed" with "the budget
/// ran out".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReplayReport {
    pub stats: ReplayStats,
    /// At most one file issue: where and why replay stopped on the bytes.
    pub issue: Option<FileIssue>,
    /// Set when the `on_record` callback failed: the pass aborted with this
    /// caller failure (the failing record is not in `stats.records`).
    pub failure: Option<ReplayFailure>,
    /// Set when the byte budget stopped the pass at a frame boundary.
    pub budget: Option<BudgetStop>,
}

/// Stream every valid record from `file`, calling `on_record` with the
/// decoded record plus its envelope correlation/boot IDs. `expected_boot_id`
/// / `expected_build_id` mismatch is reported as `CorruptRecord` at that
/// offset (identity is trusted input, so a mismatch means the file was
/// cross-wired, not that bytes are corrupt — reported, never repaired).
///
/// Reading always starts at the true file start: the handle's cursor is
/// forced to 0 first, and every offset in the report is absolute.
///
/// `budget_bytes` is a TOTAL bound on frame bytes consumed by this pass.
/// The model: reading any frame costs at least its header, so when the
/// remaining budget cannot afford a full `HEADER_LEN` peek the pass stops
/// BEFORE touching a byte of that frame; otherwise the header is peeked,
/// the announced frame size is checked against the remaining budget BEFORE
/// the body read (and enforced DURING reads), and exhaustion stops at the
/// next frame boundary with `budget: Some(BudgetStop)` plus the stats
/// gathered so far. A stopped pass never books more than `budget_bytes` and
/// never allocates beyond the fixed protocol-cap buffer. `u64::MAX` means
/// "no practical bound".
pub fn replay_file<F>(
    file: &mut File,
    expected_boot_id: &[u8; 16],
    expected_build_id: &[u8; 16],
    on_record: F,
    budget_bytes: u64,
) -> io::Result<ReplayReport>
where
    F: FnMut(&FramedLog<'_>) -> Result<(), ()>,
{
    replay_pass(
        file,
        expected_boot_id,
        expected_build_id,
        on_record,
        budget_bytes,
        true,
    )
}

fn replay_pass<F>(
    file: &mut File,
    expected_boot_id: &[u8; 16],
    expected_build_id: &[u8; 16],
    mut on_record: F,
    budget_bytes: u64,
    from_start: bool,
) -> io::Result<ReplayReport>
where
    F: FnMut(&FramedLog<'_>) -> Result<(), ()>,
{
    if from_start {
        // Replay is defined over the whole file: whatever cursor the caller
        // left behind is irrelevant. Seek to the true start so reads and
        // every reported offset are absolute from 0.
        file.seek(SeekFrom::Start(0))?;
    }
    let file_len = file.metadata()?.len();
    // Fixed buffer sized to the protocol cap: no allocation follows the
    // announced length, ever. The first HEADER_LEN bytes double as a peek
    // buffer so an over-budget frame is refused before its body is read.
    let mut buf = vec![0u8; HEADER_LEN + MAX_LOG_BODY];
    let start_offset = file.stream_position()?;
    let mut offset: u64 = start_offset;
    // The remaining budget, tracked with checked arithmetic so a huge or
    // MAX-sized budget never overflows. A frame is only ever peeked/read
    // within this bound.
    let mut remaining: u64 = budget_bytes;
    let mut last_seq: u64 = 0;
    let mut records: u64 = 0;
    let mut good_bytes: u64 = 0;
    let mut last_good_offset: u64 = start_offset;

    loop {
        // Budget check BEFORE reading each frame. The header is a fixed
        // peek (never more than the buffer, never unbounded): with it we
        // know the announced frame size and can refuse an unfittable frame
        // at its start offset without consuming its body. A peek that
        // itself exceeds the remaining budget reads only what the budget
        // allows — enforcement DURING reads.
        // A whole header is always affordable when `remaining >= HEADER_LEN`
        // (any frame is at least a header, so the fit check would pass), so
        // the peek only ever shrinks below a full header when the remaining
        // budget truly cannot afford one: that is a budget stop.
        // A header peek costs `HEADER_LEN` against the budget: when the
        // remaining budget cannot even afford a header, stop BEFORE reading
        // anything of this frame — zero bytes touched.
        let want = HEADER_LEN;
        if want == 0 || remaining < HEADER_LEN as u64 {
            // A zero remaining budget reads nothing at all.
            return Ok(finish(
                offset,
                None,
                Some(BudgetStop {
                    budget_bytes,
                    consumed_bytes: good_bytes,
                    offset,
                }),
                records,
                good_bytes,
                last_good_offset,
                file_len,
            ));
        }
        let got = read_exact_or_eof(file, &mut buf[..want])?;
        // A partial header here can only be the file ending first — the
        // peek was pre-checked against the budget above — so it is a
        // truncated tail (file issue), never a budget stop. The guard below
        // stays as defence in depth for a short read under budget.
        if (got as u64) < (want as u64) && remaining < got as u64 {
            return Ok(finish(
                offset,
                None,
                Some(BudgetStop {
                    budget_bytes,
                    consumed_bytes: good_bytes,
                    offset,
                }),
                records,
                good_bytes,
                last_good_offset,
                file_len,
            ));
        }
        if got < HEADER_LEN {
            // A clean stop: EOF reached. Reaching the true end of file is
            // never budget exhaustion — BudgetStop is only for a stop that
            // the budget forced before or during a frame read.
            return Ok(finish(
                offset,
                if got == 0 {
                    None
                } else {
                    Some(FileIssue {
                        offset,
                        kind: FileIssueKind::TruncatedHeader,
                    })
                },
                None,
                records,
                good_bytes,
                last_good_offset,
                file_len,
            ));
        }
        // Structural header check FIRST (magic/version/op/reserved/ids), so
        // garbage where a record should start is reported as MalformedHeader,
        // not as an oversize announcement parsed from random bytes.
        if &buf[0..4] != b"DCTL"
            || u16::from_le_bytes([buf[4], buf[5]]) != device_control::VERSION
            || !matches!(buf[6], 1..=4)
            || buf[7] != 0
        {
            return Ok(finish(
                offset,
                Some(FileIssue {
                    offset,
                    kind: FileIssueKind::MalformedHeader,
                }),
                None,
                records,
                good_bytes,
                last_good_offset,
                file_len,
            ));
        }
        // Check the ANNOUNCED body length from the peeked header BEFORE
        // reading the body: against MAX_LOG_BODY first (a bad announcement
        // is a file issue, not a budget event), then against the remaining
        // budget.
        let declared = u32::from_le_bytes([buf[32], buf[33], buf[34], buf[35]]);
        if declared > MAX_LOG_BODY as u32 {
            return Ok(finish(
                offset,
                Some(FileIssue {
                    offset,
                    kind: FileIssueKind::OversizeAnnouncement,
                }),
                None,
                records,
                good_bytes,
                last_good_offset,
                file_len,
            ));
        }
        let total = HEADER_LEN + usize::try_from(declared).unwrap_or(MAX_LOG_BODY);
        if (total as u64) > remaining {
            // The whole announced frame does not fit the remaining budget:
            // stop at this frame's start, having booked nothing of it.
            return Ok(finish(
                offset,
                None,
                Some(BudgetStop {
                    budget_bytes,
                    consumed_bytes: good_bytes,
                    offset,
                }),
                records,
                good_bytes,
                last_good_offset,
                file_len,
            ));
        }
        // Read the rest of the frame (declared bytes bounded by MAX_LOG_BODY
        // and by the remaining budget, checked above).
        let got_body = read_exact_or_eof(file, &mut buf[HEADER_LEN..total])?;
        if got_body < total - HEADER_LEN {
            return Ok(finish(
                offset,
                Some(FileIssue {
                    offset,
                    kind: FileIssueKind::TruncatedFrame,
                }),
                None,
                records,
                good_bytes,
                last_good_offset,
                file_len,
            ));
        }
        // Full frame in hand: validate it completely.
        let parsed = match Frame::parse(&buf[..total], &Policy::default()) {
            Ok(f) => f,
            Err(_) => {
                return Ok(finish(
                    offset,
                    Some(FileIssue {
                        offset,
                        kind: FileIssueKind::MalformedHeader,
                    }),
                    None,
                    records,
                    good_bytes,
                    last_good_offset,
                    file_len,
                ))
            }
        };
        let framed = match LogRecord::from_frame(&parsed) {
            Ok(r) => r,
            Err(_) => {
                return Ok(finish(
                    offset,
                    Some(FileIssue {
                        offset,
                        kind: FileIssueKind::CorruptRecord,
                    }),
                    None,
                    records,
                    good_bytes,
                    last_good_offset,
                    file_len,
                ))
            }
        };
        if framed.boot_id() != expected_boot_id || framed.record().build_id() != expected_build_id {
            return Ok(finish(
                offset,
                Some(FileIssue {
                    offset,
                    kind: FileIssueKind::CorruptRecord,
                }),
                None,
                records,
                good_bytes,
                last_good_offset,
                file_len,
            ));
        }
        let seq = framed.record().sequence();
        if seq <= last_seq {
            return Ok(finish(
                offset,
                Some(FileIssue {
                    offset,
                    kind: FileIssueKind::SequenceRegression {
                        last: last_seq,
                        got: seq,
                    },
                }),
                None,
                records,
                good_bytes,
                last_good_offset,
                file_len,
            ));
        }
        if on_record(&framed).is_err() {
            // The CALLER failed on this record. This is not a delivery and
            // not "the caller asked to stop": it aborts the pass and is
            // propagated as a typed failure. The failing record is not
            // counted, and `last_good_offset` stays at the end of the last
            // ACCEPTED record so a retry resumes exactly at this record.
            return Ok(finish(
                offset,
                None,
                None,
                records,
                good_bytes,
                last_good_offset,
                file_len,
            )
            .with_failure(ReplayFailure::CallbackAborted { offset }));
        }
        offset += total as u64;
        seq_update(&mut last_seq, seq);
        records += 1;
        good_bytes += total as u64;
        remaining -= total as u64;
        last_good_offset = offset;
    }
}

fn seq_update(last: &mut u64, seq: u64) {
    *last = seq;
}

impl ReplayReport {
    fn with_failure(mut self, failure: ReplayFailure) -> Self {
        self.failure = Some(failure);
        self
    }
}

fn finish(
    _offset: u64,
    issue: Option<FileIssue>,
    budget: Option<BudgetStop>,
    records: u64,
    good_bytes: u64,
    last_good_offset: u64,
    file_len: u64,
) -> ReplayReport {
    ReplayReport {
        stats: ReplayStats {
            records,
            good_bytes,
            last_good_offset,
            file_len,
        },
        issue,
        failure: None,
        budget,
    }
}

/// Read until `dst` is full or EOF; returns bytes actually read.
fn read_exact_or_eof(file: &mut File, dst: &mut [u8]) -> io::Result<usize> {
    let mut done = 0;
    while done < dst.len() {
        match file.read(&mut dst[done..]) {
            Ok(0) => break,
            Ok(n) => done += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(done)
}

/// Bounded replay pass over ONE segment: starts at the CURRENT file cursor
/// (position it first with [`seek_to`]) and reads at most `span_bytes` from
/// there. Unlike [`replay_file`] — which is always the whole file, from 0 —
/// this is for callers that know a clean framing boundary and want to
/// re-read only what follows it. Every reported offset is an ABSOLUTE file
/// offset; `file_len` is still the whole file's length. The sequence check
/// restarts per pass, exactly like replay_file's. `span_bytes` bounds the
/// segment: when the remaining span cannot fit the next whole frame the
/// pass stops at that frame's start with a typed [`BudgetStop`] whose
/// `budget_bytes`/`consumed_bytes` are in SEGMENT terms.
pub fn replay_segment<F>(
    file: &mut File,
    expected_boot_id: &[u8; 16],
    expected_build_id: &[u8; 16],
    on_record: F,
    span_bytes: u64,
) -> io::Result<ReplayReport>
where
    F: FnMut(&FramedLog<'_>) -> Result<(), ()>,
{
    // Same bounded pass, but anchored at the caller's cursor instead of 0.
    replay_pass(
        file,
        expected_boot_id,
        expected_build_id,
        on_record,
        span_bytes,
        false,
    )
}

/// Resume helper: seek a fresh handle to the absolute `last_good_offset`
/// reported by a previous pass. Because replay always starts at 0, this is
/// for callers that want to re-read only what follows a known-good point.
/// This module never appends — appending is the store's job, so an offset
/// here is a READ resume point, never "where the next append goes".
pub fn seek_to(file: &mut File, offset: u64) -> io::Result<u64> {
    file.seek(SeekFrom::Start(offset))
}
