//! V1b (part 2) — Redox constructors for the `usbgadgetd` MMIO/DMA
//! windows (`cfg(target_os = "redox")` only).
//!
//! This is the glue `physmap_plan.rs` (part 1) planned for: every
//! mapping/allocation is constructed THROUGH the part-1 plans, so
//! the page-floor / in-page-offset arithmetic and the SMMU mode
//! gate are applied in exactly one place.
//!
//! Dependency naming (settled by experiment, 2026-10-30): the
//! standalone crate names the Redox drivers `common` crate as a
//! target-specific path dependency on the forks tree the whole
//! repo already builds against (G-W1 recipe, stage scripts'
//! REDOX_FORKS default):
//! `common = { path = "../../../../forks/redox/base-e1/drivers/common" }`
//! Verified: host `cargo test --offline` stays green (the dep is
//! not built on host) and the cookbook-toolchain
//! `cargo build --offline --target aarch64-unknown-redox` compiles
//! `common` + `libredox` + `redox_syscall` and links.
//!
//! NOT YET CALLED from `main`: payload-v2 wiring is the next
//! packet. It must (a) call `common::init()` exactly once first
//! (it panics on a second call, per the `common` docs), (b) pass
//! `DmaMode::Bypass` only after platform setup has put SID 0x740
//! in bypass AND verified it, or `DmaMode::Translated` with the
//! bus address from the translation setup — the V3 oracle proved
//! the stream is TRANSLATED (iommu_group 12), so identity DMA must
//! never be assumed, and (c) re-pin the V2 QEMU expectation, whose
//! daemon line changes once `main` attempts real mappings.

use common::dma::Dma;
use common::{MemoryType, PhysBorrowed, Prot};
use dwc3_gadget::layout::DMA_SIZE;

use crate::physmap_plan::{
    core_window_plan, plan_dma, plan_window, qscratch_window_plan, DmaMode, DmaPlan, PlanError,
};
use crate::redox_mmio::{DmaPtrMmio, RegsPtrMmio};
use crate::windows::{CORE_BASE, CORE_LEN, QSCRATCH_BASE, QSCRATCH_LEN};

#[derive(Debug)]
pub enum GlueError {
    Plan(PlanError),
    Map(String),
    DmaAlloc(String),
}

impl std::fmt::Display for GlueError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GlueError::Plan(e) => write!(f, "window/DMA plan refused: {e:?}"),
            GlueError::Map(e) => write!(f, "physmap failed: {e}"),
            GlueError::DmaAlloc(e) => write!(f, "DMA allocation failed: {e}"),
        }
    }
}

impl std::error::Error for GlueError {}

/// Hardware windows for the DWC3 core: the two device-memory
/// mappings, the DMA allocation, and the pointer-backed `Mmio`
/// impls over them. The mappings/allocation outlive the `Mmio`
/// impls because all of them are owned by this one struct; the
/// raw pointers inside the impls are only dereferenced through
/// the `Mmio` trait while this struct is alive.
pub struct HwWindows {
    regs: RegsPtrMmio,
    dma: DmaPtrMmio,
    dma_plan: DmaPlan,
    _core_map: PhysBorrowed,
    _qscratch_map: PhysBorrowed,
    _dma: Dma<[u8; DMA_SIZE]>,
}

impl HwWindows {
    /// Register window, borrowed against `&mut self` so the
    /// mappings backing its raw pointers cannot be dropped while
    /// it is in use (Wonder diff review of a5a46e8: public fields
    /// allowed moving a wrapper out and dropping the mappings).
    pub fn regs(&mut self) -> &mut RegsPtrMmio {
        &mut self.regs
    }

    /// DMA window; same borrowing discipline as `regs`.
    pub fn dma(&mut self) -> &mut DmaPtrMmio {
        &mut self.dma
    }

    /// The SMMU-gated DMA plan this window was constructed with.
    pub fn dma_plan(&self) -> &DmaPlan {
        &self.dma_plan
    }
}

/// Map the core + QSCRATCH windows and allocate the DMA region,
/// per the part-1 plans.
///
/// SMMU gate: the mode is PREFLIGHTED before any mapping or
/// allocation exists (Wonder diff review of a5a46e8): `Unproven`,
/// a missing translated bus, and an unaligned translated bus are
/// refused up front, so on refusal no `Mmio` impl has been
/// constructed and no address has been handed anywhere. The full
/// `plan_dma` check still runs on the real allocation afterwards.
pub fn map_windows(mode: DmaMode, translated_bus: Option<u64>) -> Result<HwWindows, GlueError> {
    match mode {
        DmaMode::Unproven => return Err(GlueError::Plan(PlanError::SmmuUnproven)),
        DmaMode::Translated => {
            let b = translated_bus.ok_or(GlueError::Plan(PlanError::MissingTranslatedBus))?;
            if b % crate::physmap_plan::PAGE_SIZE != 0 {
                return Err(GlueError::Plan(PlanError::Unaligned));
            }
        }
        DmaMode::Bypass => {}
    }
    let core_plan = core_window_plan().map_err(GlueError::Plan)?;
    let q_plan = qscratch_window_plan().map_err(GlueError::Plan)?;
    // The plans are also recomputed inside RegsPtrMmio::new; the
    // copies here drive the mapping bases/lengths and offsets.
    debug_assert_eq!(core_plan, plan_window(CORE_BASE, CORE_LEN).map_err(GlueError::Plan)?);
    debug_assert_eq!(q_plan, plan_window(QSCRATCH_BASE, QSCRATCH_LEN).map_err(GlueError::Plan)?);

    let core_map = PhysBorrowed::map(
        core_plan.map_base() as usize,
        core_plan.map_len() as usize,
        Prot::RW,
        MemoryType::DeviceMemory,
    )
    .map_err(|e| GlueError::Map(format!("core window: {e:?}")))?;
    let qscratch_map = PhysBorrowed::map(
        q_plan.map_base() as usize,
        q_plan.map_len() as usize,
        Prot::RW,
        MemoryType::DeviceMemory,
    )
    .map_err(|e| GlueError::Map(format!("qscratch window: {e:?}")))?;

    // Pointer AT each window base: page-floor mapping pointer plus
    // the plan's translate() offset for the full window (0 for the
    // core, 0x800 for QSCRATCH — the part-1 finding).
    let core_off = core_plan
        .translate(CORE_BASE, CORE_LEN)
        .ok_or(GlueError::Plan(PlanError::Overflow))? as usize;
    let q_off = q_plan
        .translate(QSCRATCH_BASE, QSCRATCH_LEN)
        .ok_or(GlueError::Plan(PlanError::Overflow))? as usize;
    // SAFETY: each offset is translate()-checked to lie fully
    // inside its mapping (map_len covers [map_base, base+len)).
    let core_ptr = unsafe { (core_map.as_ptr() as *mut u8).add(core_off) };
    let qscratch_ptr = unsafe { (qscratch_map.as_ptr() as *mut u8).add(q_off) };
    // SAFETY: the pointers point at the window bases inside
    // mappings owned by the returned struct, which outlives the
    // RegsPtrMmio built here; single-threaded daemon access.
    let regs = unsafe { RegsPtrMmio::new(core_ptr, qscratch_ptr) };

    let dma_zeroed = Dma::<[u8; DMA_SIZE]>::zeroed()
        .map_err(|e| GlueError::DmaAlloc(format!("{e:?}")))?;
    // SAFETY: zeroed DMA memory is a valid, initialized [u8; N].
    let dma = unsafe { dma_zeroed.assume_init() };
    let dma_plan = plan_dma(dma.physical() as u64, mode, translated_bus).map_err(GlueError::Plan)?;
    // SAFETY: dma.virt() points at DMA_SIZE bytes owned by `dma`,
    // which the returned struct keeps alive; bus is the plan's
    // SMMU-gated bus address.
    let dma_mmio = unsafe { DmaPtrMmio::new(dma.virt() as *mut u8, dma_plan.bus()) };

    Ok(HwWindows {
        regs,
        dma: dma_mmio,
        dma_plan,
        _core_map: core_map,
        _qscratch_map: qscratch_map,
        _dma: dma,
    })
}
