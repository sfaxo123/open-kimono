use alloc::vec::Vec;
use core::{
    arch::asm,
    ptr::{read_volatile, write_volatile},
};

use self::model::{Gicv3Error, Mmio, RdRegistry, Route};
use super::{gic::GicDistIf, InterruptController};
use crate::{
    dtb::{
        irqchip::{InterruptHandler, IrqCell, IrqDesc},
        translate_mmio_address, Fdt,
    },
    sync::CleanLockToken,
};
use syscall::{
    error::{Error, EINVAL},
    Result,
};

mod model;

/// INTIDs 1020..=1023 are special (1023 = spurious) and never map to a virq.
const INTID_SPECIAL_FIRST: u32 = model::INTID_SPECIAL_FIRST;

/// Upper bound on GICR_WAKER / GICR_CTLR.RWP polling.
const GICR_MAX_POLLS: usize = 1_000_000;

struct KernelMmio;

impl Mmio for KernelMmio {
    fn read32(&self, addr: usize) -> u32 {
        unsafe { read_volatile(addr as *const u32) }
    }
    fn write32(&mut self, addr: usize, value: u32) {
        unsafe { write_volatile(addr as *mut u32, value) }
    }
    fn read64(&self, addr: usize) -> u64 {
        unsafe { read_volatile(addr as *const u64) }
    }
}

fn current_mpidr() -> u64 {
    let mpidr: u64;
    unsafe { asm!("mrs {}, mpidr_el1", out(reg) mpidr) };
    mpidr
}

/// `ICC_IAR1_EL1.INTID` is bits [23:0]; SDM845 SPIs reach INTID 668.
fn iar_intid(iar: usize) -> u32 {
    (iar & 0xff_ffff) as u32
}

#[derive(Debug)]
pub struct GicV3 {
    pub gic_dist_if: GicDistIf,
    pub gic_cpu_if: GicV3CpuIf,
    /// Redistributor regions as (physical address, length).
    pub gicrs: Vec<(usize, usize)>,
    /// DT `redistributor-stride`; `None` = derive from GICR_TYPER.
    pub gicr_stride: Option<usize>,
    /// Virtual RD_base of every CPU that initialized against this controller.
    pub rd_registry: RdRegistry,
    //TODO: GICC, GICH, GICV?
    pub irq_range: (usize, usize),
}

impl GicV3 {
    pub fn new() -> Self {
        GicV3 {
            gic_dist_if: GicDistIf::default(),
            gic_cpu_if: GicV3CpuIf,
            gicrs: Vec::new(),
            gicr_stride: None,
            rd_registry: RdRegistry::new(),
            irq_range: (0, 0),
        }
    }

    fn init_current_rd(&mut self) -> core::result::Result<(), Gicv3Error> {
        let affinity = model::mpidr_affinity(current_mpidr());
        self.rd_registry.remove(affinity);
        let mut regions = Vec::with_capacity(self.gicrs.len());
        for &(phys, len) in &self.gicrs {
            let virt = crate::PHYS_OFFSET
                .checked_add(phys)
                .ok_or(Gicv3Error::BadRegion)?;
            regions.push((virt, len));
        }
        let rd = model::init_cpu_rd(
            &mut KernelMmio,
            &mut self.rd_registry,
            &regions,
            self.gicr_stride,
            affinity,
            self.gic_dist_if.address,
            GICR_MAX_POLLS,
        )?;
        info!("gicv3: affinity {:#x} uses redistributor {:#x}", affinity, rd);
        Ok(())
    }

    /// Private INTIDs are banked per CPU; never fall back to the distributor.
    fn private_rd(&self, intid: u32) -> Option<usize> {
        let affinity = model::mpidr_affinity(current_mpidr());
        match self.rd_registry.lookup(affinity) {
            Ok(rd) => Some(rd),
            Err(_) => {
                error!(
                    "gicv3: private INTID {} on affinity {:#x} has no redistributor mapping",
                    intid, affinity
                );
                None
            }
        }
    }

    pub fn parse(&mut self, fdt: &Fdt) -> Result<()> {
        let Some(node) = fdt.all_compatible(&["arm,gic-v3"]).next() else {
            return Err(Error::new(EINVAL));
        };

        // Clear current registers
        //TODO: deinit?
        self.gic_dist_if.address = 0;
        self.gicrs.clear();
        self.rd_registry.clear();

        // Get number of GICRs
        let gicrs = node
            .raw_property("#redistributor-regions")
            .and_then(|prop| prop.as_value::<u32>().ok())
            .unwrap_or(1) as usize;
        // Validate the actual property bytes before distributor initialization/MMIO.
        let stride = model::parse_stride_property(
            node.raw_property("redistributor-stride").map(|prop| prop.value),
        ).map_err(|err| {
            error!("gicv3: invalid redistributor-stride: {:?}", err);
            Error::new(EINVAL)
        })?;
        self.gicr_stride = stride;

        // Read registers: <GICD> then one tuple per redistributor region.
        let chunks = node.reg().ok_or_else(|| Error::new(EINVAL))?;
        let mut entries = Vec::new();
        for chunk in chunks.iter::<usize, usize>().take(gicrs.saturating_add(1)) {
            let chunk = chunk.map_err(|_| Error::new(EINVAL))?;
            let addr =
                translate_mmio_address(fdt, &node, &chunk).ok_or_else(|| Error::new(EINVAL))?;
            entries.push((addr, chunk.len));
        }
        let ((gicd, _), rds) = model::split_regions(&entries, gicrs).map_err(|err| {
            error!("gicv3: bad reg property: {:?}", err);
            Error::new(EINVAL)
        })?;
        self.gicrs.extend_from_slice(rds);
        unsafe {
            self.gic_dist_if.init(crate::PHYS_OFFSET + gicd);
            self.gic_dist_if.set_spis_group1();
        }
        Ok(())
    }
}

impl InterruptHandler for GicV3 {
    fn irq_handler(&mut self, _irq: u32, _token: &mut CleanLockToken) {}
}

impl InterruptController for GicV3 {
    fn irq_init(
        &mut self,
        fdt_opt: Option<&Fdt>,
        irq_desc: &mut [IrqDesc; 2048],
        ic_idx: usize,
        irq_idx: &mut usize,
    ) -> Result<()> {
        if let Some(fdt) = fdt_opt {
            self.parse(fdt)?;
        }
        info!("{:X?}", self);

        // SPIs stay usable without a redistributor; private INTIDs then fail
        // loudly on enable instead of being written to GICD.
        if let Err(err) = self.init_current_rd() {
            error!("gicv3: current CPU redistributor unavailable: {:?}", err);
        }
        unsafe {
            self.gic_cpu_if.init();
        }
        let idx = *irq_idx;
        let cnt = if self.gic_dist_if.nirqs > 1024 {
            1024
        } else {
            self.gic_dist_if.nirqs as usize
        };
        let mut i: usize = 0;
        //only support linear irq map now.
        while i < cnt && (idx + i < 1024) {
            irq_desc[idx + i].basic.ic_idx = ic_idx;
            irq_desc[idx + i].basic.ic_irq = i as u32;
            irq_desc[idx + i].basic.used = false;

            i += 1;
        }

        info!("gic irq_range = ({}, {})", idx, idx + cnt);
        self.irq_range = (idx, idx + cnt);
        *irq_idx = idx + cnt;
        Ok(())
    }
    fn irq_ack(&mut self) -> u32 {
        let irq_num = unsafe { self.gic_cpu_if.irq_ack() };
        irq_num
    }
    fn irq_eoi(&mut self, irq_num: u32) {
        unsafe { self.gic_cpu_if.irq_eoi(irq_num) }
    }
    fn irq_enable(&mut self, irq_num: u32) {
        match model::route(irq_num, self.gic_dist_if.nirqs) {
            Ok(Route::Private(intid)) => {
                if let Some(rd) = self.private_rd(intid)
                    && let Err(err) = model::private_enable(&mut KernelMmio, rd, intid)
                {
                    error!("gicv3: enable INTID {} failed: {:?}", intid, err);
                }
            }
            Ok(Route::Shared(intid)) => unsafe { self.gic_dist_if.irq_enable(intid) },
            Err(err) => {
                error!("gicv3: enable rejected: {:?}", err);
            }
        }
    }
    fn irq_disable(&mut self, irq_num: u32) {
        match model::route(irq_num, self.gic_dist_if.nirqs) {
            Ok(Route::Private(intid)) => {
                if let Some(rd) = self.private_rd(intid)
                    && let Err(err) =
                        model::private_disable(&mut KernelMmio, rd, intid, GICR_MAX_POLLS)
                {
                    error!("gicv3: disable INTID {} failed: {:?}", intid, err);
                }
            }
            Ok(Route::Shared(intid)) => unsafe { self.gic_dist_if.irq_disable(intid) },
            Err(err) => {
                error!("gicv3: disable rejected: {:?}", err);
            }
        }
    }
    fn irq_configure(&mut self, irq_data: IrqCell) -> Result<()> {
        match irq_data {
            IrqCell::L3(0, irq, flags) => {
                // SPI
                let hwirq = irq.checked_add(32).ok_or_else(|| Error::new(EINVAL))?;
                unsafe { self.gic_dist_if.irq_configure(hwirq, flags) }
            }
            IrqCell::L3(1, irq, flags) => {
                // PPI
                let intid = irq.checked_add(16).ok_or_else(|| Error::new(EINVAL))?;
                let rd = self.private_rd(intid).ok_or_else(|| Error::new(EINVAL))?;
                model::ppi_configure(&mut KernelMmio, rd, intid, flags).map_err(|err| {
                    error!("gicv3: configure INTID {} failed: {:?}", intid, err);
                    Error::new(EINVAL)
                })
            }
            _ => Err(Error::new(EINVAL)),
        }
    }
    fn irq_xlate(&self, irq_data: IrqCell) -> Result<usize> {
        let off = match irq_data {
            IrqCell::L3(0, irq, _flags) => irq as usize + 32, // SPI
            IrqCell::L3(1, irq, _flags) => irq as usize + 16, // PPI
            _ => return Err(Error::new(EINVAL)),
        };
        if off >= self.gic_dist_if.nirqs as usize {
            return Err(Error::new(EINVAL));
        }
        Ok(off + self.irq_range.0)
    }
    fn irq_to_virq(&self, hwirq: u32) -> Option<usize> {
        if hwirq >= INTID_SPECIAL_FIRST || hwirq >= self.gic_dist_if.nirqs {
            None
        } else {
            Some(self.irq_range.0 + hwirq as usize)
        }
    }
}

#[derive(Debug)]
pub struct GicV3CpuIf;

impl GicV3CpuIf {
    pub unsafe fn init(&mut self) {
        unsafe {
            // Enable system register access
            {
                let value = 1_usize;
                // Synchronize system-register interface enable before other ICC accesses.
                asm!("msr icc_sre_el1, {}", "isb", in(reg) value);
            }
            // Set control register
            {
                let value = 0_usize;
                asm!("msr icc_ctlr_el1, {}", in(reg) value);
            }
            // Enable non-secure group 1
            {
                let value = 1_usize;
                asm!("msr icc_igrpen1_el1, {}", in(reg) value);
            }
            // Set CPU0's Interrupt Priority Mask
            {
                let value = 0xFF_usize;
                asm!("msr icc_pmr_el1, {}", "isb", in(reg) value);
            }
        }
    }

    /// Returns the acknowledged INTID; 1020..=1023 (spurious) are passed on
    /// so the dispatcher can skip the EOI.
    unsafe fn irq_ack(&mut self) -> u32 {
        unsafe {
            let iar: usize;
            asm!("mrs {}, icc_iar1_el1", out(reg) iar);
            iar_intid(iar)
        }
    }

    unsafe fn irq_eoi(&mut self, irq: u32) {
        unsafe {
            asm!("msr icc_eoir1_el1, {}", in(reg) irq as usize);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gicv3(nirqs: u32) -> GicV3 {
        let mut gic = GicV3::new();
        gic.gic_dist_if.nirqs = nirqs;
        gic.irq_range = (0, nirqs as usize);
        gic
    }

    #[test]
    fn iar_keeps_intids_above_511() {
        // SDM845 PDC maps pins onto SPIs up to 636, i.e. INTID 668.
        assert_eq!(iar_intid(668), 668);
        assert_eq!(iar_intid(386), 386);
        assert_eq!(iar_intid(0xff00_0000_03ff), 1023);
    }

    #[test]
    fn spi_354_round_trips_through_virq_386() {
        let gic = gicv3(1024);
        let virq = gic.irq_xlate(IrqCell::L3(0, 354, 4)).unwrap();
        assert_eq!(virq, 386);
        assert_eq!(gic.irq_to_virq(iar_intid(386)), Some(virq));
    }

    #[test]
    fn special_intids_never_map_to_a_virq() {
        let gic = gicv3(1024);
        for intid in 1020..=1023 {
            assert_eq!(gic.irq_to_virq(intid), None);
        }
        assert_eq!(gicv3(640).irq_to_virq(640), None);
        assert_eq!(gicv3(640).irq_to_virq(639), Some(639));
    }
}
