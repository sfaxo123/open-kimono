//! Host-testable MMIO window dispatch for `usbgadgetd` (V1 compile spike).
//!
//! Implements the two-window contract from
//! `docs/workbench/USBGADGETD-WIRING.md` section 3 against a
//! [`MemModel`] backing store, so the dispatch, bounds and sticky-fault
//! semantics are proven on the host before any Redox physmap glue lands:
//!
//! - `R` (registers): absolute addresses; only the DWC3 core window
//!   (`0x0a60_0000`, `0xcd00` bytes) and the QSCRATCH window
//!   (`0x0a6f_8800`, `0x400` bytes) are valid. Anything else reads as 0,
//!   drops the write, and sets a sticky fault flag (never panics across
//!   the trait boundary).
//! - `D` (DMA): bus addresses; valid range is exactly
//!   `bus .. bus + DMA_SIZE`. Same fault semantics.
//!
//! The Redox pointer-backed impls live in `redox_mmio.rs`
//! (`cfg(target_os = "redox")`) and mirror these exact semantics over
//! volatile accesses.

use dwc3_gadget::layout::DMA_SIZE;
use mmio_replay::{MemModel, Mmio, Width};

/// DWC3 core window base (`snps,dwc3` on SDM845 `usb_1`).
pub const CORE_BASE: u64 = 0x0a60_0000;
/// DWC3 core window length (per wiring design section 3).
pub const CORE_LEN: u64 = 0xcd00;
/// QUSB2/QSCRATCH glue window base.
pub const QSCRATCH_BASE: u64 = 0x0a6f_8800;
/// QSCRATCH window length (per wiring design section 3).
pub const QSCRATCH_LEN: u64 = 0x400;

fn in_window(addr: u64, width: Width, base: u64, len: u64) -> bool {
    let end = match addr.checked_add(width.bytes() as u64) {
        Some(e) => e,
        None => return false,
    };
    addr >= base && end <= base.saturating_add(len)
}

/// Register window `R`: dispatch over the core + QSCRATCH windows.
#[derive(Debug, Default)]
pub struct RegsMmio {
    backing: MemModel,
    fault: bool,
}

impl RegsMmio {
    pub fn new() -> Self {
        Self::default()
    }

    /// Sticky fault: set by any out-of-window access, never auto-cleared.
    pub fn faulted(&self) -> bool {
        self.fault
    }

    fn valid(&self, addr: u64, width: Width) -> bool {
        in_window(addr, width, CORE_BASE, CORE_LEN)
            || in_window(addr, width, QSCRATCH_BASE, QSCRATCH_LEN)
    }
}

impl Mmio for RegsMmio {
    fn read(&mut self, addr: u64, width: Width) -> u64 {
        if self.valid(addr, width) {
            self.backing.read(addr, width)
        } else {
            self.fault = true;
            0
        }
    }

    fn write(&mut self, addr: u64, width: Width, value: u64) {
        if self.valid(addr, width) {
            self.backing.write(addr, width, value);
        } else {
            self.fault = true;
        }
    }
}

/// DMA window `D`: the single contiguous region, addressed by bus address.
#[derive(Debug)]
pub struct DmaMmio {
    backing: MemModel,
    bus: u64,
    fault: bool,
}

impl DmaMmio {
    /// `bus` must be 4 KiB aligned (the core checks this itself in
    /// `Dwc3::init` and reports `Error::UnalignedDma`).
    pub fn new(bus: u64) -> Self {
        Self {
            backing: MemModel::new(),
            bus,
            fault: false,
        }
    }

    pub fn bus(&self) -> u64 {
        self.bus
    }

    pub fn faulted(&self) -> bool {
        self.fault
    }

    fn offset(&self, addr: u64, width: Width) -> Option<u64> {
        let off = addr.checked_sub(self.bus)?;
        let end = off.checked_add(width.bytes() as u64)?;
        if end <= DMA_SIZE as u64 {
            Some(off)
        } else {
            None
        }
    }
}

impl Mmio for DmaMmio {
    fn read(&mut self, addr: u64, width: Width) -> u64 {
        match self.offset(addr, width) {
            Some(off) => self.backing.read(off, width),
            None => {
                self.fault = true;
                0
            }
        }
    }

    fn write(&mut self, addr: u64, width: Width, value: u64) {
        match self.offset(addr, width) {
            Some(off) => self.backing.write(off, width, value),
            None => {
                self.fault = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_window_roundtrip() {
        let mut r = RegsMmio::new();
        r.write_u32(CORE_BASE + 0x10, 0xdead_beef);
        assert_eq!(r.read_u32(CORE_BASE + 0x10), 0xdead_beef);
        assert!(!r.faulted());
    }

    #[test]
    fn qscratch_window_roundtrip() {
        let mut r = RegsMmio::new();
        r.write_u32(QSCRATCH_BASE + 0x20, 0x1234_5678);
        assert_eq!(r.read_u32(QSCRATCH_BASE + 0x20), 0x1234_5678);
        assert!(!r.faulted());
    }

    #[test]
    fn gap_address_faults_reads_zero_and_drops_write() {
        let mut r = RegsMmio::new();
        let gap = CORE_BASE + CORE_LEN + 0x100;
        r.write_u32(gap, 0xffff_ffff);
        assert!(r.faulted());
        assert_eq!(r.read_u32(gap), 0);
    }

    #[test]
    fn crossing_core_end_faults_but_last_full_word_ok() {
        let mut r = RegsMmio::new();
        r.write_u32(CORE_BASE + CORE_LEN - 4, 0x0bad_f00d);
        assert!(!r.faulted());
        assert_eq!(r.read_u32(CORE_BASE + CORE_LEN - 4), 0x0bad_f00d);
        r.write_u32(CORE_BASE + CORE_LEN - 2, 1);
        assert!(r.faulted());
    }

    #[test]
    fn dma_roundtrip_by_bus_address() {
        let bus = 0x8000_0000u64;
        let mut d = DmaMmio::new(bus);
        d.write_u32(bus + 0x1400, 0xcafe_babe);
        assert_eq!(d.read_u32(bus + 0x1400), 0xcafe_babe);
        assert!(!d.faulted());
        assert_eq!(d.bus(), bus);
    }

    #[test]
    fn dma_below_bus_and_past_end_fault() {
        let bus = 0x8000_0000u64;
        let mut d = DmaMmio::new(bus);
        assert_eq!(d.read_u32(bus - 4), 0);
        assert!(d.faulted());
        let mut d2 = DmaMmio::new(bus);
        d2.write_u32(bus + DMA_SIZE as u64 - 2, 1);
        assert!(d2.faulted());
        let mut d3 = DmaMmio::new(bus);
        d3.write_u32(bus + DMA_SIZE as u64 - 4, 7);
        assert!(!d3.faulted());
    }

    #[test]
    fn fault_is_sticky_across_valid_accesses() {
        let mut r = RegsMmio::new();
        assert_eq!(r.read_u32(0x1000), 0);
        assert!(r.faulted());
        r.write_u32(CORE_BASE, 1);
        assert_eq!(r.read_u32(CORE_BASE), 1);
        assert!(r.faulted());
    }

    #[test]
    fn dwc3_init_fails_cleanly_on_zeroed_model() {
        use dwc3_gadget::{Config, Dwc3, Error};
        fn delay(_us: u32) {}
        let bus = 0x8000_0000u64;
        let mut core = Dwc3::new(
            RegsMmio::new(),
            DmaMmio::new(bus),
            Config::sdm845_usb1(bus, delay),
        );
        assert_eq!(core.init(), Err(Error::UnsupportedCore(0)));
    }
}
