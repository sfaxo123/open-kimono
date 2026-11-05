//! The MMIO access trait and a sparse byte-memory model.
//!
//! Drivers never touch `volatile` pointers directly: they take a `&mut impl
//! Mmio` (or `&mut dyn Mmio`). In production that is backed by the real
//! register window; in host tests it is backed by [`MemModel`] (a plain
//! sparse memory), a [`crate::Recorder`] around one, or a
//! [`crate::Replayer`] replaying a trace recorded on the phone.

use alloc::collections::BTreeMap;

/// Access width of one MMIO transaction.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Width {
    /// 8-bit access.
    W8,
    /// 16-bit access.
    W16,
    /// 32-bit access (the common Qualcomm register width).
    W32,
    /// 64-bit access.
    W64,
}

impl Width {
    /// Width in bits.
    pub fn bits(self) -> u32 {
        match self {
            Width::W8 => 8,
            Width::W16 => 16,
            Width::W32 => 32,
            Width::W64 => 64,
        }
    }

    /// Width in bytes.
    pub fn bytes(self) -> usize {
        (self.bits() / 8) as usize
    }

    /// Decode from a bit count (8, 16, 32 or 64).
    pub fn from_bits(bits: u32) -> Option<Width> {
        match bits {
            8 => Some(Width::W8),
            16 => Some(Width::W16),
            32 => Some(Width::W32),
            64 => Some(Width::W64),
            _ => None,
        }
    }

    /// Bitmask of the low `bits()` bits, e.g. `W16 -> 0xffff`.
    pub fn mask(self) -> u64 {
        match self {
            Width::W64 => u64::MAX,
            w => (1u64 << w.bits()) - 1,
        }
    }
}

/// Direction of one MMIO transaction.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Op {
    /// Read transaction.
    Read,
    /// Write transaction.
    Write,
}

/// A memory-mapped register window a driver can touch.
///
/// All values are transferred little-endian; `write` must ignore the bits of
/// `value` above `width`. Implementations must not reorder or coalesce
/// transactions — trace replay depends on one event per call.
pub trait Mmio {
    /// Read `width` bits at `addr`.
    fn read(&mut self, addr: u64, width: Width) -> u64;

    /// Write the low `width` bits of `value` at `addr`.
    fn write(&mut self, addr: u64, width: Width, value: u64);

    /// `W8` read.
    fn read_u8(&mut self, addr: u64) -> u8 {
        self.read(addr, Width::W8) as u8
    }

    /// `W16` read.
    fn read_u16(&mut self, addr: u64) -> u16 {
        self.read(addr, Width::W16) as u16
    }

    /// `W32` read.
    fn read_u32(&mut self, addr: u64) -> u32 {
        self.read(addr, Width::W32) as u32
    }

    /// `W64` read.
    fn read_u64(&mut self, addr: u64) -> u64 {
        self.read(addr, Width::W64)
    }

    /// `W8` write.
    fn write_u8(&mut self, addr: u64, value: u8) {
        self.write(addr, Width::W8, value as u64)
    }

    /// `W16` write.
    fn write_u16(&mut self, addr: u64, value: u16) {
        self.write(addr, Width::W16, value as u64)
    }

    /// `W32` write.
    fn write_u32(&mut self, addr: u64, value: u32) {
        self.write(addr, Width::W32, value as u64)
    }

    /// `W64` write.
    fn write_u64(&mut self, addr: u64, value: u64) {
        self.write(addr, Width::W64, value)
    }
}

/// A sparse little-endian byte memory: a simple device model for host tests
/// (pre-load reset values with [`MemModel::load`], then let the driver run).
///
/// Unwritten addresses read as zero. No alignment or side-effect modelling —
/// drivers that need register semantics layer them on top of this.
#[derive(Clone, Debug, Default)]
pub struct MemModel {
    bytes: BTreeMap<u64, u8>,
}

impl MemModel {
    /// An empty model (everything reads zero).
    pub fn new() -> Self {
        Self::default()
    }

    /// Pre-load raw bytes (device-tree style setup of reset/firmware state).
    pub fn load(&mut self, addr: u64, data: &[u8]) {
        for (i, b) in data.iter().enumerate() {
            self.bytes.insert(addr + i as u64, *b);
        }
    }

    /// The stored byte at `addr`, or `None` if never written or loaded.
    pub fn peek(&self, addr: u64) -> Option<u8> {
        self.bytes.get(&addr).copied()
    }
}

impl Mmio for MemModel {
    fn read(&mut self, addr: u64, width: Width) -> u64 {
        let mut v = 0u64;
        for i in 0..width.bytes() {
            let b = self.bytes.get(&(addr + i as u64)).copied().unwrap_or(0);
            v |= (b as u64) << (8 * i);
        }
        v
    }

    fn write(&mut self, addr: u64, width: Width, value: u64) {
        let value = value & width.mask();
        for i in 0..width.bytes() {
            self.bytes
                .insert(addr + i as u64, ((value >> (8 * i)) & 0xff) as u8);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn width_helpers() {
        assert_eq!(Width::W32.bits(), 32);
        assert_eq!(Width::W16.bytes(), 2);
        assert_eq!(Width::from_bits(64), Some(Width::W64));
        assert_eq!(Width::from_bits(12), None);
        assert_eq!(Width::W8.mask(), 0xff);
        assert_eq!(Width::W16.mask(), 0xffff);
        assert_eq!(Width::W32.mask(), 0xffff_ffff);
        assert_eq!(Width::W64.mask(), u64::MAX);
    }

    #[test]
    fn u32_roundtrip_little_endian() {
        let mut m = MemModel::new();
        m.write_u32(0xa84010, 0xdead_beef);
        assert_eq!(m.peek(0xa84010), Some(0xef));
        assert_eq!(m.peek(0xa84011), Some(0xbe));
        assert_eq!(m.peek(0xa84012), Some(0xad));
        assert_eq!(m.peek(0xa84013), Some(0xde));
        assert_eq!(m.read_u32(0xa84010), 0xdead_beef);
    }

    #[test]
    fn narrow_write_does_not_clobber_neighbours() {
        let mut m = MemModel::new();
        m.write_u32(0x100, 0x1122_3344);
        m.write_u8(0x101, 0xff);
        assert_eq!(m.read_u32(0x100), 0x1122_ff44);
        m.write_u16(0x102, 0xabcd);
        assert_eq!(m.read_u32(0x100), 0xabcd_ff44);
    }

    #[test]
    fn write_masks_value_to_width() {
        let mut m = MemModel::new();
        m.write(0x200, Width::W8, 0x1ff);
        assert_eq!(m.peek(0x200), Some(0xff));
        assert_eq!(m.peek(0x201), None);
        m.write(0x210, Width::W16, 0x1_2345);
        assert_eq!(m.read_u16(0x210), 0x2345);
        assert_eq!(m.peek(0x212), None);
    }

    #[test]
    fn unwritten_reads_zero() {
        let mut m = MemModel::new();
        assert_eq!(m.read_u64(0x8000), 0);
        assert_eq!(m.read_u8(0), 0);
    }

    #[test]
    fn u64_spans_loaded_and_unwritten_bytes() {
        let mut m = MemModel::new();
        m.load(0x40, &[0x01, 0x02]);
        assert_eq!(m.read_u64(0x40), 0x0000_0000_0000_0201);
    }

    #[test]
    fn load_preloads_reset_state() {
        let mut m = MemModel::new();
        // Pretend firmware left SE_HW_PARAM_0 = 0x0000_0010 at +0xe24.
        m.load(0xa84e24, &[0x10, 0x00, 0x00, 0x00]);
        assert_eq!(m.read_u32(0xa84e24), 0x10);
    }
}
