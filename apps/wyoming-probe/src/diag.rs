//! Opt-in raw-I/O diagnostic. OFF by default; the probe protocol is untouched
//! and no Wyoming frame is sent here, so this can never stand in for a probe
//! result.
//!
//! Why: guest phase `wy-1792954191-guest3` armed the nonblocking adapter for
//! 10 s and printed no `PROBE FAIL` in 20 s, and the logs could not say which
//! primitive never returned. Each phase here emits TWO lines: an UNTIMED raw
//! line flushed before any clock is touched, then a stamped line. The tail of
//! the log then separates four distinct failures:
//!
//! * raw phase line with no matching stamped line → the `Instant` read itself
//!   hung (the clock call, not the primitive, is the hang point);
//! * stamped phase line with no following raw phase line → that primitive
//!   blocked (e.g. `read_before` and nothing after = the read syscall);
//! * `retry_sleep_before` with no `retry_sleep_after` → `thread::sleep` never
//!   returned;
//! * stamps flat across a real sleep, or `retry_cap_hit` while
//!   `op_elapsed_us` is far below `op_timeout_us` → clock not advancing.
//!
//! HONEST LIMITS. The retry cap bounds *returned iterations*, not a read,
//! sleep or clock call that never returns: nothing in this module can bound
//! such a hang, so the external `timeout --kill-after` around the process
//! remains required. A `deadline_hit` is the in-process bound working
//! correctly against a silent peer — NOT a primitive that failed to return.
//! Only the first read/retry is individually bracketed; a later stall can be
//! in a subsequent read, sleep or clock check and requires a narrower follow-up.
//! Output is bounded: two lines per phase, never per iteration or per byte.

use std::io::{ErrorKind, Read, Write};
use std::net::TcpStream;
use std::thread;
use std::time::{Duration, Instant};

use crate::deadline::RETRY_BACKOFF;

/// Phase names, fixed and finite: greppable in the serial log, and what the
/// host tests assert on.
pub mod phase {
    pub const CLOCK_INIT_BEFORE: &str = "clock_init_before";
    pub const CLOCK_INIT_AFTER: &str = "clock_init_after";
    pub const START: &str = "start";
    pub const CLOCK_A: &str = "clock_a";
    pub const PROBE_SLEEP_BEFORE: &str = "probe_sleep_before";
    pub const PROBE_SLEEP_AFTER: &str = "probe_sleep_after";
    pub const CLOCK_B: &str = "clock_b";
    pub const NB_BEFORE: &str = "nonblocking_before";
    pub const NB_OK: &str = "nonblocking_ok";
    pub const NB_ERR: &str = "nonblocking_err";
    pub const WRITE_BEFORE: &str = "write_before";
    pub const WRITE_DATA: &str = "write_data";
    pub const WRITE_WOULDBLOCK: &str = "write_wouldblock";
    pub const WRITE_ERR: &str = "write_err";
    pub const READ_BEFORE: &str = "read_before";
    pub const READ_DATA: &str = "read_data";
    pub const READ_EOF: &str = "read_eof";
    pub const READ_WOULDBLOCK: &str = "read_wouldblock";
    pub const READ_ERR: &str = "read_err";
    pub const RETRY_SLEEP_BEFORE: &str = "retry_sleep_before";
    pub const RETRY_SLEEP_AFTER: &str = "retry_sleep_after";
    pub const DEADLINE_HIT: &str = "deadline_hit";
    pub const RETRY_CAP_HIT: &str = "retry_cap_hit";
    pub const DONE: &str = "done";
}

/// One stamped phase marker (the raw pre-stamp line is emitted separately).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Marker {
    pub seq: u32,
    pub phase: &'static str,
    /// Microseconds since the clock was constructed, or 0 when the clock read
    /// did not return (raw line only).
    pub t_us: u128,
    pub note: Option<String>,
}

/// Marker sink. Records markers for the host tests; when `echo` is set it also
/// prints them, flushing the raw line before the clock is read.
pub struct Diag {
    started: Instant,
    seq: u32,
    echo: bool,
    markers: Vec<Marker>,
}

/// Untimed line: safe to emit when the clock may be the thing that is hung.
fn raw_line(seq: u32, phase: &str) {
    println!("DIAG-R {seq} {phase}");
    let _ = std::io::stdout().flush();
}

impl Diag {
    /// Even the initial `Instant::now()` is bracketed by untimed raw lines, so
    /// a hang inside the first clock read is still visible in the log.
    pub fn new(echo: bool) -> Self {
        if echo {
            raw_line(0, phase::CLOCK_INIT_BEFORE);
        }
        let started = Instant::now();
        if echo {
            raw_line(0, phase::CLOCK_INIT_AFTER);
        }
        Self {
            started,
            seq: 0,
            echo,
            markers: Vec::new(),
        }
    }

    /// Emit one phase: raw line first (flushed, no clock touched), then the
    /// clock sample, then the stamped line. Returns the stamp.
    pub fn mark(&mut self, phase: &'static str, note: Option<String>) -> u128 {
        if self.echo {
            raw_line(self.seq, phase);
        }
        let t_us = self.started.elapsed().as_micros();
        let m = Marker {
            seq: self.seq,
            phase,
            t_us,
            note: note.clone(),
        };
        self.seq += 1;
        if self.echo {
            match &note {
                Some(n) => println!("DIAG {} t_us={} {} {}", m.seq, t_us, phase, n),
                None => println!("DIAG {} t_us={} {}", m.seq, t_us, phase),
            }
            let _ = std::io::stdout().flush();
        }
        self.markers.push(m);
        t_us
    }

    pub fn markers(&self) -> &[Marker] {
        &self.markers
    }

    #[cfg(test)]
    pub fn phases(&self) -> Vec<&'static str> {
        self.markers.iter().map(|m| m.phase).collect()
    }

    #[cfg(test)]
    pub fn stamp(&self, phase: &'static str) -> Option<u128> {
        self.markers
            .iter()
            .find(|m| m.phase == phase)
            .map(|m| m.t_us)
    }
}

/// Bounds for one diagnostic run.
#[derive(Debug, Clone, Copy)]
pub struct DiagConfig {
    /// Per-operation wall-clock deadline, same role as the adapter's.
    pub op_timeout: Duration,
    /// Sleep between nonblocking retries — the adapter's own backoff.
    pub backoff: Duration,
    /// Bounds RETURNED iterations only (see module limits). Sized so a
    /// working clock reaches `op_timeout` first.
    pub max_retries: u32,
}

impl DiagConfig {
    pub fn new(op_timeout: Duration) -> Self {
        let backoff = RETRY_BACKOFF;
        let retries = (op_timeout.as_micros() / backoff.as_micros()).saturating_mul(4) + 8;
        Self {
            op_timeout,
            backoff,
            max_retries: retries.min(u128::from(u32::MAX)) as u32,
        }
    }
}

/// What the run concluded, as a truthful class — not "something hung".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// The read answered with bytes or EOF: no hang on this socket.
    ReadAnswered,
    /// Every read returned WouldBlock and the in-process deadline fired. The
    /// bounded silent-peer timeout WORKED — this is the known-good control.
    BoundedTimeout,
    /// The read returned an error other than WouldBlock: the primitive
    /// answered, the socket did not.
    ReadErrored,
    /// `set_nonblocking` failed, so nothing downstream can be bounded.
    NonblockingFailed,
    /// The clock-independent cap ended the leg before the deadline: either
    /// `max_retries` is below `op_timeout/backoff` or the clock is not
    /// advancing. The last raw phase names where iterations were still going.
    RetryCapHit,
}

impl Verdict {
    pub fn name(&self) -> &'static str {
        match self {
            Verdict::ReadAnswered => "read_answered",
            Verdict::BoundedTimeout => "bounded_silent_timeout_ok",
            Verdict::ReadErrored => "read_answered_with_error",
            Verdict::NonblockingFailed => "nonblocking_failed",
            Verdict::RetryCapHit => "retry_cap_before_deadline",
        }
    }

    /// 0 = every primitive answered (including a correct bounded timeout);
    /// 1 = something failed to answer, or the clock looks non-advancing.
    pub fn exit_code(&self) -> i32 {
        match self {
            Verdict::ReadAnswered | Verdict::BoundedTimeout | Verdict::ReadErrored => 0,
            Verdict::NonblockingFailed | Verdict::RetryCapHit => 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub verdict: Verdict,
    /// Marker naming the FIRST raw write result.
    pub write_result: Option<&'static str>,
    /// Marker naming the FIRST raw read result.
    pub read_result: Option<&'static str>,
    pub retries: u32,
    /// `clock_b - clock_a` microseconds; 0 across a real sleep = dead clock.
    pub clock_advanced_us: u128,
    pub nonblocking_took_us: u128,
}

/// Bytes written by the write leg: tiny and fixed, the point is the first
/// result of the primitive, not throughput.
const PROBE_BYTES: &[u8] = b"wy-diag";

/// Exercise the primitives the deadline adapter depends on, in the order the
/// real probe uses them, against ONE socket.
pub fn run(stream: &TcpStream, cfg: &DiagConfig, d: &mut Diag) -> Outcome {
    d.mark(
        phase::START,
        Some(format!(
            "op_timeout_us={} backoff_us={} max_retries={}",
            cfg.op_timeout.as_micros(),
            cfg.backoff.as_micros(),
            cfg.max_retries
        )),
    );

    // Clock pair around one real sleep: a returned sleep with identical
    // stamps means the monotonic clock is not advancing.
    let clock_a = d.mark(phase::CLOCK_A, None);
    d.mark(
        phase::PROBE_SLEEP_BEFORE,
        Some(format!("{}us", cfg.backoff.as_micros())),
    );
    thread::sleep(cfg.backoff);
    d.mark(phase::PROBE_SLEEP_AFTER, None);
    let clock_b = d.mark(phase::CLOCK_B, None);
    let clock_advanced_us = clock_b.saturating_sub(clock_a);

    let nb_before = d.mark(phase::NB_BEFORE, None);
    let mut nb_took_us = 0u128;
    let nb_ok = match stream.set_nonblocking(true) {
        Ok(()) => {
            nb_took_us = d.mark(phase::NB_OK, None).saturating_sub(nb_before);
            true
        }
        Err(e) => {
            d.mark(phase::NB_ERR, Some(format!("{e}")));
            false
        }
    };
    if !nb_ok {
        d.mark(phase::DONE, Some("verdict=nonblocking_failed".to_string()));
        return Outcome {
            verdict: Verdict::NonblockingFailed,
            write_result: None,
            read_result: None,
            retries: 0,
            clock_advanced_us,
            nonblocking_took_us: nb_took_us,
        };
    }

    d.mark(
        phase::WRITE_BEFORE,
        Some(format!("{} bytes", PROBE_BYTES.len())),
    );
    let mut writer = stream;
    let write_result = match writer.write(PROBE_BYTES) {
        Ok(n) => {
            d.mark(phase::WRITE_DATA, Some(format!("n={n}")));
            phase::WRITE_DATA
        }
        Err(e) if e.kind() == ErrorKind::WouldBlock => {
            d.mark(phase::WRITE_WOULDBLOCK, None);
            phase::WRITE_WOULDBLOCK
        }
        Err(e) => {
            d.mark(phase::WRITE_ERR, Some(format!("kind={:?}", e.kind())));
            phase::WRITE_ERR
        }
    };

    let mut buf = [0u8; 64];
    if d.echo {
        raw_line(d.seq, "operation_clock_before");
    }
    let op_start = Instant::now();
    let until = op_start + cfg.op_timeout;
    if d.echo {
        raw_line(d.seq, "operation_clock_after");
    }
    d.mark(phase::READ_BEFORE, Some(format!("cap={}", buf.len())));
    let mut reader = stream;
    let mut retries = 0u32;
    let mut read_result: Option<&'static str> = None;
    let ended_by = loop {
        match reader.read(&mut buf) {
            Ok(n) => {
                let ph = if n == 0 {
                    phase::READ_EOF
                } else {
                    phase::READ_DATA
                };
                d.mark(ph, Some(format!("n={n} retries={retries}")));
                read_result.get_or_insert(ph);
                break ph;
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => {
                // Sleep on EVERY retry, like the adapter: a spin loop would
                // reach the cap in microseconds and masquerade as a dead
                // clock. Only the first retry is marked.
                let first = read_result.is_none();
                if first {
                    read_result = Some(phase::READ_WOULDBLOCK);
                    d.mark(phase::READ_WOULDBLOCK, None);
                    d.mark(
                        phase::RETRY_SLEEP_BEFORE,
                        Some(format!("{}us", cfg.backoff.as_micros())),
                    );
                }
                thread::sleep(cfg.backoff);
                if first {
                    d.mark(phase::RETRY_SLEEP_AFTER, None);
                }
                retries += 1;
                if Instant::now() >= until {
                    d.mark(
                        phase::DEADLINE_HIT,
                        Some(format!(
                            "retries={retries} op_elapsed_us={}",
                            op_start.elapsed().as_micros()
                        )),
                    );
                    break phase::DEADLINE_HIT;
                }
                if retries >= cfg.max_retries {
                    d.mark(
                        phase::RETRY_CAP_HIT,
                        Some(format!(
                            "retries={retries} op_elapsed_us={} op_timeout_us={}",
                            op_start.elapsed().as_micros(),
                            cfg.op_timeout.as_micros()
                        )),
                    );
                    break phase::RETRY_CAP_HIT;
                }
            }
            Err(e) => {
                d.mark(phase::READ_ERR, Some(format!("kind={:?}", e.kind())));
                read_result.get_or_insert(phase::READ_ERR);
                break phase::READ_ERR;
            }
        }
    };

    let verdict = match ended_by {
        phase::READ_DATA | phase::READ_EOF => Verdict::ReadAnswered,
        phase::READ_ERR => Verdict::ReadErrored,
        phase::DEADLINE_HIT => Verdict::BoundedTimeout,
        _ => Verdict::RetryCapHit,
    };
    d.mark(
        phase::DONE,
        Some(format!(
            "verdict={} retries={retries} clock_advanced_us={clock_advanced_us} nonblocking_took_us={nb_took_us}",
            verdict.name()
        )),
    );

    Outcome {
        verdict,
        write_result: Some(write_result),
        read_result,
        retries,
        clock_advanced_us,
        nonblocking_took_us: nb_took_us,
    }
}

#[cfg(test)]
mod tests;
