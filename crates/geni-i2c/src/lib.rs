//! Bounded, polled GENI I2C FIFO transactions for SDM845 i2c12.
use mmio_replay::Mmio;

const TX_LEN: u64 = 0x26c;
const RX_LEN: u64 = 0x270;
const CMD: u64 = 0x600;
const CMD_CTRL: u64 = 0x604;
const IRQ: u64 = 0x610;
const IRQ_CLEAR: u64 = 0x618;
const TX_FIFO: u64 = 0x700;
const RX_FIFO: u64 = 0x780;
const TX_STATUS: u64 = 0x800;
const RX_STATUS: u64 = 0x804;
const DONE: u32 = 1;
// The GENI master error bits are distinct from CMD_DONE and FIFO watermark bits.
const ERROR_BITS: u32 = (1 << 1) | (1 << 2) | (1 << 3) | (1 << 4) | (1 << 5) | (1 << 6) | (1 << 7);
const POLLS: usize = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    InvalidTransfer,
    Timeout,
    Controller(u32),
}

/// A minimal byte-transfer interface for RMI4 and register-model tests.
pub trait Bus {
    fn write(&mut self, addr: u8, data: &[u8]) -> Result<(), Error>;
    fn write_read(&mut self, addr: u8, command: &[u8], data: &mut [u8]) -> Result<(), Error>;
}

pub struct Geni<M: Mmio> {
    io: M,
    base: u64,
}
impl<M: Mmio> Geni<M> {
    pub fn new(io: M, base: u64) -> Self {
        Self { io, base }
    }
    pub fn into_io(self) -> M {
        self.io
    }
    fn read(&mut self, reg: u64) -> u32 {
        self.io.read_u32(self.base + reg)
    }
    fn put(&mut self, reg: u64, value: u32) {
        self.io.write_u32(self.base + reg, value);
    }
    fn cancel(&mut self) {
        self.put(CMD_CTRL, 1 << 2);
    }
    fn wait(&mut self, reg: u64, ready: impl Fn(u32) -> bool) -> Result<u32, Error> {
        for _ in 0..POLLS {
            let irq = self.read(IRQ);
            if irq & ERROR_BITS != 0 {
                self.cancel();
                return Err(Error::Controller(irq));
            }
            let status = if reg == IRQ { irq } else { self.read(reg) };
            if ready(status) {
                return Ok(status);
            }
        }
        self.cancel();
        Err(Error::Timeout)
    }
    fn phase(&mut self, addr: u8, data: &mut [u8], read: bool, hold: bool) -> Result<(), Error> {
        let irq = self.read(IRQ);
        self.put(IRQ_CLEAR, irq);
        self.put(if read { RX_LEN } else { TX_LEN }, data.len() as u32);
        self.put(
            CMD,
            ((if read { 2 } else { 1 }) << 27) | ((addr as u32) << 9) | if hold { 4 } else { 0 },
        );
        for chunk in data.chunks_mut(4) {
            if read {
                self.wait(RX_STATUS, |s| s & 0x3f != 0)?;
                let word = self.read(RX_FIFO).to_le_bytes();
                chunk.copy_from_slice(&word[..chunk.len()]);
            } else {
                self.wait(TX_STATUS, |s| s & 0x1f < 16)?;
                let mut word = [0u8; 4];
                word[..chunk.len()].copy_from_slice(chunk);
                self.put(TX_FIFO, u32::from_le_bytes(word));
            }
        }
        let done = self.wait(IRQ, |s| s & DONE != 0)?;
        self.put(IRQ_CLEAR, done);
        Ok(())
    }
}
impl<M: Mmio> Bus for Geni<M> {
    fn write(&mut self, addr: u8, data: &[u8]) -> Result<(), Error> {
        if addr > 0x7f || data.is_empty() || data.len() > u16::MAX as usize {
            return Err(Error::InvalidTransfer);
        }
        self.phase(addr, &mut data.to_vec(), false, false)
    }
    fn write_read(&mut self, addr: u8, command: &[u8], data: &mut [u8]) -> Result<(), Error> {
        if addr > 0x7f
            || command.is_empty()
            || data.is_empty()
            || command.len() > u16::MAX as usize
            || data.len() > u16::MAX as usize
        {
            return Err(Error::InvalidTransfer);
        }
        self.phase(addr, &mut command.to_vec(), false, true)?;
        self.phase(addr, data, true, false)
    }
}
