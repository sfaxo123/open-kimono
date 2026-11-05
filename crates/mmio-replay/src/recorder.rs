//! Recorder: wraps any [`Mmio`] device and captures every transaction as a
//! [`Trace`].
//!
//! On the phone this wraps the real register window (via the oracle tooling);
//! in host tests it wraps a [`MemModel`] so a reference driver run can be
//! captured and frozen as JSONL.

use crate::mmio::{Mmio, Op, Width};
use crate::trace::{Event, Trace};

/// Wraps a device and records each MMIO transaction in order.
pub struct Recorder<'a, T: Mmio> {
    inner: &'a mut T,
    trace: Trace,
    delay_us: u64,
}

impl<'a, T: Mmio> Recorder<'a, T> {
    /// Start recording accesses to `inner`.
    pub fn new(inner: &'a mut T) -> Self {
        Self {
            inner,
            trace: Trace::new(),
            delay_us: 0,
        }
    }

    /// Set the delay (µs since the previous event) attributed to the *next*
    /// transaction. Resets to 0 after each recorded event.
    pub fn set_delay_us(&mut self, delay_us: u64) {
        self.delay_us = delay_us;
    }

    /// The trace recorded so far.
    pub fn trace(&self) -> &Trace {
        &self.trace
    }

    /// Finish recording and return the trace.
    pub fn into_trace(self) -> Trace {
        self.trace
    }
}

impl<T: Mmio> Mmio for Recorder<'_, T> {
    fn read(&mut self, addr: u64, width: Width) -> u64 {
        let value = self.inner.read(addr, width);
        self.trace.push(Event {
            op: Op::Read,
            width,
            addr,
            value,
            delay_us: self.delay_us,
        });
        self.delay_us = 0;
        value
    }

    fn write(&mut self, addr: u64, width: Width, value: u64) {
        self.inner.write(addr, width, value);
        self.trace.push(Event {
            op: Op::Write,
            width,
            addr,
            value: value & width.mask(),
            delay_us: self.delay_us,
        });
        self.delay_us = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mmio::MemModel;

    #[test]
    fn records_reads_and_writes_in_order() {
        let mut model = MemModel::new();
        model.load(0x40, &[0x78, 0x56, 0x34, 0x12]);
        let mut rec = Recorder::new(&mut model);
        rec.write_u32(0x10, 0xdead_beef);
        assert_eq!(
            rec.read_u32(0x40),
            0x1234_5678,
            "device value must pass through"
        );
        rec.write_u8(0x10, 0xff);
        let trace = rec.into_trace();
        assert_eq!(trace.events.len(), 3);
        assert_eq!(trace.events[0], Event::write(0x10, Width::W32, 0xdead_beef));
        assert_eq!(trace.events[1], Event::read(0x40, Width::W32, 0x1234_5678));
        assert_eq!(trace.events[2], Event::write(0x10, Width::W8, 0xff));
        // The wrapped device really was written through the recorder.
        // write_u8 replaced only the low byte of the earlier u32 write.
        assert_eq!(model.read_u32(0x10), 0xdead_beef & 0xffff_ff00 | 0xff);
    }

    #[test]
    fn recorded_writes_are_masked_to_width() {
        let mut model = MemModel::new();
        let mut rec = Recorder::new(&mut model);
        rec.write(0x100, Width::W16, 0xffff_ffff);
        let t = rec.into_trace();
        assert_eq!(t.events[0].value, 0xffff);
    }

    #[test]
    fn delays_attach_to_next_event_and_reset() {
        let mut model = MemModel::new();
        let mut rec = Recorder::new(&mut model);
        rec.set_delay_us(1500);
        rec.write_u32(0x0, 1);
        rec.write_u32(0x0, 2); // no delay set: 0
        rec.set_delay_us(7);
        let _ = rec.read_u32(0x0);
        let t = rec.into_trace();
        assert_eq!(t.total_delay_us(), 1507);
        assert_eq!(t.events[0].delay_us, 1500);
        assert_eq!(t.events[1].delay_us, 0);
        assert_eq!(t.events[2].delay_us, 7);
    }

    #[test]
    fn recorded_trace_roundtrips_through_jsonl() {
        let mut model = MemModel::new();
        let mut rec = Recorder::new(&mut model);
        rec.write_u32(0xa84010, 2);
        let _ = rec.read_u32(0xa84018);
        let trace = rec.into_trace();
        let text = trace.to_jsonl();
        assert_eq!(Trace::from_jsonl(&text).unwrap(), trace);
    }
}
