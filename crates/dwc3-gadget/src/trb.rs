//! Transfer Request Blocks (16 bytes, little-endian, spec §3).

use mmio_replay::Mmio;

pub const TRB_SIZE: u64 = 16;

pub const HWO: u32 = 1 << 0;
pub const LST: u32 = 1 << 1;
pub const IOC: u32 = 1 << 11;
pub const SPR: u32 = 1 << 26;
pub const BUFSIZ_MASK: u32 = 0x00ff_ffff;

/// `TRBCTL` (control bits 9:4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrbCtl {
    Normal = 1,
    Setup = 2,
    Status2 = 3,
    Status3 = 4,
    ControlData = 5,
}

impl TrbCtl {
    pub fn from_control(control: u32) -> Option<Self> {
        Some(match (control >> 4) & 0x3f {
            1 => Self::Normal,
            2 => Self::Setup,
            3 => Self::Status2,
            4 => Self::Status3,
            5 => Self::ControlData,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Trb {
    pub ptr: u64,
    pub status: u32,
    pub control: u32,
}

impl Trb {
    /// A single, last, interrupt-on-complete TRB handed to the hardware.
    pub fn single(ctl: TrbCtl, ptr: u64, len: u32) -> Self {
        Self {
            ptr,
            status: len & BUFSIZ_MASK,
            control: ((ctl as u32) << 4) | LST | IOC | HWO,
        }
    }

    /// Bytes the hardware did not transfer.
    pub fn remaining(&self) -> u32 {
        self.status & BUFSIZ_MASK
    }

    pub fn hw_owned(&self) -> bool {
        self.control & HWO != 0
    }

    /// Write at `addr`; the control word (with `HWO`) goes last so the
    /// controller never sees a half-written TRB.
    pub fn store(&self, dma: &mut impl Mmio, addr: u64) {
        dma.write_u32(addr, self.ptr as u32);
        dma.write_u32(addr + 4, (self.ptr >> 32) as u32);
        dma.write_u32(addr + 8, self.status);
        dma.write_u32(addr + 12, self.control);
    }

    pub fn load(dma: &mut impl Mmio, addr: u64) -> Self {
        let lo = dma.read_u32(addr) as u64;
        let hi = dma.read_u32(addr + 4) as u64;
        Self {
            ptr: lo | (hi << 32),
            status: dma.read_u32(addr + 8),
            control: dma.read_u32(addr + 12),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mmio_replay::MemModel;

    #[test]
    fn single_trb_bits() {
        let t = Trb::single(TrbCtl::Setup, 0x1_2345_6780, 8);
        assert_eq!(t.control, (2 << 4) | LST | IOC | HWO);
        assert_eq!(t.status, 8);
        assert_eq!(TrbCtl::from_control(t.control), Some(TrbCtl::Setup));
        assert!(t.hw_owned());
    }

    #[test]
    fn store_load_roundtrip_and_control_last() {
        let mut m = MemModel::new();
        let t = Trb::single(TrbCtl::Normal, 0x8000_1000, 512);
        t.store(&mut m, 0x100);
        assert_eq!(Trb::load(&mut m, 0x100), t);
        assert_eq!(m.read_u32(0x10c), t.control);
    }

    #[test]
    fn bufsiz_is_24_bits() {
        let t = Trb::single(TrbCtl::Normal, 0, 0x0100_0010);
        assert_eq!(t.remaining(), 0x10);
        assert_eq!(TrbCtl::from_control(0), None);
    }
}
