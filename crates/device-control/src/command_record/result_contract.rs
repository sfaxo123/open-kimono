//! DC-302B: the FROZEN result-contract for the COMMAND result body. This
//! module freezes the result layout, the bounds, the public error names and
//! the truncation SEMANTICS so a later codec packet cannot drift from them.
//! It contains NO codec and NO transport: nothing here encodes, decodes,
//! truncates or transmits. It is a compile-time contract, nothing more.
//!
//! Why this exists: the DC-302R candidate was rejected (root, 3:29 AM PT)
//! because it (a) bypassed the frozen shared `MAX_COMMAND_BODY` bound with a
//! private `MAX_RESULT_BODY`, (b) conflated correlation mismatch with
//! length errors and invalid detail with argv errors while claiming distinct
//! names, and (c) left truncation behavior absent — oversize streams were
//! rejected instead of yielding bounded flagged prefixes. Every one of
//! those is nailed down here as data and names.

use super::common::MAX_COMMAND_BODY;

/// Per-stream cap for one output stream of a result. Derived from the
/// frozen shared command-body bound in [`super::common`] — never redefined
/// here, so the request and result halves can never drift apart.
pub const MAX_STREAM_BODY: usize = MAX_COMMAND_BODY / 2;

/// Compile-time combined rule: the two per-stream caps must fit inside the
/// whole-body bound. `MAX_STREAM_BODY * 2 <= MAX_COMMAND_BODY`. A const
/// expression that panics if the rule is ever violated.
const _: () = assert!(MAX_STREAM_BODY * 2 <= MAX_COMMAND_BODY);

/// How an action command finished. `UnknownOutcome` is a DISTINCT variant
/// and is never conflated with `Failure`: it means the command MAY have
/// executed, so it never licenses a retry and never implies success.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultStatus {
    /// The command completed and reported success.
    Success,
    /// The command completed and reported failure.
    Failure,
    /// The outcome is not known: the command may or may not have executed.
    /// Distinct from `Failure` by discriminant and by name — a consumer
    /// must handle it separately.
    UnknownOutcome,
}

/// One bounded output stream (stdout or stderr).
///
/// Truncation semantics — FROZEN:
/// - The PRODUCER truncates: when the real stream exceeds `MAX_STREAM_BODY`,
///   the producer emits the bounded PREFIX (exactly `MAX_STREAM_BODY` bytes)
///   and sets `truncated: true`. That is the correct handling of oversize
///   input — NOT an error.
/// - A constructed value is valid only when the flag matches actual cap
///   occupancy: `truncated == (bytes.len() == MAX_STREAM_BODY)`. A
///   flagged-short prefix (bytes below the cap yet `truncated: true`) is
///   INVALID — the flag must mean "the cap was actually hit", not "I chose
///   to cut". A later validator rejects that as
///   [`ResultContractError::TruncatedFlagMismatch`].
/// - The byte array is at most `MAX_STREAM_BODY` bytes; a stream longer
///   than the cap cannot be represented untruncated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamBody {
    /// The bounded prefix of the stream, at most [`MAX_STREAM_BODY`] bytes.
    pub bytes: [u8; MAX_STREAM_BODY],
    /// Number of valid bytes in `bytes` (at most `MAX_STREAM_BODY`).
    pub len: usize,
    /// Set by the producer exactly when the stream was cut at the cap.
    pub truncated: bool,
}

impl StreamBody {
    /// Whether the occupancy equals the cap — the only occupancy at which
    /// `truncated: true` is consistent.
    pub fn at_cap(&self) -> bool {
        self.len == MAX_STREAM_BODY
    }
}

/// The correlation binding of a result back to its DC-301 request. A result
/// whose ref does not match the request it answers is a typed rejection
/// ([`ResultContractError::CorrelationMismatch`]), never a length error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActionCommandRef {
    /// Identifies the control session the command was issued on.
    pub session_id: u64,
    /// The request sequence number within that session.
    pub request_seq: u64,
}

/// One action result: status plus both bounded output streams plus the
/// correlation binding. Plain data — no codec, no transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActionResult {
    pub status: ResultStatus,
    pub stdout: StreamBody,
    pub stderr: StreamBody,
    pub correlation: ActionCommandRef,
}

/// The public error names for the result contract. DISTINCT variants — the
/// DC-302R conflation (correlation error folded into a length error, detail
/// error folded into an argv error) is explicitly forbidden. A later codec
/// must surface exactly these names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultContractError {
    /// The result's [`ActionCommandRef`] (session_id or request_seq) does
    /// not match the request it is being matched to. Fires on correlation
    /// only — never mixed with any length or detail condition.
    CorrelationMismatch,
    /// A declared stream length does not fit the actual body bytes (the
    /// declared bytes are not present, or extra bytes trail). Fires only
    /// on length arithmetic, never on correlation.
    BodyLengthMismatch,
    /// The `truncated` flag does not match actual cap occupancy: a
    /// constructed flagged-short prefix (len < cap yet flag set), or an
    /// at-cap prefix with the flag clear when the producer's contract says
    /// the cut happened. Fires only on flag/occupancy disagreement.
    TruncatedFlagMismatch,
    /// A status-dependent detail value violated its rule for that status
    /// (e.g. a detail required by Failure supplied as Success, or an
    /// out-of-range detail). Fires only on the detail/status rule — never
    /// folded into an argv or length rejection.
    InvalidDetail,
    /// Producing or accepting the result would exceed the frozen combined
    /// budget: total body bytes above `MAX_COMMAND_BODY`, or a per-stream
    /// length above `MAX_STREAM_BODY`. Fires only on budget arithmetic.
    BudgetExceeded,
}

/// A row of the frozen fixture table: literal expectations a later codec
/// must reproduce. No logic — data only. `ok` means the scenario is
/// ACCEPTED (no error), including the oversize-input scenario, which is
/// handled as a bounded flagged prefix rather than a rejection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FixtureRow {
    pub name: &'static str,
    pub status: ResultStatus,
    pub stdout_len: usize,
    pub stderr_len: usize,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
    /// `None` = scenario must be accepted; `Some(e)` = must reject with
    /// exactly this error name.
    pub expected_error: Option<ResultContractError>,
}

/// Frozen fixture rows. Every row is internally consistent: `truncated`
/// matches cap occupancy (`len == MAX_STREAM_BODY`), and every error row
/// names a distinct variant.
pub const FIXTURES: [FixtureRow; 4] = [
    FixtureRow {
        // Both streams exactly at cap, both flagged: the normal oversize
        // outcome — ACCEPTED, not an error.
        name: "both_streams_at_cap_truncated",
        status: ResultStatus::Success,
        stdout_len: MAX_STREAM_BODY,
        stderr_len: MAX_STREAM_BODY,
        stdout_truncated: true,
        stderr_truncated: true,
        expected_error: None,
    },
    FixtureRow {
        // Oversize-input scenario (two streams of MAX_STREAM_BODY + 1
        // real bytes): the FROZEN handling is a bounded flagged PREFIX of
        // MAX_STREAM_BODY bytes with truncated = true — documented here
        // as the post-truncation shape, ACCEPTED, NOT an error.
        name: "oversize_input_yields_flagged_prefix",
        status: ResultStatus::Failure,
        stdout_len: MAX_STREAM_BODY,
        stderr_len: MAX_STREAM_BODY,
        stdout_truncated: true,
        stderr_truncated: true,
        expected_error: None,
    },
    FixtureRow {
        // Result answers a different request than the one it is matched
        // to: correlation-only rejection.
        name: "correlation_mismatch",
        status: ResultStatus::Success,
        stdout_len: 0,
        stderr_len: 0,
        stdout_truncated: false,
        stderr_truncated: false,
        expected_error: Some(ResultContractError::CorrelationMismatch),
    },
    FixtureRow {
        // UnknownOutcome is a first-class status, distinct from Failure:
        // a small-stream result carrying it is ACCEPTED as-is.
        name: "unknown_outcome_distinct_from_failure",
        status: ResultStatus::UnknownOutcome,
        stdout_len: 5,
        stderr_len: 0,
        stdout_truncated: false,
        stderr_truncated: false,
        expected_error: None,
    },
];
