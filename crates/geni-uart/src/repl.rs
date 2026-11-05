//! HW-29: device-independent console REPL glue — the join point between the
//! HW-23 output seam ([`TxSink`](crate::TxSink)) and the HW-26/HW-29 input
//! seam ([`ByteSource`](crate::ByteSource) + [`LineReader`](crate::rxline::LineReader)).
//!
//! [`Console`] is generic over **no** concrete type: its constructor takes
//! `&mut dyn TxSink` and `&mut dyn ByteSource`, so a caller wires the
//! concrete `GeniUart<R>` on both sides (or any future mock/tty device)
//! without the REPL layer naming the driver at all. This works because HW-29
//! made `LineReader`'s source a trait object (`&mut dyn ByteSource`; see
//! [`LineReader::new`](crate::rxline::LineReader::new) — existing generic
//! call sites keep compiling via coercion).
//!
//! ## Poll-loop contract
//! [`Console::echo_line`] is ONE poll-driven step: it performs exactly one
//! `LineReader::read_line` attempt and returns immediately — it never
//! busy-waits, loops internally, or uses timers. The **caller** drives:
//!
//! ```text
//! loop {
//!     match console.echo_line(&mut buf)? {
//!         Some(len) => { /* a COMPLETED line was echoed; print next prompt */ }
//!         None      => { /* pending — do other work, retry */ }
//!     }
//! }
//! ```
//!
//! The HW-29 gap contract is what makes this sound: a FIFO gap mid-line
//! yields `Ok(None)` with the partial still buffered inside the reader —
//! nothing is emitted on TX and no bytes are lost (root probe
//! `tests/uart_line_root_probe.rs`). A *completed empty* line (bare LF)
//! yields `Some(0)` and echoes `"you said: \n"`.
//!
//! HONESTY: host-mock tested only, exactly like the rest of this crate. The
//! glue itself is pure logic over the two seams, but both paths beneath it
//! (TX putc cycle, RX watermark re-arm / secondary-sequencer poll) have
//! never run on hardware — see HW-CONSOLE.md §HW-18/§HW-19/§HW-26.

use crate::rxline::{LineReader, RxLineError};
use crate::{ByteSource, TxSink, TxSinkError};
use core::fmt;

/// Errors the console glue can report. Input-side "nothing yet" is **not**
/// an error — it is `Ok(None)` from [`Console::echo_line`] (HW-29 contract:
/// `WouldReadNothing` mid-line means pending, not fault).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsoleError {
    /// The output sink failed. Carries the raw [`TxSinkError`] so callers
    /// can inspect the underlying [`TxError`](crate::TxError).
    Tx(TxSinkError),
    /// The line discipline rejected input — currently only
    /// [`RxLineError::LineTooLong`]. On this path the offending partial
    /// line has already been DISCARDED by the reader (HW-26 policy,
    /// unchanged by HW-29); see [`Console::echo_line`] docs.
    Rx(RxLineError),
}

impl fmt::Display for ConsoleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Tx(e) => write!(f, "console output sink failure: {e:?}"),
            Self::Rx(e) => write!(f, "console input line error: {e}"),
        }
    }
}

/// Device-independent console REPL glue (HW-29): prompt/echo over any
/// [`TxSink`], input over any [`ByteSource`] through the HW-26/HW-29
/// [`LineReader`].
pub struct Console<'a> {
    /// Output seam (HW-23).
    out: &'a mut dyn TxSink,
    /// The HW-26/HW-29 line discipline; holds the input fat reference
    /// (`&'a mut dyn ByteSource` lives inside it — `LineReader` defaults to
    /// `D = dyn ByteSource`).
    reader: LineReader<'a>,
}

impl<'a> Console<'a> {
    /// Wire an output sink and an input source into one console. Both may
    /// come from the same `GeniUart<R>` (reborrowed on both seams), with
    /// `init` + `start_rx` done first.
    pub fn new(out: &'a mut dyn TxSink, rx: &'a mut dyn ByteSource) -> Self {
        let reader = LineReader::new(rx);
        Self { out, reader }
    }

    /// Write `prompt` followed by an LF (the crate's LF-only newline policy,
    /// see [`TxSink::write_str`]). Returns the byte count written.
    pub fn write_prompt(&mut self, prompt: &str) -> Result<usize, ConsoleError> {
        self.out.write_str(prompt).map_err(ConsoleError::Tx)?;
        self.out.write_str("\n").map_err(ConsoleError::Tx)?;
        Ok(prompt.len() + 1)
    }

    /// One poll step of the REPL loop. See the module docs for the contract.
    ///
    /// Returns `Ok(Some(n))` — and echoes `"you said: <line>\n"` on TX —
    /// **only** when a line COMPLETED this poll (HW-29's
    /// [`LineReader::last_completed`]; includes completed-empty, `n = 0`).
    /// Returns `Ok(None)` while the line is pending across a FIFO gap:
    /// nothing is written to TX and the partial stays buffered. On a TX
    /// failure mid-echo the error surfaces as
    /// [`ConsoleError::Tx`]; note the line was already consumed by the
    /// reader, so bytes written before the failing one are not re-sent.
    pub fn echo_line(&mut self, buf: &mut [u8]) -> Result<Option<usize>, ConsoleError> {
        let n = self.reader.read_line(buf).map_err(ConsoleError::Rx)?;
        if !self.reader.last_completed() {
            // HW-29: pending — a gap drained the FIFO mid-line (or nothing
            // arrived). Emit nothing; the partial survives inside `reader`.
            return Ok(None);
        }
        self.out.write_str("you said: ").map_err(ConsoleError::Tx)?;
        let line = core::str::from_utf8(&buf[..n]).unwrap_or("<invalid utf-8>");
        self.out.write_str(line).map_err(ConsoleError::Tx)?;
        self.out.write_str("\n").map_err(ConsoleError::Tx)?;
        Ok(Some(n))
    }
}
