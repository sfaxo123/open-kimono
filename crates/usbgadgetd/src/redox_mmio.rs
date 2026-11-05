//! Redox pointer-backed MMIO impls (`cfg(target_os = "redox")` only).
//!
//! These mirror the host `windows.rs` semantics exactly (same windows,
//! same sticky-fault behaviour) over volatile accesses. They are the
//! only `unsafe` in the daemon; the `dwc3-gadget` core stays
//! `forbid(unsafe_code)`.
//!
//! The physmap glue that produces the window pointers lives in
//! `redox_glue.rs` (V1b part 2): `PhysBorrowed::map` over the
//! part-1 page-floor plans, and `common::dma::Dma` for the DMA
//! region, per USBGADGETD-WIRING.md section 3. NOT YET CONSTRUCTED
//! by `main` — payload-v2 wiring is the next packet, so the
//! daemon still deliberately never touches hardware.
#![allow(dead_code)]

use dwc3_gadget::layout::DMA_SIZE;
use mmio_replay::{Mmio, Width};

use crate::physmap_plan::{core_window_plan, qscratch_window_plan, WindowPlan};

unsafe fn vol_read(ptr: *const u8, width: Width) -> u64 {
    // SAFETY: caller guarantees `ptr` is valid for a `width`-sized
    // volatile read inside a mapped MMIO/DMA window.
    unsafe {
        match width {
            Width::W8 => std::ptr::read_volatile(ptr) as u64,
            Width::W16 => std::ptr::read_volatile(ptr as *const u16) as u64,
            Width::W32 => std::ptr::read_volatile(ptr as *const u32) as u64,
            Width::W64 => std::ptr::read_volatile(ptr as *const u64),
        }
    }
}

unsafe fn vol_write(ptr: *mut u8, width: Width, value: u64) {
    // SAFETY: as for `vol_read`, for a write.
    unsafe {
        match width {
            Width::W8 => std::ptr::write_volatile(ptr, value as u8),
            Width::W16 => std::ptr::write_volatile(ptr as *mut u16, value as u16),
            Width::W32 => std::ptr::write_volatile(ptr as *mut u32, value as u32),
            Width::W64 => std::ptr::write_volatile(ptr as *mut u64, value),
        }
    }
}

/// Register window over two mapped device-memory windows.
///
/// Address translation goes through the part-1 `WindowPlan`s
/// (`physmap_plan::WindowPlan::translate`), NOT a local bounds
/// check: the plans are the host-tested arithmetic, and their
/// returned offset is into the page-floor mapping — which is
/// exactly where the pointers below point, because `redox_glue`
/// constructs them already offset to each window base.
pub struct RegsPtrMmio {
    core: *mut u8,
    qscratch: *mut u8,
    core_plan: WindowPlan,
    qscratch_plan: WindowPlan,
    fault: bool,
}

impl RegsPtrMmio {
    /// SAFETY: `core` / `qscratch` must point AT the core /
    /// QSCRATCH window bases (the glue adds each plan's in-page
    /// offset to the page-floor mapping pointer), inside mappings
    /// of at least the plans' `map_len` bytes that stay alive for
    /// the lifetime of this value, accessed only from one thread.
    pub unsafe fn new(core: *mut u8, qscratch: *mut u8) -> Self {
        Self {
            core,
            qscratch,
            // The window constants are pinned by host tests in
            // physmap_plan; a plan failure here would be a bug in
            // those constants, not a runtime condition.
            core_plan: core_window_plan().expect("pinned CORE window plan"),
            qscratch_plan: qscratch_window_plan().expect("pinned QSCRATCH window plan"),
            fault: false,
        }
    }

    pub fn faulted(&self) -> bool {
        self.fault
    }

    fn locate(&self, addr: u64, width: Width) -> Option<*mut u8> {
        // rel_offset() is the host-tested base-relative arithmetic
        // (physmap_plan): the stored pointers are already offset
        // to the window bases by redox_glue, so they are indexed
        // relative to those bases, never by mapping offsets.
        if let Some(rel) = self.core_plan.rel_offset(addr, width.bytes() as u64) {
            // SAFETY: rel_offset checked the access inside the window.
            Some(unsafe { self.core.add(rel as usize) })
        } else if let Some(rel) = self.qscratch_plan.rel_offset(addr, width.bytes() as u64) {
            // SAFETY: as above, for the QSCRATCH window.
            Some(unsafe { self.qscratch.add(rel as usize) })
        } else {
            None
        }
    }
}

impl Mmio for RegsPtrMmio {
    fn read(&mut self, addr: u64, width: Width) -> u64 {
        match self.locate(addr, width) {
            Some(p) => unsafe { vol_read(p, width) },
            None => {
                self.fault = true;
                0
            }
        }
    }

    fn write(&mut self, addr: u64, width: Width, value: u64) {
        match self.locate(addr, width) {
            Some(p) => unsafe { vol_write(p, width, value) },
            None => {
                self.fault = true;
            }
        }
    }
}

/// DMA window over one mapped uncached region, addressed by bus address.
pub struct DmaPtrMmio {
    base: *mut u8,
    bus: u64,
    fault: bool,
}

impl DmaPtrMmio {
    /// SAFETY: `base` must point at a mapping of at least `DMA_SIZE`
    /// bytes whose bus address is `bus`, alive for this value's
    /// lifetime, accessed only from one thread.
    pub unsafe fn new(base: *mut u8, bus: u64) -> Self {
        Self {
            base,
            bus,
            fault: false,
        }
    }

    pub fn faulted(&self) -> bool {
        self.fault
    }

    fn locate(&self, addr: u64, width: Width) -> Option<*mut u8> {
        let off = addr.checked_sub(self.bus)?;
        let end = off.checked_add(width.bytes() as u64)?;
        if end <= DMA_SIZE as u64 {
            // SAFETY: offset checked inside the DMA region above.
            Some(unsafe { self.base.add(off as usize) })
        } else {
            None
        }
    }
}

impl Mmio for DmaPtrMmio {
    fn read(&mut self, addr: u64, width: Width) -> u64 {
        match self.locate(addr, width) {
            Some(p) => unsafe { vol_read(p, width) },
            None => {
                self.fault = true;
                0
            }
        }
    }

    fn write(&mut self, addr: u64, width: Width, value: u64) {
        match self.locate(addr, width) {
            Some(p) => unsafe { vol_write(p, width, value) },
            None => {
                self.fault = true;
            }
        }
    }
}
