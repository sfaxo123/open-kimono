//! HW-26: console INPUT line discipline on top of the HW-19 RX primitives.
//!
//! [`LineReader`] turns the raw byte source ([`GeniUart::read_byte`], which
//! mirrors the oracle's getc) into terminal-style lines: LF termination, CR
//! swallowing, backspace editing, control-character filtering and a
//! too-long-line policy. It is **generic over the driver `D`**, not over the
//! register seam `R: SeRegs` directly — deliberate choice: the line layer
//! only needs one byte at a time, so it depends on [`ByteSource`] (an
//! object-safe byte-input trait implemented by [`GeniUart<D>`] for every
//! `D: SeRegs`). Consumers (a later Redox tty / console REPL) depend on the
//! trait and never name the driver type — the same seam shape as HW-23's
//! [`TxSink`](`crate::TxSink`) on the output side.
//!
//! HONESTY: host-mock tested only, exactly like the rest of this crate. The
//! line discipline itself is pure logic over the byte seam, but the RX path
//! beneath it (watermark re-arm, secondary-sequencer poll) has never run on
//! hardware; geni RX watermark registers are oracle-cited in HW-CONSOLE.md
//! §HW-18/§HW-19.

use crate::{ByteSource, ByteSourceError, RxError};
use alloc::vec::Vec;
use core::fmt;

/// ASCII line-feed — ends a line and is consumed (not returned).
pub const LF: u8 = 0x0A;
/// ASCII carriage return — treated as a line ending too, but swallowed
/// (see [`LineReader`] docs: CR alone ends a line; CRLF ends ONE line).
pub const CR: u8 = 0x0D;
/// ASCII backspace (BS) — erases the last buffered byte.
pub const BS: u8 = 0x08;
/// ASCII delete (DEL) — also treated as backspace, the VT100 convention.
pub const DEL: u8 = 0x7F;

/// HW-31: hard cap on how many bytes one [`LineReader::read_line`] call may
/// consume from the [`ByteSource`] — the per-call work budget. When the
/// budget is exhausted without a line completing, the call returns `Ok(0)`
/// with [`LineReader::last_completed`] `false` (pending) and preserves all
/// line state exactly like the FIFO-gap path; the caller just loops. This
/// bounds per-call work even for adversarial streams (continuous control
/// bytes, BS, discard-mode bytes) that never produce a line end.
pub const MAX_BYTES_PER_CALL: usize = 256;

/// Errors the line discipline can report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RxLineError {
    /// The incoming line exceeded the caller's `buf` capacity. Carries the
    /// capacity (`buf.len()`) that was exceeded — the maximum storable line
    /// length for this call. **Policy: the entire partial line is
    /// DISCARDED** (the reader's internal buffer is emptied and everything
    /// already assembled for that line is lost — nothing is copied into
    /// `buf`). Bytes already sitting in the driver FIFO are NOT flushed:
    /// they keep arriving, so the remainder of the over-long line up to and
    /// including its terminating LF is **swallowed silently** (the error is
    /// reported once per over-long line), and bytes after that LF start a
    /// fresh clean line. This keeps a flood of junk from wedging the
    /// console or spamming the caller with repeat errors.
    LineTooLong(usize),
}

impl fmt::Display for RxLineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LineTooLong(cap) => write!(
                f,
                "input line exceeded buffer capacity {cap}; partial line discarded"
            ),
        }
    }
}

/// Line-oriented console reader over any [`ByteSource`] (HW-26; gap
/// contract fixed in HW-29).
///
/// ## Allocation contract (HW-33; qualified to the reader in HW-33B)
/// Measured with the `#[cfg(test)]` counting global allocator in
/// `src/tests.rs` (mechanism (a): a wrapper around the system allocator that
/// counts alloc/dealloc CALLS, gated per-thread so each scenario's
/// `hw33_allocs()` snapshot delta counts only its own allocations — see the
/// module docs on `hw33_alloc` in `src/tests.rs` for determinism notes).
/// HW-33B: growth claims are pinned to the **actual** [`LineReader`] (via
/// the `#[cfg(test)]`-only capacity accessor `line_capacity()` and per-line
/// allocator deltas), not a plain-`Vec` proxy; zero-allocation claims state
/// exactly which windows were measured warm.
/// - **Exactly ONE growable buffer.** A [`LineReader`] owns exactly one
///   heap-allocated, growable line buffer (`line: Vec<u8>`). The caller's
///   `read_line` `buf` is borrowed and never allocated by this layer — the
///   copy out ([`LineReader::take_into`]) is a plain slice copy.
/// - **Capacity persists across calls (warm reuse).** Buffer capacity is
///   never shrunk: after the first line that needs it, subsequent lines of
///   up to that length cause **zero allocations** — measured 0 allocs /
///   0 deallocs across a 1000-line repeated workload through ONE reader once
///   warm (pinned by `alloc_repeated_workload_zero_churn`).
/// - **Growth is amortized doubling, with this exact bound — measured ON
///   THE READER.** Driving lines of length `n` (every `n` in `1..=300`)
///   through ONE fresh [`LineReader`], the internal buffer's capacity after
///   each line is exactly the smallest power of two `>= max(n, 8)` — never
///   smaller, never more than 2x the longest buffered line — and reaching
///   length 300 costs the reader exactly **7 allocations** total (blocks
///   8,16,32,…,512). Pinned by `alloc_growth_bound_pow2_on_reader`
///   (allocator deltas + `line_capacity()` on the real reader). The bound
///   matches what HW-33 measured on a bare `Vec`, because the reader's
///   buffer is that `Vec` and `read_line` pushes through it byte-by-byte.
/// - **Gap and budget paths allocate nothing — in WARM windows only.** A
///   FIFO-gap `Ok(0)` (pending) and HW-31 budget-exhaustion `Ok(0)` were
///   measured at 0 allocs / 0 deallocs **when the internal buffer was
///   already grown** (warm from a prior line) and the scenario moved only
///   control/BS bytes that never enter the buffer. That is NOT a blanket
///   "allocate nothing": a cold gap return that accumulates printable bytes
///   into a not-yet-grown buffer DOES pay growth-block allocations, exactly
///   as in the warm-reuse bullet above (`alloc_gap_and_budget_allocate_nothing`
///   measures only the warm windows; `alloc_growth_bound_pow2_on_reader`
///   pins what cold accumulation costs).
/// - **`LineTooLong` keeps capacity.** Discarding the partial line empties
///   the buffer but NEVER shrinks its capacity (measured: zero allocs for
///   the next same-length line — `alloc_line_too_long_keeps_capacity`).
///
/// (Measured truth note: the brief's naive guess "capacity <= smallest
/// power-of-two >= longest buffered line" is off **below 8 bytes** — the
/// real bound, measured on the reader itself, is `max(n, 8)` rounded up to a
/// power of two. The doc states the measured behavior.)
///
/// ## The FIFO-gap contract (HW-29 — supersedes HW-26's "return what we have")
/// HW-26 returned the buffered partial as if it were a line whenever a call
/// boundary landed on an empty FIFO ([`RxError::WouldReadNothing`],
/// [`RxError::SecondaryTimeout`]) — source `a, b, GAP, c, LF` came back as
/// two lines `"ab"` + `"c"`. That is a **real defect**: one line was split
/// across a FIFO gap. Root advisory (HW-29): an empty read mid-line must
/// NOT emit the partial and must NOT clear state — the bytes stay buffered
/// and the next call continues them. The independent root probe
/// `tests/uart_line_root_probe.rs` pins this: after `a, b, GAP` the first
/// `read_line` returns `Ok(0)` **with nothing completed**, and after `c, LF`
/// the second returns `Ok(3)` with `"abc"`.
///
/// So `Ok(0)` is ambiguous on its face — "no line completed *this poll*"
/// (pending) vs "a *completed empty line*" (a bare LF arriving with an
/// empty buffer). [`LineReader::last_completed`] disambiguates:
/// - **pending** (gap, or FIFO drained mid-line): `read_line -> Ok(0)` and
///   `last_completed() == false` — retry; the partial stays in the reader.
/// - **completed-empty** (bare LF on an empty buffer): `Ok(0)` and
///   `last_completed() == true` — a real (empty) line did end.
/// A completed non-empty line (`Ok(n > 0)`) always implies a completion;
/// the flag is what makes the *zero-length* case explicit for consumers
/// like the REPL ([`Console::echo_line`](crate::repl::Console::echo_line)).
///
/// ## Line rules (documented contract)
/// - **LF (0x0A)** ends a line: it is consumed and NOT part of the returned
///   bytes; `read_line` returns the count of buffered bytes before it.
/// - **CR (0x0D)** is also a line ending, but *swallowed*: CR alone ends a
///   line, and CRLF ends exactly ONE line — when a CR has just ended a line
///   (inside a `read_line` call, or in a previous call whose line ended at
///   CR), an immediately following LF is dropped. A stray LF that arrives
///   with an empty buffer ends an empty line: `read_line` returns `Ok(0)`
///   with the LF consumed and `last_completed() == true`.
/// - **Backspace — BS (0x08) and DEL (0x7F)** erase the last buffered byte
///   when the buffer is non-empty; on an empty buffer they are ignored
///   silently. Editing is in-buffer only: nothing is echoed to TX (echo is
///   a consumer concern above this layer).
/// - **Control bytes**: any byte `< 0x20` other than CR/LF/BS is dropped
///   silently (NUL, tabs, ETX, etc. never enter the buffer). DEL is handled
///   as backspace per the rule above; all other bytes `>= 0x20` (including
///   high-bit bytes) are stored verbatim — no UTF-8 validation at this
///   layer.
/// - **Buffer full**: if a storable byte arrives when the internal buffer
///   already holds `buf.len()` bytes, `read_line` returns
///   [`RxLineError::LineTooLong(buf.len())`] and DISCARDS the partial line
///   (see the variant docs). The offending byte is dropped too.
/// - **Empty read is NOT an error and does NOT end a line** (HW-29): exactly
///   one driver poll per incoming byte, and `read_line` returns as soon as
///   the driver reports [`RxError::WouldReadNothing`] — no busy-wait loop
///   inside. Buffered partial state survives between calls, so the caller
///   just loops. A secondary-sequencer timeout is treated identically: the
///   partial stays buffered and `Ok(0)` with `last_completed() == false`
///   means "nothing completed yet" — never "here is a line".
/// - **Bounded per-call work (HW-31)**: one `read_line` call consumes **at
///   most [`MAX_BYTES_PER_CALL`]** bytes from the source. When the budget
///   runs out without a line completing, the call returns `Ok(0)` with
///   `last_completed() == false` (pending) and preserves ALL state — the
///   partial line, `pending_cr`, and any in-progress `discard_line` —
///   exactly like the FIFO-gap path; the caller just loops. Together with
///   the empty-read rule this makes the "no busy-wait" claim hold for **any
///   stream**, including continuous control-byte / BS / discard-mode
///   streams that never end a line: each call does at most
///   `MAX_BYTES_PER_CALL` source reads, never an unbounded loop.
///   (Interaction with `LineTooLong`: the too-long check fires on the
///   offending byte before any budget return, so a storable overflow is
///   still reported as `LineTooLong` within the budget; the budget only
///   kicks in when no line end, and no overflow, occurred in those reads.)
pub struct LineReader<'d> {
    source: &'d mut dyn ByteSource,
    /// Partial-line buffer (the line being assembled across polls).
    line: Vec<u8>,
    /// True right after this call returned a line that ended at CR: an
    /// immediately following LF is swallowed (CRLF = one line).
    pending_cr: bool,
    /// HW-29: true iff the **most recent** `read_line` call completed a
    /// line — including a zero-length one (bare LF on an empty buffer).
    /// False when that call stopped at an empty FIFO / timeout with a
    /// pending partial. Read via [`LineReader::last_completed`].
    last_completed: bool,
    /// True after a `LineTooLong` was reported: the rest of the discarded
    /// line (up to and including its terminating LF) is swallowed, so the
    /// next line starts cleanly at the first post-LF byte. A `LineTooLong`
    /// is reported **once** per over-long line — while this flag is set,
    /// further bytes are dropped silently until the LF.
    discard_line: bool,
}

impl<'d> LineReader<'d> {
    /// Bind the line reader to a byte source. Typically a `&mut GeniUart<_>`
    /// — since HW-29 the source is stored as `&mut dyn ByteSource`, so
    /// existing call sites like `LineReader::new(&mut uart)` keep compiling
    /// via unsized coercion, and consumers such as
    /// [`Console`](crate::repl::Console) can hand in a trait object.
    pub fn new(source: &'d mut dyn ByteSource) -> Self {
        Self {
            source,
            line: Vec::new(),
            pending_cr: false,
            discard_line: false,
            last_completed: false,
        }
    }

    /// HW-33B test seam: the **actual** capacity of the reader's internal
    /// line buffer, so allocation-growth claims can be pinned to the real
    /// [`LineReader`] instead of a plain-`Vec` proxy. `#[cfg(test)]` only —
    /// it is not part of the public API and never compiles into production.
    #[cfg(test)]
    pub(crate) fn line_capacity(&self) -> usize {
        self.line.capacity()
    }

    /// HW-29: did the **most recent** [`LineReader::read_line`] call com-
    /// plete a line? `true` after any line end — including a completed
    /// *empty* line (bare LF on an empty buffer, which returns `Ok(0)`);
    /// `false` when that call stopped at an empty FIFO / secondary timeout
    /// with the partial still buffered (pending). Together with the `Ok(0)`
    /// return this makes pending-vs-completed-empty explicit; see the type
    /// docs ("The FIFO-gap contract").
    pub fn last_completed(&self) -> bool {
        self.last_completed
    }

    /// Read one line into `buf`. See the type docs for the full rule table.
    ///
    /// Returns `Ok(n)` with `n > 0` when a line ended and was copied into
    /// `buf`; `Ok(0)` **either** when a completed *empty* line ended (bare
    /// LF — check [`LineReader::last_completed`] == true) **or** when the
    /// driver had nothing to read this poll and any partial stays buffered
    /// (`last_completed` == false — HW-29: retry, never a lost line) **or**
    /// when this call exhausted its [`MAX_BYTES_PER_CALL`] byte budget
    /// without a line completing (HW-31: pending, all state preserved, just
    /// call again); and `Err(RxLineError::LineTooLong(buf.len()))` when an
    /// incoming byte did not fit — in which case the partial line has been
    /// discarded.
    ///
    /// ## Caller-buffer capacity contract (HW-32 — explicit, pinned by tests)
    /// The capacity `cap = buf.len()` is the **maximum COMPLETED line length
    /// deliverable in one call**. It is a caller-chosen bound with no other
    /// obligation — callers need NOT relate it to [`MAX_BYTES_PER_CALL`]:
    /// - **Exact fit completes.** A line of exactly `cap` storable bytes
    ///   followed by LF completes: `Ok(cap)`, all `cap` bytes copied into
    ///   `buf`. The overflow check fires only when a storable byte arrives
    ///   with `line.len() >= cap`, so `cap` bytes always fit.
    /// - **A line longer than `cap` fails once, cleanly.** On the first
    ///   storable byte that would exceed `cap` (the `cap + 1`-th), the call
    ///   returns `Err(RxLineError::LineTooLong(cap))`, the entire partial
    ///   line is DISCARDED (nothing copied into `buf`), and the remainder of
    ///   that line up to and including its terminating LF is **swallowed** —
    ///   exactly one error per over-long line, and the next line after the
    ///   LF completes normally (pinned by `cap_smaller_than_line`).
    /// - **`cap` may change per call; a shrinking cap never drops bytes
    ///   silently (HW-34).** A caller may legitimately pass a *smaller* `buf`
    ///   on a later call. If at line completion (LF or CR) the buffered line
    ///   is longer than the current `cap`, the call returns
    ///   `Err(RxLineError::LineTooLong(cap))` — partial discarded, terminator
    ///   consumed, **never** a silent `min(len, cap)` + clear that loses the
    ///   tail (pinned by `shrinking_cap_completion_errors`; state stays clean
    ///   for the next line: `shrinking_cap_next_line_ok`). This fires even
    ///   when the completing byte is the LF/CR itself — a pending 4-byte line
    ///   completed into a 2-byte buffer is an explicit error, not `Ok(2)`.
    /// - **`cap` may EXCEED the per-call budget.** The two bounds are
    ///   independent: with `cap > MAX_BYTES_PER_CALL`, a line longer than
    ///   the budget but within capacity completes **across calls, losslessly**
    ///   — budget exhaustion returns pending `Ok(0)` (`last_completed() ==
    ///   false`) with all state preserved, and the next call continues from
    ///   the exact partial: no byte is lost or duplicated across the budget
    ///   boundary (pinned by `lossless_across_budget`).
    /// - **`cap == 0` is defined behavior.** With capacity zero, the very
    ///   first storable byte of a line (`>= 0x20`, not BS/DEL) immediately
    ///   returns `Err(RxLineError::LineTooLong(0))` — the partial (empty) is
    ///   "discarded" and the rest of that line is swallowed to its LF. A
    ///   zero-cap call with no storable byte still advances state normally:
    ///   a bare LF completes an empty line `Ok(0)` with
    ///   `last_completed() == true`; control bytes are dropped; a FIFO gap
    ///   returns pending `Ok(0)`. A zero-length `buf` never receives bytes.
    ///   (Pinned by `zero_cap_defined`.)
    /// - **Losslessness claim.** Except for the *documented* discard rules —
    ///   control bytes `< 0x20` (other than CR/LF/BS) dropped, BS/DEL edit
    ///   the buffer, and a `LineTooLong` line discarded-and-swallowed-to-LF
    ///   — **every source byte is either consumed into the line state or
    ///   ends a line: no silent loss.** Budget exhaustion and FIFO gaps
    ///   consume bytes into preserved state (they never drop what they read),
    ///   and CR/LF are consumed as line endings.
    pub fn read_line(&mut self, buf: &mut [u8]) -> Result<usize, RxLineError> {
        // HW-31: per-call work budget. Every source read (success OR
        // gap-error) counts against it, so no call can loop longer than
        // MAX_BYTES_PER_CALL iterations regardless of the byte stream.
        let mut budget = MAX_BYTES_PER_CALL;
        loop {
            if budget == 0 {
                // HW-31: budget exhausted without a completed line. Return
                // pending with ALL state (partial line, pending_cr,
                // discard_line) untouched — identical shape to the gap path.
                self.last_completed = false;
                return Ok(0);
            }
            budget -= 1;
            let b = match self.source.read_byte() {
                Ok(b) => b,
                // Empty FIFO and secondary-timeout alike: not errors, and
                // NOT line endings (HW-29). The partial stays buffered;
                // `Ok(0)` + `last_completed == false` means "pending".
                Err(ByteSourceError::Rx(RxError::WouldReadNothing | RxError::SecondaryTimeout)) => {
                    self.last_completed = false;
                    return Ok(0);
                }
            };

            if self.discard_line {
                // Swallow the remainder of the discarded line until its
                // LF; bytes after that LF start a fresh line.
                if b == LF {
                    self.discard_line = false;
                }
                continue;
            }

            if self.pending_cr && b == LF {
                // Second half of a CRLF pair: swallow the LF.
                self.pending_cr = false;
                continue;
            }
            self.pending_cr = false;

            match b {
                LF => {
                    // HW-34: a caller cap that SHRANK between calls must
                    // never silently truncate. If the buffered line no
                    // longer fits `buf`, complete with an explicit
                    // LineTooLong instead of min+clear: partial discarded,
                    // terminator consumed — same contract as store-path
                    // too-long (except no swallow-to-LF: the LF ending
                    // this line was just consumed).
                    if self.line.len() > buf.len() {
                        self.line.clear();
                        self.last_completed = false;
                        return Err(RxLineError::LineTooLong(buf.len()));
                    }
                    let n = self.take_into(buf);
                    self.last_completed = true;
                    return Ok(n);
                }
                CR => {
                    if !self.line.is_empty() {
                        // HW-34: same shrinking-cap rule on the CR path.
                        // HW-36: the CR terminator is consumed by this
                        // error return, so `pending_cr` MUST be set —
                        // otherwise the matching LF of the CRLF pair is
                        // treated as a bare LF on the next call and
                        // manufactures a phantom completed empty line
                        // (`Ok(0)`, `last_completed == true`). Pinned by
                        // `cr_shrinking_cap_then_lf_no_phantom_line`.
                        if self.line.len() > buf.len() {
                            self.line.clear();
                            self.pending_cr = true;
                            self.last_completed = false;
                            return Err(RxLineError::LineTooLong(buf.len()));
                        }
                        self.pending_cr = true;
                        let n = self.take_into(buf);
                        self.last_completed = true;
                        return Ok(n);
                    }
                    // Bare CR with an empty buffer: ignored.
                }
                BS | DEL => {
                    self.line.pop();
                }
                _ if b < 0x20 => {
                    // Other control bytes: dropped silently.
                }
                _ => {
                    if self.line.len() >= buf.len() {
                        self.line.clear();
                        self.discard_line = true;
                        self.last_completed = false;
                        return Err(RxLineError::LineTooLong(buf.len()));
                    }
                    self.line.push(b);
                }
            }
        }
    }

    /// Copy the assembled line into `buf`, reset state, return the length.
    fn take_into(&mut self, buf: &mut [u8]) -> usize {
        let n = core::cmp::min(self.line.len(), buf.len());
        buf[..n].copy_from_slice(&self.line[..n]);
        self.line.clear();
        n
    }
}
