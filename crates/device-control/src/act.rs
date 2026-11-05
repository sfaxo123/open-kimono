//! DC-CLI-02: bounded ACT executor over the scripted fake transport.
//!
//! [`ActExecutor`] takes one validated [`CommandRequest`], drives it through
//! [`SessionPolicy`] (start_action -> record_ack -> record_result) and a
//! [`ScenarioTransport`], and returns a frozen-contract [`ActionResult`].
//! It retries nothing and re-sends nothing: a request crosses the transport
//! exactly once per `run`.
//!
//! ## Scripted-frame convention (defined HERE; fake_transport is untouched)
//!
//! The frozen [`FakeTransport`]/`Script` pair serves only its fixed
//! log/screenshot frames and cannot be re-scripted, so the act slice ships
//! its own tiny scenario transport ([`ScenarioTransport`], same borrow-only
//! serve contract) fed by [`script_ack`] / [`script_result`] frames: `DCTL`
//! envelopes with `op == Op::Command` whose payload is a tiny act body:
//! - ACK body (9 bytes): byte 0 = 1, bytes 1..9 = u64 LE `request_seq`
//!   (duplicated in the envelope correlation id);
//! - RESULT body: byte 0 = status (0 Success, 1 Failure, 2 UnknownOutcome),
//!   then u16 LE stdout len + bytes, then u16 LE stderr len + bytes, no
//!   trailing bytes. The answered `request_seq` is the correlation id.
//!
//! A frame the executor cannot classify (non-Command op, unknown body tag,
//! trailing bytes) is SKIPPED; an unparseable envelope ends the await.
//!
//! ## Closure rule (root advisory 2026-10-28)
//!
//! A policy slot closes as a KNOWN SUCCESS ONLY on evidence: a scripted
//! result with status Success, or a pre-send cancel (a proven-unsent
//! abandonment — the POLICY's in-flight record says `sent == false`,
//! DC-CLI-02D). A scripted result with status FAILURE closes the slot as a
//! KNOWN FAILED outcome through the policy's dedicated
//! [`SessionPolicy::record_command_failure`] edge (DC-ACT-FAILURE): the
//! frozen result contract says Failure is completed and reported, so the
//! action is no longer unresolved — but the ActionResult still reports
//! Failure, and no success is reported or implied by the closure. A
//! STATUS_UNKNOWN result never
//! closes the slot — the policy stays in-flight exactly like the timeout
//! path and the ActionResult keeps `UnknownOutcome`. And `cancel` after
//! any send is refused with [`ActError::SentNoAck`] with zero policy
//! mutation: a sent-but-un-ACKed action may have executed on the device,
//! so forging an ACK + success closure is not evidence.
//!
//! Send evidence is SESSION-OWNED, not executor-local: `run` stamps the
//! policy's in-flight record via `SessionPolicy::mark_sent` at send time,
//! and `cancel` consults that record. Dropping and reconstructing an
//! `ActExecutor` over the same policy therefore NEVER proves a pending
//! action was unsent — the fresh executor sees `sent == true` and refuses
//! the forged closure.
//!
//! ## Truncation, oversize, time
//!
//! Per the FROZEN contract, a stream at exactly [`MAX_STREAM_BODY`] with
//! `truncated: true` is the normal oversize outcome; a body LONGER than the
//! cap is unrepresentable and rejected as [`ActError::OversizeResult`].
//! `budget_ticks` bounds await steps (one per frame) — no clock in no_std.

use crate::command_record::request::CommandRequest;
use crate::command_record::result_contract::{
    ActionCommandRef, ActionResult, ResultStatus, StreamBody, MAX_STREAM_BODY,
};
use crate::session::{SessionError, SessionPolicy};
use crate::{Error, Frame, Policy};
use core::fmt;

/// Scripted act-body tags (this module's convention, not command-body v1).
const ACT_ACK: u8 = 1;
/// Status bytes of the scripted result body.
const STATUS_SUCCESS: u8 = 0;
const STATUS_FAILURE: u8 = 1;
const STATUS_UNKNOWN: u8 = 2;
/// Fixed prefix of an ACK act body: tag + u64 request_seq.
const ACK_BODY_LEN: usize = 9;
/// Fixed prefix of a scripted result body: status + two u16 lengths.
const RESULT_HEADER_LEN: usize = 5;

/// Distinct act-slice failures. Each names exactly one condition class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActError {
    /// Started but never acknowledged within the budget.
    NotAcked,
    /// Cancelled by the caller before the ACK landed; policy back to idle.
    /// Legal ONLY for a proven-unsent abandonment (`requests_sent == 0`).
    Cancelled,
    /// The request was already sent but never acknowledged: it may have
    /// executed on the device, so cancelling would forge evidence. Refused
    /// with zero policy mutation.
    SentNoAck,
    /// A frame answered a different request than the in-flight one, or
    /// carried a result before its ACK. Policy state is unchanged.
    CorrelationMismatch,
    /// A scripted stream exceeds [`MAX_STREAM_BODY`]: unrepresentable
    /// under the frozen contract. Policy state is unchanged.
    OversizeResult,
    /// Invalid envelope or act body.
    Malformed,
    /// The session policy rejected a transition.
    Policy(SessionError),
    /// The request could not be encoded.
    Transport(Error),
}

impl fmt::Display for ActError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let m = match self {
            ActError::NotAcked => "action never acknowledged within the budget",
            ActError::Cancelled => "action cancelled before its ACK",
            ActError::SentNoAck => {
                "action already sent without an ACK: cancelling would forge evidence"
            }
            ActError::CorrelationMismatch => "frame does not answer the in-flight request",
            ActError::OversizeResult => "scripted stream body exceeds the frozen stream cap",
            ActError::Malformed => "malformed scripted act frame",
            ActError::Policy(e) => return write!(f, "session policy error: {e}"),
            ActError::Transport(e) => return write!(f, "transport/encode error: {e}"),
        };
        write!(f, "{m}")
    }
}

/// A scenario script: an ordered list of already-encoded act frames
/// (built by [`script_ack`] / [`script_result`]) living in caller-owned
/// buffers; serving only borrows slices from them.
pub struct ScenarioScript<'s> {
    frames: &'s [(&'s [u8], usize)],
}

impl<'s> ScenarioScript<'s> {
    pub fn new(frames: &'s [(&'s [u8], usize)]) -> Self {
        Self { frames }
    }

    /// The frame bytes at `i`, or None past the end of the scenario.
    pub fn frame(&self, i: usize) -> Option<&'s [u8]> {
        self.frames.get(i).map(|(b, n)| &b[..*n])
    }
}

/// One scripted ACK frame: Command envelope, act body `[1, seq(8)]`.
pub fn script_ack(request_seq: u64, boot_id: &[u8; 16]) -> ([u8; 36 + ACK_BODY_LEN], usize) {
    let mut body = [0u8; ACK_BODY_LEN];
    body[0] = ACT_ACK;
    body[1..9].copy_from_slice(&request_seq.to_le_bytes());
    let mut frame = [0u8; 36 + ACK_BODY_LEN];
    let len = Frame::encode(
        crate::Op::Command,
        request_seq,
        boot_id,
        &body,
        &mut frame,
        &Policy::default(),
    )
    .expect("ack act frame is valid by construction");
    (frame, len)
}

/// One scripted RESULT frame: Command envelope whose correlation id is
/// the `request_seq` it answers, carrying the tiny result body.
pub fn script_result(
    request_seq: u64,
    status: ResultStatus,
    stdout: &[u8],
    stderr: &[u8],
    boot_id: &[u8; 16],
) -> ([u8; 36 + RESULT_HEADER_LEN + 2 * MAX_STREAM_BODY], usize) {
    // Lay out the tiny result body; streams above u16::MAX cannot be
    // declared, and the buffer holds any representable pair.
    assert!(
        stdout.len() <= u16::MAX as usize && stderr.len() <= u16::MAX as usize,
        "scripted stream length exceeds the u16 length field"
    );
    let mut body = [0u8; RESULT_HEADER_LEN + 2 * MAX_STREAM_BODY];
    body[0] = match status {
        ResultStatus::Success => STATUS_SUCCESS,
        ResultStatus::Failure => STATUS_FAILURE,
        ResultStatus::UnknownOutcome => STATUS_UNKNOWN,
    };
    body[1..3].copy_from_slice(&(stdout.len() as u16).to_le_bytes());
    body[3..3 + stdout.len()].copy_from_slice(stdout);
    let off = 3 + stdout.len();
    body[off..off + 2].copy_from_slice(&(stderr.len() as u16).to_le_bytes());
    body[off + 2..off + 2 + stderr.len()].copy_from_slice(stderr);
    let total = RESULT_HEADER_LEN + stdout.len() + stderr.len();
    let mut frame = [0u8; 36 + RESULT_HEADER_LEN + 2 * MAX_STREAM_BODY];
    let len = Frame::encode(
        crate::Op::Command,
        request_seq,
        boot_id,
        &body[..total],
        &mut frame,
        &Policy::default(),
    )
    .expect("result act frame is valid by construction");
    (frame, len)
}

/// A scenario transport: serves a [`ScenarioScript`]'s frames in order —
/// the same borrow-only serve contract the frozen [`FakeTransport`] offers,
/// pointed at the act scenario's own frames.
pub struct ScenarioTransport<'s> {
    script: &'s ScenarioScript<'s>,
    next: usize,
}

impl<'s> ScenarioTransport<'s> {
    pub fn new(script: &'s ScenarioScript<'s>) -> Self {
        Self { script, next: 0 }
    }

    /// Next scenario frame, or None at end of stream.
    pub fn serve(&mut self) -> Option<&'s [u8]> {
        let f = self.script.frame(self.next);
        if f.is_some() {
            self.next += 1;
        }
        f
    }
}

/// A bounded act executor: one request, one transport pass, no replay.
/// The scenario transport is fixed at construction so a failed `run` can
/// never drain and re-serve the same scripted frames (no blind replay).
pub struct ActExecutor<'a> {
    transport: &'a mut ScenarioTransport<'a>,
    policy: &'a mut SessionPolicy,
    boot_id: [u8; 16],
    session_id: u64,
    /// How many request frames have crossed the transport. Tests assert
    /// this stays at exactly 1 after a timeout — no blind replay — and
    /// `cancel` refuses once it is above 0 (sent, un-ACKed).
    requests_sent: usize,
    /// Set when a legal (proven-unsent) cancel interrupted an await loop.
    cancelled: bool,
}

impl<'a> ActExecutor<'a> {
    pub fn new(
        transport: &'a mut ScenarioTransport<'a>,
        policy: &'a mut SessionPolicy,
        boot_id: [u8; 16],
        session_id: u64,
    ) -> Self {
        Self {
            transport,
            policy,
            boot_id,
            session_id,
            requests_sent: 0,
            cancelled: false,
        }
    }

    /// Requests sent onto the transport by this executor (never re-sent).
    pub fn requests_sent(&self) -> usize {
        self.requests_sent
    }

    /// Cancel the in-flight action, legal ONLY for a proven-unsent
    /// abandonment — the POLICY's in-flight record for this id has
    /// `sent == false` (DC-CLI-02D session-level evidence, not
    /// executor-local state): the action is abandoned before anything
    /// crossed the
    /// transport, the policy returns to idle and no ActionResult is ever
    /// reported for it. The frozen policy has no Pending -> Ready edge, so
    /// the abandonment runs the machine's own close edge — a scripted
    /// no-op ACK plus a closure result (a Command slot closes only on a
    /// success *closure*, never reported for the command): Pending ->
    /// Acknowledged -> ResultKnown.
    /// If the request was already SENT but never acknowledged, it may have
    /// executed on the device: forging an ACK + success closure is not
    /// evidence, so the cancel is refused with [`ActError::SentNoAck`] and
    /// ZERO policy mutation. After a real ACK the cancel is likewise
    /// refused without mutation.
    pub fn cancel(&mut self, request_id: u64) -> Result<(), ActError> {
        let snap = self.policy.snapshot();
        if snap.inflight_id != Some(request_id) {
            return Err(ActError::Policy(SessionError::UnknownRequest));
        }
        if snap.state != crate::session::State::Pending || snap.acked == Some(true) {
            // Past the ACK the outcome is already in motion.
            return Err(ActError::Policy(SessionError::ActionInProgress));
        }
        if snap.sent != Some(false) {
            // The POLICY record says the request crossed (or may have
            // crossed) the transport: it may have executed on the
            // device. Forging an ACK + success closure is not evidence,
            // so refuse without touching the policy. This is session-level
            // send evidence (DC-CLI-02D): a fresh executor over the same
            // policy sees the same `sent` flag, so reconstructing an
            // executor never proves a pending action was unsent.
            return Err(ActError::SentNoAck);
        }
        self.policy
            .record_ack(request_id)
            .map_err(ActError::Policy)?;
        // A Command slot closes only on a success closure; the command
        // itself never produced and never reports an ActionResult.
        self.policy
            .record_result(request_id, true)
            .map_err(ActError::Policy)?;
        self.cancelled = true;
        Ok(())
    }

    /// Drive one request end to end within `budget_ticks` await steps.
    /// On timeout returns an UnknownOutcome [`ActionResult`] with empty
    /// bodies and the correlation set; the policy STAYS in-flight
    /// (no auto-reset, no re-send).
    pub fn run(
        &mut self,
        request: &CommandRequest<'_>,
        budget_ticks: usize,
    ) -> Result<ActionResult, ActError> {
        self.cancelled = false;
        // Next id = the policy's high-water mark + 1 (`start_action`
        // re-validates monotonicity). The action opens BEFORE the send:
        // nothing crosses the transport without a policy slot behind it.
        let request_id = self
            .policy
            .snapshot()
            .last_id
            .checked_add(1)
            .ok_or(ActError::Policy(SessionError::RequestExhausted))?;
        self.policy
            .start_action(crate::Op::Command, request_id)
            .map_err(ActError::Policy)?;

        // Send the request exactly once. Never re-sent on any failure path.
        let mut buf = [0u8; 36 + crate::command_record::MAX_COMMAND_BODY];
        request
            .encode_envelope(request_id, &self.boot_id, &mut buf, &Policy::default())
            .map_err(|e| match e {
                crate::command_record::CommandError::Envelope(err) => ActError::Transport(err),
                _ => ActError::Malformed,
            })?;
        // Stamp the session-owned send evidence BEFORE counting the send.
        // The policy refuses to stamp a non-in-flight id; a stamp can
        // never be un-earned and never survives as forgeable executor
        // state (DC-CLI-02D).
        self.policy
            .mark_sent(request_id)
            .map_err(ActError::Policy)?;
        self.requests_sent += 1;

        // Await the ACK.
        if !self.await_ack(request_id, budget_ticks)? {
            return Err(if self.cancelled {
                ActError::Cancelled
            } else {
                ActError::NotAcked
            });
        }

        // Await the result.
        match self.await_result(request_id, budget_ticks)? {
            Some(outcome) => {
                let (status, stdout, stderr) = outcome;
                // Closure rule: only a known Success or Failure result is
                // evidence that closes the slot. STATUS_UNKNOWN never
                // closes as success — the policy stays in-flight exactly
                // like the timeout path.
                match status {
                    ResultStatus::Success => self
                        .policy
                        .record_result(request_id, true)
                        .map_err(ActError::Policy)?,
                    // DC-ACT-FAILURE: a parsed Failure is a completed,
                    // reported outcome, so it closes the slot through the
                    // policy's dedicated known-command-failure edge — never
                    // through `record_result(_, true)`, which would report a
                    // success the device did not return.
                    ResultStatus::Failure => self
                        .policy
                        .record_command_failure(request_id)
                        .map_err(ActError::Policy)?,
                    ResultStatus::UnknownOutcome => {}
                }
                Ok(ActionResult {
                    status,
                    stdout,
                    stderr,
                    correlation: ActionCommandRef {
                        session_id: self.session_id,
                        request_seq: request_id,
                    },
                })
            }
            None => {
                // Timeout: the command MAY have executed. Report a distinct
                // UnknownOutcome with empty bodies and the correlation set;
                // the policy deliberately stays in-flight.
                Ok(ActionResult {
                    status: ResultStatus::UnknownOutcome,
                    stdout: empty_stream(),
                    stderr: empty_stream(),
                    correlation: ActionCommandRef {
                        session_id: self.session_id,
                        request_seq: request_id,
                    },
                })
            }
        }
    }

    /// Await an ACK for `request_id`, skipping unrecognized frames. A
    /// Command frame answering a different request — or a RESULT before
    /// its ACK — is a CorrelationMismatch. A cancel mid-await wins.
    fn await_ack(&mut self, request_id: u64, budget: usize) -> Result<bool, ActError> {
        for _ in 0..budget {
            if self.cancelled {
                return Ok(false);
            }
            let frame = match self.next_frame() {
                NextFrame::End => return Ok(false),
                NextFrame::Skip => continue,
                NextFrame::Frame(f) => f,
            };
            if frame.op() != crate::Op::Command {
                continue;
            }
            if frame.correlation_id() != request_id {
                return Err(ActError::CorrelationMismatch);
            }
            let body = frame.payload();
            if body.len() == ACK_BODY_LEN && body[0] == ACT_ACK {
                self.policy
                    .record_ack(request_id)
                    .map_err(ActError::Policy)?;
                return Ok(true);
            }
            if !body.is_empty() {
                // A result (or any non-ACK act body) before its ACK:
                // rejected with CorrelationMismatch semantics; the peek
                // happens before any policy mutation, so the policy is
                // left exactly as it was.
                return Err(ActError::CorrelationMismatch);
            }
            continue;
        }
        Ok(false)
    }

    /// Await the result for `request_id`. `Ok(None)` = budget spent.
    /// Oversize/malformed bodies end the await with an error, recording
    /// nothing. An UnknownOutcome status carries no closure bool: it must
    /// never close the slot as a known success (see `run`).
    fn await_result(
        &mut self,
        request_id: u64,
        budget: usize,
    ) -> Result<Option<(ResultStatus, StreamBody, StreamBody)>, ActError> {
        for _ in 0..budget {
            let frame = match self.next_frame() {
                NextFrame::End => return Ok(None),
                NextFrame::Skip => continue,
                NextFrame::Frame(f) => f,
            };
            if frame.op() != crate::Op::Command {
                continue;
            }
            if frame.correlation_id() != request_id {
                return Err(ActError::CorrelationMismatch);
            }
            let body = frame.payload();
            if body.len() == ACK_BODY_LEN && body[0] == ACT_ACK {
                // A duplicate ACK after the first is ignored.
                continue;
            }
            if body.len() < RESULT_HEADER_LEN {
                return Err(ActError::Malformed);
            }
            let status = match body[0] {
                STATUS_SUCCESS => ResultStatus::Success,
                STATUS_FAILURE => ResultStatus::Failure,
                STATUS_UNKNOWN => ResultStatus::UnknownOutcome,
                _ => return Err(ActError::Malformed),
            };
            let out_len = usize::from(u16::from_le_bytes(body[1..3].try_into().unwrap()));
            let out_start: usize = 3;
            let out_end = out_start.checked_add(out_len).ok_or(ActError::Malformed)?;
            let err_len_start = out_end;
            if err_len_start + 2 > body.len() {
                return Err(ActError::Malformed);
            }
            let err_len = usize::from(u16::from_le_bytes(
                body[err_len_start..err_len_start + 2].try_into().unwrap(),
            ));
            let err_start = err_len_start + 2;
            let err_end = err_start.checked_add(err_len).ok_or(ActError::Malformed)?;
            if err_end != body.len() {
                return Err(ActError::Malformed);
            }
            // A declared stream longer than MAX_STREAM_BODY cannot be
            // represented under the frozen contract at all.
            if out_len > MAX_STREAM_BODY || err_len > MAX_STREAM_BODY {
                return Err(ActError::OversizeResult);
            }
            let stdout = build_stream(&body[out_start..out_end])?;
            let stderr = build_stream(&body[err_start..err_end])?;
            return Ok(Some((status, stdout, stderr)));
        }
        Ok(None)
    }

    /// Classify the next served frame: envelope, skip, or end of stream.
    fn next_frame(&mut self) -> NextFrame<'_> {
        match self.transport.serve() {
            None => NextFrame::End,
            Some(bytes) => match Frame::parse(bytes, &Policy::default()) {
                Ok(f) => NextFrame::Frame(f),
                Err(_) => NextFrame::Skip,
            },
        }
    }
}

enum NextFrame<'b> {
    Frame(Frame<'b>),
    Skip,
    End,
}

/// Frozen-contract stream from scripted bytes. Over the cap is
/// unrepresentable (OversizeResult); exactly at the cap is the frozen
/// flagged-prefix outcome: truncated = true.
fn build_stream(bytes: &[u8]) -> Result<StreamBody, ActError> {
    if bytes.len() > MAX_STREAM_BODY {
        return Err(ActError::OversizeResult);
    }
    let mut stream = empty_stream();
    stream.bytes[..bytes.len()].copy_from_slice(bytes);
    stream.len = bytes.len();
    stream.truncated = bytes.len() == MAX_STREAM_BODY;
    Ok(stream)
}

fn empty_stream() -> StreamBody {
    StreamBody {
        bytes: [0u8; MAX_STREAM_BODY],
        len: 0,
        truncated: false,
    }
}
