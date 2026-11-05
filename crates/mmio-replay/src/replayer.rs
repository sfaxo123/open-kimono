//! Replayer: feeds a recorded [`Trace`] back to a driver and compares every
//! transaction against it.
//!
//! The driver under test is handed `&mut Replayer` as its [`Mmio`]:
//!
//! - reads return the value the real hardware returned at that point in the
//!   recording, so state machines (FIFO-ready bits, handshakes) replay
//!   faithfully;
//! - writes are compared against the recording.
//!
//! ## Compare modes
//!
//! [`Compare::Strict`] — every transaction must equal the next recorded
//! event (op, width, address, and written value). The first deviation fails
//! the replay; later accesses are counted but not compared.
//!
//! [`Compare::Lenient`] — structure must match: a driver may write values
//! that differ from the recording (recorded as [`MismatchKind::WriteValue`]
//! warnings) and may skip recorded events (e.g. extra polling reads the
//! recording contains), but touching a register that is not in the trace at
//! all is a [`MismatchKind::Structure`] failure.
//!
//! Call [`Replayer::finish`] at the end of the driver run and check
//! [`ReplayReport::ok`].

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::mmio::{Mmio, Op, Width};
use crate::trace::{Event, Trace};

/// How strictly observed accesses are compared against the trace.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Compare {
    /// Everything must match exactly; first deviation fails.
    Strict,
    /// Values may differ and recorded events may be skipped; unknown
    /// registers or directions fail.
    Lenient,
}

/// An access the driver actually performed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ObservedAccess {
    /// Direction the driver used.
    pub op: Op,
    /// Width the driver used.
    pub width: Width,
    /// Address the driver touched.
    pub addr: u64,
    /// Value the driver wrote (0 for reads).
    pub value: u64,
}

/// Why an observed access did not match the recording.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MismatchKind {
    /// Wrong op/width/address: the access does not correspond to the next
    /// (or any remaining) recorded event. Fails in both compare modes.
    Structure,
    /// Right register, wrong value. Fails in strict mode only.
    WriteValue,
}

/// One comparison failure.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Mismatch {
    /// Index in the trace of the event that was expected (`None` when the
    /// driver ran past the end of the trace).
    pub index: Option<usize>,
    /// The recorded event that was expected, if any.
    pub expected: Option<Event>,
    /// What the driver actually did.
    pub got: ObservedAccess,
    /// Which rule was broken.
    pub kind: MismatchKind,
}

/// The outcome of a replay, produced by [`Replayer::finish`].
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ReplayReport {
    /// The compare mode that was used.
    pub mode: Compare,
    /// Observed accesses that matched a recorded event.
    pub matched: usize,
    /// All comparison failures, in order.
    pub mismatches: Vec<Mismatch>,
    /// Recorded events the lenient replay stepped over (extra accesses in
    /// the recording the driver did not perform).
    pub skipped: Vec<Event>,
    /// Recorded events never reached by the driver.
    pub unconsumed: Vec<Event>,
    /// Accesses made after a strict replay already failed (not compared).
    pub uncompared: usize,
}

impl ReplayReport {
    /// Whether the replay counts as a pass under its compare mode.
    pub fn ok(&self) -> bool {
        match self.mode {
            Compare::Strict => {
                self.mismatches.is_empty() && self.unconsumed.is_empty() && self.uncompared == 0
            }
            Compare::Lenient => !self
                .mismatches
                .iter()
                .any(|m| m.kind == MismatchKind::Structure),
        }
    }

    /// Non-failing deviations worth printing in a test log.
    pub fn warnings(&self) -> usize {
        self.mismatches
            .iter()
            .filter(|m| m.kind == MismatchKind::WriteValue)
            .count()
            + self.skipped.len()
            + self.unconsumed.len()
    }
}

impl core::fmt::Display for ReplayReport {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "{:?} replay: {} matched, {} mismatch(es), {} skipped, {} unconsumed, {} uncompared -> {}",
            self.mode,
            self.matched,
            self.mismatches.len(),
            self.skipped.len(),
            self.unconsumed.len(),
            self.uncompared,
            if self.ok() { "PASS" } else { "FAIL" },
        )?;
        for m in &self.mismatches {
            let exp = match m.expected {
                Some(e) => format!(
                    "expected {:?} {:#x} w{} {:#x}",
                    e.op,
                    e.addr,
                    e.width.bits(),
                    e.value
                ),
                None => String::from("expected end of trace"),
            };
            write!(
                f,
                "\n  [{:?} @{}] {}: {:?} {:#x} w{} {:#x}",
                m.kind,
                match m.index {
                    Some(i) => alloc::string::ToString::to_string(&i),
                    None => String::from("end"),
                },
                exp,
                m.got.op,
                m.got.addr,
                m.got.width.bits(),
                m.got.value,
            )?;
        }
        Ok(())
    }
}

/// Replays a recorded [`Trace`] against a driver and compares accesses.
pub struct Replayer {
    trace: Vec<Event>,
    next: usize,
    mode: Compare,
    matched: usize,
    mismatches: Vec<Mismatch>,
    skipped: Vec<Event>,
    uncompared: usize,
    failed: bool,
    read_default: u64,
}

impl Replayer {
    /// Replay `trace` under the given compare mode.
    pub fn new(trace: Trace, mode: Compare) -> Self {
        Self {
            trace: trace.events,
            next: 0,
            mode,
            matched: 0,
            mismatches: Vec::new(),
            skipped: Vec::new(),
            uncompared: 0,
            failed: false,
            read_default: 0,
        }
    }

    /// [`Compare::Strict`] replay.
    pub fn strict(trace: Trace) -> Self {
        Self::new(trace, Compare::Strict)
    }

    /// [`Compare::Lenient`] replay.
    pub fn lenient(trace: Trace) -> Self {
        Self::new(trace, Compare::Lenient)
    }

    /// Value returned for reads that have no recorded counterpart (default 0).
    pub fn with_read_default(mut self, value: u64) -> Self {
        self.read_default = value;
        self
    }

    /// Number of recorded events consumed so far.
    pub fn consumed(&self) -> usize {
        self.next
    }

    /// Number of recorded events not yet consumed.
    pub fn remaining(&self) -> usize {
        self.trace.len() - self.next
    }

    /// Whether the replay has already failed (strict mode stops comparing).
    pub fn failed(&self) -> bool {
        self.failed
    }

    /// End the run and produce the report.
    pub fn finish(&mut self) -> ReplayReport {
        ReplayReport {
            mode: self.mode,
            matched: self.matched,
            mismatches: core::mem::take(&mut self.mismatches),
            skipped: core::mem::take(&mut self.skipped),
            unconsumed: self.trace[self.next..].to_vec(),
            uncompared: core::mem::replace(&mut self.uncompared, 0),
        }
    }

    fn check(&mut self, op: Op, width: Width, addr: u64, value: u64) -> Option<u64> {
        let value = value & width.mask();
        if self.failed {
            self.uncompared += 1;
            return (op == Op::Read).then_some(self.read_default);
        }
        let got = ObservedAccess {
            op,
            width,
            addr,
            value,
        };

        // Fast path: the next recorded event.
        if let Some(e) = self.trace.get(self.next) {
            if e.op == op && e.width == width && e.addr == addr {
                if op == Op::Write && value != e.value {
                    self.mismatches.push(Mismatch {
                        index: Some(self.next),
                        expected: Some(*e),
                        got,
                        kind: MismatchKind::WriteValue,
                    });
                    if self.mode == Compare::Strict {
                        self.failed = true;
                        return None;
                    }
                }
                self.matched += 1;
                self.next += 1;
                return (op == Op::Read).then_some(e.value);
            }
        }

        if self.mode == Compare::Lenient {
            // Look ahead: the driver may be skipping recorded events.
            if let Some(j) = (self.next..self.trace.len()).find(|&j| {
                self.trace[j].op == op && self.trace[j].width == width && self.trace[j].addr == addr
            }) {
                self.skipped
                    .extend(self.trace[self.next..j].iter().copied());
                let e = self.trace[j];
                if op == Op::Write && value != e.value {
                    self.mismatches.push(Mismatch {
                        index: Some(j),
                        expected: Some(e),
                        got,
                        kind: MismatchKind::WriteValue,
                    });
                }
                self.matched += 1;
                self.next = j + 1;
                return (op == Op::Read).then_some(e.value);
            }
        }

        // No counterpart anywhere (lenient) or not the next event (strict).
        let expected = self.trace.get(self.next).copied();
        self.mismatches.push(Mismatch {
            index: Some(self.next),
            expected,
            got,
            kind: MismatchKind::Structure,
        });
        if self.mode == Compare::Strict {
            self.failed = true;
        }
        (op == Op::Read).then_some(self.read_default)
    }
}

impl Mmio for Replayer {
    fn read(&mut self, addr: u64, width: Width) -> u64 {
        self.check(Op::Read, width, addr, 0)
            .unwrap_or(self.read_default)
    }

    fn write(&mut self, addr: u64, width: Width, value: u64) {
        self.check(Op::Write, width, addr, value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trace_of(spec: &[(&str, u64, u64)]) -> Trace {
        let mut t = Trace::new();
        for (line, addr, value) in spec {
            let (op, width) = line.split_once('@').unwrap();
            let width = Width::from_bits(width.parse().unwrap()).unwrap();
            let e = if op == "w" {
                Event::write(*addr, width, *value)
            } else {
                Event::read(*addr, width, *value)
            };
            t.push(e);
        }
        t
    }

    #[test]
    fn strict_exact_pass() {
        let t = trace_of(&[
            ("w@32", 0x10, 1),
            ("r@32", 0x18, 0x4000_0000),
            ("w@32", 0x10, 0),
        ]);
        let mut rep = Replayer::strict(t.clone());
        rep.write_u32(0x10, 1);
        assert_eq!(
            rep.read_u32(0x18),
            0x4000_0000,
            "reads return recorded values"
        );
        rep.write_u32(0x10, 0);
        let report = rep.finish();
        assert!(report.ok(), "{report}");
        assert_eq!(report.matched, 3);
        assert_eq!(rep.remaining(), 0);
    }

    #[test]
    fn strict_write_value_mismatch_fails_at_right_index() {
        let t = trace_of(&[("w@32", 0x10, 1), ("w@32", 0x14, 7)]);
        let mut rep = Replayer::strict(t);
        rep.write_u32(0x10, 1);
        rep.write_u32(0x14, 8); // wrong value
        assert!(rep.failed());
        let r = rep.finish();
        assert!(!r.ok());
        assert_eq!(r.mismatches.len(), 1);
        assert_eq!(r.mismatches[0].kind, MismatchKind::WriteValue);
        assert_eq!(r.mismatches[0].index, Some(1));
        assert_eq!(r.mismatches[0].expected.unwrap().value, 7);
        assert_eq!(r.mismatches[0].got.value, 8);
    }

    #[test]
    fn strict_structure_mismatch_stops_comparing() {
        let t = trace_of(&[("w@32", 0x10, 1), ("w@32", 0x14, 7)]);
        let mut rep = Replayer::strict(t);
        rep.write_u32(0x99, 1); // wrong register
        rep.write_u32(0x10, 1); // would have matched, but strict already failed
        let r = rep.finish();
        assert!(!r.ok());
        assert_eq!(r.mismatches.len(), 1);
        assert_eq!(r.mismatches[0].kind, MismatchKind::Structure);
        assert_eq!(r.uncompared, 1);
        assert_eq!(r.unconsumed.len(), 2);
    }

    #[test]
    fn strict_access_past_end_of_trace() {
        let t = trace_of(&[("w@32", 0x10, 1)]);
        let mut rep = Replayer::strict(t);
        rep.write_u32(0x10, 1);
        rep.write_u32(0x10, 1); // extra
        let r = rep.finish();
        assert!(!r.ok());
        assert_eq!(r.mismatches[0].kind, MismatchKind::Structure);
        assert_eq!(r.mismatches[0].index, Some(1));
        assert_eq!(r.mismatches[0].expected, None);
    }

    #[test]
    fn strict_driver_doing_fewer_accesses_fails() {
        let t = trace_of(&[("w@32", 0x10, 1), ("w@32", 0x14, 7)]);
        let mut rep = Replayer::strict(t);
        rep.write_u32(0x10, 1);
        let r = rep.finish();
        assert!(!r.ok());
        assert_eq!(r.unconsumed.len(), 1);
    }

    #[test]
    fn lenient_tolerates_write_value_differences() {
        let t = trace_of(&[("w@32", 0x10, 1), ("w@32", 0x14, 7)]);
        let mut rep = Replayer::lenient(t);
        rep.write_u32(0x10, 2);
        rep.write_u32(0x14, 9);
        let r = rep.finish();
        assert!(r.ok(), "{r}");
        assert_eq!(r.matched, 2);
        assert_eq!(r.mismatches.len(), 2);
        assert!(r
            .mismatches
            .iter()
            .all(|m| m.kind == MismatchKind::WriteValue));
        assert_eq!(r.warnings(), 2);
    }

    #[test]
    fn lenient_tolerates_skipped_recorded_events() {
        // Recording polled twice; driver polls once.
        let t = trace_of(&[
            ("r@32", 0x18, 0x1),
            ("r@32", 0x18, 0x1),
            ("w@32", 0x1c, 0x1),
        ]);
        let mut rep = Replayer::lenient(t);
        assert_eq!(rep.read_u32(0x18), 0x1);
        rep.write_u32(0x1c, 0x1); // lookahead over the second poll
        let r = rep.finish();
        assert!(r.ok(), "{r}");
        assert_eq!(r.skipped.len(), 1);
        assert_eq!(r.matched, 2);
    }

    #[test]
    fn lenient_fails_on_unknown_register() {
        let t = trace_of(&[("w@32", 0x10, 1)]);
        let mut rep = Replayer::lenient(t);
        rep.write_u32(0x20, 1); // not in the trace at all
        let r = rep.finish();
        assert!(!r.ok());
        assert_eq!(r.mismatches[0].kind, MismatchKind::Structure);
    }

    #[test]
    fn lenient_fails_on_wrong_direction() {
        let t = trace_of(&[("w@32", 0x10, 1)]);
        let mut rep = Replayer::lenient(t);
        assert_eq!(rep.read_u32(0x10), 0); // reading a write-only slot
        let r = rep.finish();
        assert!(!r.ok());
        assert_eq!(r.mismatches[0].kind, MismatchKind::Structure);
        assert_eq!(r.unconsumed.len(), 1);
    }

    #[test]
    fn lenient_fails_on_wrong_width() {
        let t = trace_of(&[("w@32", 0x10, 1)]);
        let mut rep = Replayer::lenient(t);
        rep.write_u16(0x10, 1);
        let r = rep.finish();
        assert!(!r.ok());
        assert_eq!(r.mismatches[0].kind, MismatchKind::Structure);
    }

    #[test]
    fn read_default_used_for_unmatched_reads() {
        let t = trace_of(&[("w@32", 0x10, 1)]);
        let mut rep = Replayer::lenient(t).with_read_default(0xdead_beef);
        assert_eq!(rep.read_u32(0x99), 0xdead_beef);
    }

    #[test]
    fn display_report_is_informative() {
        let t = trace_of(&[("w@32", 0x10, 1)]);
        let mut rep = Replayer::strict(t);
        rep.write_u32(0x20, 5);
        let text = alloc::format!("{}", rep.finish());
        assert!(text.contains("FAIL"));
        assert!(text.contains("Structure"));
        assert!(text.contains("0x20"));
    }
}
