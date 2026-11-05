//! Mock SE register model for host tests. Plain array, no MMIO, no unsafe.

use crate::{RegOffset, SeRegs};
use alloc::vec::Vec;

/// A recorded register write: (byte offset, value). Offsets are stored raw so
/// existing transcript assertions keep their plain numeric form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Write(pub u32, pub u32);

/// Mock of one GENI Serial Engine register window. Reads come from a sparse
/// register array (default 0 unless seeded); writes are applied to the array
/// AND recorded in order so tests can assert the exact write sequence.
pub struct MockSe {
    regs: Vec<(u32, u32)>,
    /// Every write performed, in order.
    pub writes: Vec<Write>,
    /// HW-21: total register accesses (writes + fifo reads — the mutating
    /// ones; plain reads go through `&self`) that *reached* the model, i.e.
    /// passed the validated seam. Tests use this to prove malformed offsets
    /// cause ZERO pointer-level access.
    pub accesses: u32,
}

impl MockSe {
    /// Fresh mock, all registers read 0.
    pub fn new() -> Self {
        Self {
            regs: Vec::new(),
            writes: Vec::new(),
            accesses: 0,
        }
    }

    /// Seed a register's read value before the driver touches it.
    pub fn seed(&mut self, off: RegOffset, val: u32) {
        self.seed_raw(off.get(), val);
    }

    /// HW-21 test plumbing: raw-offset seeding for negative tests that must
    /// store values at offsets the validated seam would reject (e.g. an
    /// out-of-window sentinel). Never reachable from driver code.
    pub(crate) fn seed_raw(&mut self, off: u32, val: u32) {
        if let Some(e) = self.regs.iter_mut().find(|(o, _)| *o == off) {
            e.1 = val;
        } else {
            self.regs.push((off, val));
        }
    }

    /// Current stored value of a register (post-driver writes included).
    pub fn get(&self, off: u32) -> u32 {
        self.regs
            .iter()
            .find(|(o, _)| *o == off)
            .map_or(0, |(_, v)| *v)
    }

    /// All recorded writes to a given offset, in order.
    pub fn writes_to(&self, off: u32) -> Vec<u32> {
        self.writes
            .iter()
            .filter(|w| w.0 == off)
            .map(|w| w.1)
            .collect()
    }
}

impl Default for MockSe {
    fn default() -> Self {
        Self::new()
    }
}

impl SeRegs for MockSe {
    fn read(&self, off: RegOffset) -> u32 {
        self.get(off.get())
    }
    fn write(&mut self, off: RegOffset, val: u32) {
        self.accesses += 1;
        self.seed_raw(off.get(), val);
        self.writes.push(Write(off.get(), val));
    }
}

/// A mock that fails the `n`-th TX-FIFO push (1-based) — HW-23 plumbing for
/// mid-string error propagation. Every other register behaves like `MockSe`
/// with watermark/done pre-seeded so each putc cycle succeeds up to the Nth.
pub struct FailAtByte {
    inner: MockSe,
    n: u32,
    pushes: u32,
}

impl FailAtByte {
    pub fn new(n: u32) -> Self {
        let mut inner = MockSe::new();
        // Watermark + done pre-set so every putc cycle passes its polls.
        inner.seed_raw(
            crate::SE_GENI_M_IRQ_STATUS.get(),
            crate::M_TX_FIFO_WATERMARK | crate::M_CMD_DONE,
        );
        Self {
            inner,
            n,
            pushes: 0,
        }
    }
    /// Bytes that actually reached the TX FIFO data port before the failure.
    pub fn fifo_bytes(&self) -> Vec<u8> {
        self.inner
            .writes_to(crate::SE_GENI_TX_FIFOn.get())
            .iter()
            .map(|&v| v as u8)
            .collect()
    }
}

impl SeRegs for FailAtByte {
    fn read(&self, off: RegOffset) -> u32 {
        self.inner.read(off)
    }
    fn write(&mut self, off: RegOffset, val: u32) {
        if off == crate::SE_GENI_TX_FIFOn {
            self.pushes += 1;
            if self.pushes >= self.n {
                // The Nth push "fails": the FIFO never accepts it, so the
                // watermark bit goes clear and the done bit is not re-armed.
                self.inner.seed_raw(crate::SE_GENI_M_IRQ_STATUS.get(), 0);
                return;
            }
        }
        self.inner.write(off, val);
    }
}

/// A mock whose master IRQ status auto-advances like a healthy SE:
/// - once UART_START_TX is issued (write to M_CMD0 with the opcode), the
///   TX-watermark bit becomes set (FIFO has room — depth 2 > watermark cycle);
/// - once a byte lands in the TX FIFO, the CMD_DONE bit also becomes set;
/// - HW-19: once UART_START_READ is armed on the secondary sequencer (write
///   to S_CMD0 with opcode 1 — oracle getc `c:408`), the `M_SEC_IRQ` summary
///   bit becomes set (getc polls it at `c:410–411`). RX FIFO contents are
///   modelled separately via `seed_rx`/`rx_fifo`: reads of `SE_GENI_RX_FIFOn`
///   pop the oldest seeded word and decrement the `RX_FIFO_STATUS` word count,
///   so bytes come out in order and the FIFO drains like real hardware.
pub struct AutoSe {
    inner: MockSe,
    /// Seeded RX fifo words, oldest first (popped by reads of RX_FIFOn).
    rx_fifo: Vec<u32>,
}

impl AutoSe {
    pub fn new() -> Self {
        Self {
            inner: MockSe::new(),
            rx_fifo: Vec::new(),
        }
    }
    pub fn get(&self, off: u32) -> u32 {
        self.inner.get(off)
    }
    /// Snapshot of the recorded write transcript.
    pub fn inner_writes(&self) -> Vec<Write> {
        self.inner.writes.clone()
    }
    /// HW-21 test plumbing: recorded writes to a raw offset through AutoSe.
    pub(crate) fn writes_to_raw(&self, off: u32) -> Vec<u32> {
        self.inner.writes_to(off)
    }
    /// HW-21: mutating accesses (writes + fifo reads) that reached the model.
    pub fn accesses(&self) -> u32 {
        self.inner.accesses
    }
    /// HW-21 test plumbing: raw-offset seeding through AutoSe (see
    /// `MockSe::seed_raw`) so negative tests can place sentinels where the
    /// validated seam would never let the driver look.
    pub(crate) fn seed_raw_at(&mut self, off: u32, val: u32) {
        self.inner.seed_raw(off, val);
    }
    /// Seed one RX fifo word (low byte = character) and bump the modelled
    /// `SE_GENI_RX_FIFO_STATUS` word count (`RX_FIFO_WC`, h:224).
    pub fn seed_rx(&mut self, word: u32) {
        self.rx_fifo.push(word);
        let wc = self.inner.get(crate::SE_GENI_RX_FIFO_STATUS.get()) & crate::RX_FIFO_WC;
        self.inner
            .seed_raw(crate::SE_GENI_RX_FIFO_STATUS.get(), wc + 1);
    }
}

impl Default for AutoSe {
    fn default() -> Self {
        Self::new()
    }
}

impl SeRegs for AutoSe {
    fn read(&self, off: RegOffset) -> u32 {
        self.inner.read(off)
    }
    fn read_fifo(&mut self, off: RegOffset) -> u32 {
        self.inner.accesses += 1;
        // A FIFO *read* pops the oldest seeded word (in-order delivery) and
        // decrements the modelled word count. Mocks get &mut in read_fifo so
        // popping is legal here.
        if off != crate::SE_GENI_RX_FIFOn {
            return self.inner.get(off.get());
        }
        let word = if self.rx_fifo.is_empty() {
            0
        } else {
            self.rx_fifo.remove(0)
        };
        let wc = self.inner.get(crate::SE_GENI_RX_FIFO_STATUS.get()) & crate::RX_FIFO_WC;
        if wc > 0 {
            self.inner
                .seed_raw(crate::SE_GENI_RX_FIFO_STATUS.get(), wc - 1);
        }
        word
    }
    fn write(&mut self, off: RegOffset, val: u32) {
        self.inner.write(off, val);
        if off == crate::SE_GENI_M_CMD0 && (val >> 27) == crate::UART_START_TX_OPCODE {
            // Sequencer started: FIFO below watermark -> room available.
            let s = self.inner.get(crate::SE_GENI_M_IRQ_STATUS.get());
            self.inner.seed_raw(
                crate::SE_GENI_M_IRQ_STATUS.get(),
                s | crate::M_TX_FIFO_WATERMARK,
            );
        }
        if off == crate::SE_GENI_TX_FIFOn {
            // Byte queued with length 1: command completes.
            let s = self.inner.get(crate::SE_GENI_M_IRQ_STATUS.get());
            self.inner
                .seed_raw(crate::SE_GENI_M_IRQ_STATUS.get(), s | crate::M_CMD_DONE);
        }
        if off == crate::SE_GENI_S_CMD0 && (val >> 27) == crate::UART_START_READ_OPCODE {
            // RX read armed: secondary summary IRQ fires (getc poll c:410).
            let s = self.inner.get(crate::SE_GENI_M_IRQ_STATUS.get());
            self.inner
                .seed_raw(crate::SE_GENI_M_IRQ_STATUS.get(), s | crate::M_SEC_IRQ);
        }
    }
}
