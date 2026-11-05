//! V1b (part 1) — physmap / DMA *planning* layer for `usbgadgetd`.
//!
//! Pure, host-testable validation that the Redox constructors in
//! part 2 will run BEFORE touching `common::physmap` /
//! `common::dma::Dma`. It encodes three facts read from the forks
//! sources on 2026-10-29 (base/drivers/common/src/{lib,dma}.rs):
//!
//! 1. `physmap` rejects a zero physical base (`EINVAL`) and maps
//!    `len.next_multiple_of(PAGE_SIZE)` bytes starting AT the given
//!    base as the mmap offset. A window whose base is not page
//!    aligned therefore cannot be mapped naively: the mapping must
//!    start at the page floor and the window pointer must be
//!    offset by the in-page displacement. This is not theoretical —
//!    the QSCRATCH window base `0x0a6f_8800` has in-page offset
//!    `0x800` (see the qscratch test below, which pins it). A base
//!    so low that its page floor is 0 (e.g. `0x800`) is rejected
//!    too: floor 0 is exactly the base `physmap` refuses.
//! 2. `Dma::zeroed()` for `[u8; DMA_SIZE]` rounds the allocation
//!    up to whole pages: DMA_SIZE is 22 KiB, so the allocation is
//!    24 KiB (6 pages). The physical address comes from
//!    `Dma::physical()` in the current forks source (the wiring
//!    design said `phys()`; that method does not exist — corrected
//!    in USBGADGETD-WIRING.md by this same packet).
//! 3. SMMU gate (V3 oracle, 6d21c39): `a600000.usb` is TRANSLATED
//!    (iommu_group 12), so `bus == phys` may NOT be assumed. A DMA
//!    plan is only constructible with an explicit mode: `Bypass`
//!    (bus is phys, to be used only if platform setup puts SID
//!    0x740 into bypass and verifies it) or `Translated` (caller
//!    supplies the translated bus address explicitly). `Unproven`
//!    is rejected here, in code, not just in prose. Enforcement is
//!    structural: `DmaPlan`'s fields are private and `plan_dma` is
//!    the only constructor (Wonder diff review of b0a808d).
//!
//! Part 2 status (2026-10-30): the `common::` constructors
//! themselves (`PhysBorrowed::map` / `Dma::zeroed`) now live in
//! `redox_glue.rs` (Redox target only), constructed exclusively
//! through the plans below; the `common` dependency naming is
//! settled there. Wiring into `main` (payload v2) is the next
//! packet, gated on the SMMU mode decision at runtime.

use dwc3_gadget::layout::DMA_SIZE;

use crate::windows::{CORE_BASE, CORE_LEN, QSCRATCH_BASE, QSCRATCH_LEN};

/// Redox page size (`syscall::PAGE_SIZE` on aarch64).
pub const PAGE_SIZE: u64 = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanError {
    /// `physmap` returns EINVAL for a zero base (lib.rs, read
    /// 2026-10-29) — including a nonzero base whose page floor is 0.
    ZeroBase,
    ZeroLen,
    Overflow,
    /// DMA physical/bus addresses must be page aligned: the Dma
    /// allocation is page sized and the dwc3 core itself requires a
    /// 4 KiB-aligned DMA base (`Error::UnalignedDma`).
    Unaligned,
    /// SMMU stream state not established — planning DMA is refused.
    SmmuUnproven,
    /// Translated mode was requested without a bus address.
    MissingTranslatedBus,
}

/// A validated MMIO window mapping plan. Fields are private;
/// construct via `plan_window` / the window helpers below.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowPlan {
    phys_base: u64,
    len: u64,
    map_base: u64,
    map_len: u64,
    in_page_offset: u64,
}

impl WindowPlan {
    /// Physical base the caller asked for.
    pub fn phys_base(&self) -> u64 {
        self.phys_base
    }
    /// Window length the caller asked for.
    pub fn len(&self) -> u64 {
        self.len
    }
    /// Page floor of `phys_base`: where the mapping must start.
    pub fn map_base(&self) -> u64 {
        self.map_base
    }
    /// Page-rounded length covering `[map_base, phys_base + len)`.
    pub fn map_len(&self) -> u64 {
        self.map_len
    }
    /// `phys_base - map_base`: add this to the mapped pointer.
    pub fn in_page_offset(&self) -> u64 {
        self.in_page_offset
    }

    /// Translate an absolute address inside this window to an
    /// offset into the page-floor mapping — the exact arithmetic
    /// the part-2 pointer constructor will use. `None` for any
    /// access not fully inside `[phys_base, phys_base + len)`.
    pub fn translate(&self, addr: u64, width: u64) -> Option<u64> {
        let end = addr.checked_add(width)?;
        if addr >= self.phys_base && end <= self.phys_base + self.len {
            Some(self.in_page_offset + (addr - self.phys_base))
        } else {
            None
        }
    }

    /// Offset of an access relative to the window BASE pointer —
    /// `translate()` minus `in_page_offset`. This is the exact
    /// arithmetic the Redox pointer impls apply: `redox_glue`
    /// constructs base pointers at `mapping + translate(base, len)`
    /// (= `mapping + in_page_offset`), and `redox_mmio::locate`
    /// indexes them with this function, so the composition is
    /// pinned by the host test below (Wonder diff review of
    /// a5a46e8: the cfg(redox) composition itself never executes
    /// in host tests, QEMU, or the link check).
    pub fn rel_offset(&self, addr: u64, width: u64) -> Option<u64> {
        self.translate(addr, width)
            .map(|off| off - self.in_page_offset)
    }
}

pub fn plan_window(phys_base: u64, len: u64) -> Result<WindowPlan, PlanError> {
    if phys_base == 0 {
        return Err(PlanError::ZeroBase);
    }
    if len == 0 {
        return Err(PlanError::ZeroLen);
    }
    let end = phys_base.checked_add(len).ok_or(PlanError::Overflow)?;
    let map_base = phys_base & !(PAGE_SIZE - 1);
    if map_base == 0 {
        // The mapping would start at physical 0 — the exact base
        // physmap rejects with EINVAL (Wonder diff review, b0a808d).
        return Err(PlanError::ZeroBase);
    }
    let map_end = end
        .checked_add(PAGE_SIZE - 1)
        .ok_or(PlanError::Overflow)?
        & !(PAGE_SIZE - 1);
    Ok(WindowPlan {
        phys_base,
        len,
        map_base,
        map_len: map_end - map_base,
        in_page_offset: phys_base - map_base,
    })
}

pub fn core_window_plan() -> Result<WindowPlan, PlanError> {
    plan_window(CORE_BASE, CORE_LEN)
}

pub fn qscratch_window_plan() -> Result<WindowPlan, PlanError> {
    plan_window(QSCRATCH_BASE, QSCRATCH_LEN)
}

/// How the DWC3 DMA stream (SID 0x740) will reach memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DmaMode {
    /// Stream state not yet established. Planning is refused.
    Unproven,
    /// Platform setup puts SID 0x740 in bypass and verifies it:
    /// the device sees physical addresses.
    Bypass,
    /// SMMU translates: the device sees `translated_bus`, which the
    /// caller must supply from the translation setup — never phys.
    Translated,
}

/// A validated DMA plan. Fields are private; `plan_dma` is the
/// only constructor, so the SMMU mode gate cannot be bypassed by
/// struct literal construction inside the crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DmaPlan {
    phys: u64,
    bus: u64,
    alloc_len: u64,
}

impl DmaPlan {
    /// Physical address of the DMA allocation (`Dma::physical()`).
    pub fn phys(&self) -> u64 {
        self.phys
    }
    /// Bus address handed to the dwc3 core / device.
    pub fn bus(&self) -> u64 {
        self.bus
    }
    /// Page-rounded allocation length for `DMA_SIZE` bytes.
    pub fn alloc_len(&self) -> u64 {
        self.alloc_len
    }
}

pub fn plan_dma(
    phys: u64,
    mode: DmaMode,
    translated_bus: Option<u64>,
) -> Result<DmaPlan, PlanError> {
    if phys % PAGE_SIZE != 0 {
        return Err(PlanError::Unaligned);
    }
    let alloc_len = (DMA_SIZE as u64)
        .checked_add(PAGE_SIZE - 1)
        .ok_or(PlanError::Overflow)?
        & !(PAGE_SIZE - 1);
    let bus = match mode {
        DmaMode::Unproven => return Err(PlanError::SmmuUnproven),
        DmaMode::Bypass => phys,
        DmaMode::Translated => {
            let b = translated_bus.ok_or(PlanError::MissingTranslatedBus)?;
            if b % PAGE_SIZE != 0 {
                return Err(PlanError::Unaligned);
            }
            b
        }
    };
    Ok(DmaPlan {
        phys,
        bus,
        alloc_len,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_window_is_page_aligned() {
        let p = core_window_plan().unwrap();
        assert_eq!(p.map_base(), CORE_BASE);
        assert_eq!(p.in_page_offset(), 0);
        // 0xcd00 rounds up to 13 pages.
        assert_eq!(p.map_len(), 0xd000);
    }

    #[test]
    fn qscratch_window_needs_page_floor_and_offset() {
        // Pins the finding: QSCRATCH_BASE is NOT page aligned, so a
        // naive physmap(QSCRATCH_BASE, ...) plan is wrong; the
        // mapping starts one half-page earlier.
        let p = qscratch_window_plan().unwrap();
        assert_eq!(p.map_base(), 0x0a6f_8000);
        assert_eq!(p.in_page_offset(), 0x800);
        assert_eq!(p.map_len(), PAGE_SIZE);
    }

    #[test]
    fn fake_window_qscratch_offset_roundtrip() {
        // Host fake-window test (Wonder V1b spec): a byte buffer
        // stands in for the page-floor mapping, addressed ONLY
        // through the production `translate` helper.
        let p = qscratch_window_plan().unwrap();
        let mut fake = vec![0xaau8; p.map_len() as usize];
        // Exact index: window offset 0x20 lands at 0x800 + 0x20.
        let idx = p.translate(QSCRATCH_BASE + 0x20, 4).unwrap() as usize;
        assert_eq!(idx, 0x820);
        fake[idx] = 0x5a;
        assert_eq!(fake[0x820], 0x5a);
        // Pre-offset sentinel region (the half-page before the
        // window) is untouched by the translated write.
        assert!(fake[..0x800].iter().all(|&b| b == 0xaa));
        // Bounds through the same helper: below the window, at the
        // page floor itself, and an access crossing the window end
        // are all refused; the last full word is accepted at the
        // exact index 0x800 + 0x400 - 4.
        assert_eq!(p.translate(QSCRATCH_BASE - 4, 4), None);
        assert_eq!(p.translate(p.map_base(), 4), None);
        assert_eq!(p.translate(QSCRATCH_BASE + QSCRATCH_LEN - 2, 4), None);
        assert_eq!(
            p.translate(QSCRATCH_BASE + QSCRATCH_LEN - 4, 4),
            Some(0x800 + 0x400 - 4)
        );
    }

    #[test]
    fn glue_mmio_composition_lands_exactly() {
        // Composition test (Wonder diff review of a5a46e8): a byte
        // buffer stands in for each page-floor mapping. The glue
        // side is emulated exactly as redox_glue.rs does it — base
        // index = translate(base, len) — and the mmio side exactly
        // as redox_mmio.rs does it — index = base index +
        // rel_offset(addr, width). Accesses must land at the
        // translate() index, nowhere else.
        for plan in [core_window_plan().unwrap(), qscratch_window_plan().unwrap()] {
            let mut fake = vec![0xaau8; plan.map_len() as usize];
            let base_idx = plan.translate(plan.phys_base(), plan.len()).unwrap() as usize;
            assert_eq!(base_idx, plan.in_page_offset() as usize);
            // Base, intermediate, and final valid word accesses.
            for (addr, expect_rel) in [
                (plan.phys_base(), 0u64),
                (plan.phys_base() + 0x20, 0x20),
                (plan.phys_base() + plan.len() - 4, plan.len() - 4),
            ] {
                let rel = plan.rel_offset(addr, 4).unwrap();
                assert_eq!(rel, expect_rel);
                let idx = base_idx + rel as usize;
                assert_eq!(idx, plan.translate(addr, 4).unwrap() as usize);
                fake[idx] = 0x5a;
                assert_eq!(fake[idx], 0x5a);
            }
            // The pre-base region of the mapping (the in-page
            // offset area) is untouched by base-relative writes.
            assert!(fake[..base_idx].iter().all(|&b| b == 0xaa));
            // Pre-base and boundary-crossing accesses are refused
            // by the same function the mmio side calls.
            assert_eq!(plan.rel_offset(plan.phys_base() - 4, 4), None);
            // The page floor itself is pre-base only when the
            // window is not page aligned (QSCRATCH); for the
            // aligned core window map_base == phys_base and the
            // access above already proved it valid.
            if plan.map_base() != plan.phys_base() {
                assert_eq!(plan.rel_offset(plan.map_base(), 4), None);
            }
            assert_eq!(plan.rel_offset(plan.phys_base() + plan.len() - 2, 4), None);
        }
        // Pin the QSCRATCH numbers explicitly: a double-offset
        // would land at 0x820+0x800, an underflow would panic —
        // the exact indices are 0x800 (mapping) / 0x20 (relative).
        let q = qscratch_window_plan().unwrap();
        assert_eq!(q.translate(QSCRATCH_BASE + 0x20, 4), Some(0x820));
        assert_eq!(q.rel_offset(QSCRATCH_BASE + 0x20, 4), Some(0x20));
        let c = core_window_plan().unwrap();
        assert_eq!(c.rel_offset(CORE_BASE + 0x20, 4), Some(0x20));
    }

    #[test]
    fn zero_base_and_len_rejected() {
        assert_eq!(plan_window(0, 0x1000), Err(PlanError::ZeroBase));
        assert_eq!(plan_window(0x1000, 0), Err(PlanError::ZeroLen));
    }

    #[test]
    fn sub_page_base_with_zero_floor_rejected() {
        // 0x800 != 0, but its page floor is 0 — physmap would
        // EINVAL the mapping (Wonder diff review of b0a808d).
        assert_eq!(plan_window(0x800, 0x100), Err(PlanError::ZeroBase));
    }

    #[test]
    fn overflowing_window_rejected() {
        assert_eq!(
            plan_window(u64::MAX - 0x800, 0x1000),
            Err(PlanError::Overflow)
        );
    }

    #[test]
    fn dma_bypass_bus_is_phys_and_alloc_is_six_pages() {
        let d = plan_dma(0x8000_0000, DmaMode::Bypass, None).unwrap();
        assert_eq!(d.bus(), d.phys());
        assert_eq!(d.alloc_len(), 24 * 1024);
    }

    #[test]
    fn dma_unproven_is_refused() {
        assert_eq!(
            plan_dma(0x8000_0000, DmaMode::Unproven, None),
            Err(PlanError::SmmuUnproven)
        );
    }

    #[test]
    fn dma_translated_requires_and_keeps_supplied_bus() {
        assert_eq!(
            plan_dma(0x8000_0000, DmaMode::Translated, None),
            Err(PlanError::MissingTranslatedBus)
        );
        let d = plan_dma(0x8000_0000, DmaMode::Translated, Some(0x9000_0000)).unwrap();
        assert_eq!(d.phys(), 0x8000_0000);
        assert_eq!(d.bus(), 0x9000_0000);
    }

    #[test]
    fn dma_unaligned_addresses_rejected() {
        assert_eq!(
            plan_dma(0x8000_0800, DmaMode::Bypass, None),
            Err(PlanError::Unaligned)
        );
        assert_eq!(
            plan_dma(0x8000_0000, DmaMode::Translated, Some(0x9000_0800)),
            Err(PlanError::Unaligned)
        );
    }
}
