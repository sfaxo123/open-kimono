//! Payload v3 (P5 stage 1) — SMMU CB-bypass establishment for
//! SID 0x740, the DWC3 stream in `apps_smmu`.
//!
//! This is the operator-corrected P5 plan (2026-11-01): the
//! U-Boot CB-bypass recipe ONLY. There is no S2CR TYPE=BYPASS
//! attempt anywhere in this module — the v4 design's Phase 1/2
//! (BYPASS-first) is superseded, because the fork's own driver
//! carries the warning in source: on Qualcomm parts the
//! hypervisor converts an S2CR BYPASS write to FAULT
//! (`drivers/iommu/qcom-hyp-smmu.c`, `configure_smr_s2cr`).
//! What survives from the v4 design is its verification
//! discipline, applied to the CB recipe:
//!
//!   baseline (GFSR==0, layout cross-checked, two-scan gate
//!   agreement that SID 0x740 is NotMatched)
//!   -> selection (free SMR; free CB proven by the three-part
//!      test below)
//!   -> snapshot of every register the recipe will write
//!   -> program in U-Boot's order (CB first, SMR/S2CR last)
//!   -> every written register read back TWICE, both reads equal
//!      to the written value
//!   -> GFSR re-checked
//!   -> two fresh scans through the existing `SidGate` must both
//!      report `Translated { smr: i, context_bank: n }` for OUR
//!      (i, n)
//!   -> only then is a `VerifiedCbBypass` token constructed.
//!
//! Any failure after the first write restores the snapshot
//! (writes, double readback, and a two-scan NotMatched check);
//! a restore that does not read back is its own distinct error.
//! Nothing here attempts DMA: the token is the stage-1 exit,
//! consumed by a later wiring packet.
//!
//! Free-CB proof (operator discipline, stricter than U-Boot's
//! `alloc_cb` heuristic, which never looks at the S2CRs): bank
//! `n` is free iff (a) no S2CR with TYPE=TRANS references it,
//! (b) `CBARn.TYPE == 0` (the reset/unassigned encoding —
//! architecturally named S2_TRANS; U-Boot's heuristic treats the
//! same encoding as a candidate), and (c) `CB(n)_FSR == 0`.
//! `CB_SCTLR.M == 0` is NOT a freeness signal (the recipe itself
//! leaves M=0) and is never consulted for selection.
//!
//! Register facts and their sources:
//! - Globals, SMR/S2CR addressing and field decodes: the
//!   existing daemon constants (`smmu_probe.rs`, `sid_gate.rs`),
//!   cross-checked on device by probe-2 (IDR0=0x4c017e4c,
//!   IDR1=0x5000002d, 76 groups / 45 banks, GFSR=0).
//! - GR1/CBAR/CBA2R/CB addressing and the recipe sequence: the
//!   fork's U-Boot driver `drivers/iommu/qcom-hyp-smmu.c`
//!   (`ARM_SMMU_GR1_CBAR(n) = GR1 + 0x0 + 4n`,
//!   `ARM_SMMU_GR1_CBA2R(n) = GR1 + 0x800 + 4n`, VA64 = BIT(0),
//!   CB page offset `1 << (NUMPAGENDXB + 1)` pages, pgshift from
//!   IDR1 PAGESIZE). GR1 is page 1, so GR1 base = base +
//!   page_size.
//! - CB register offsets (SCTLR 0x00, TTBR0 0x20, TTBR1 0x28,
//!   TCR 0x30, MAIR0 0x38, MAIR1 0x3c, FSR 0x58): Linux
//!   `drivers/iommu/arm/arm-smmu/arm-smmu.h`, read from the local
//!   Linux tree; SCTLR/TTBR0 agree with the U-Boot driver.
//! - The apps_smmu DT region is 0x80000 bytes
//!   (`sdm845.dtsi: iommu@15000000 reg = <0 0x15000000 0
//!   0x80000>`, also the project's own DTB source); the layout
//!   check refuses a derived window larger than that.
//!
//! Adoption rulings (root, 2026-11-01) resolved the packet's
//! two divergences IN FAVOUR of the shipped U-Boot recipe:
//! - CB_SCTLR: `CbRecipe::cb_sctlr` defaults to
//!   `CbRecipe::SCTLR_UBOOT` (CFIE|CFRE|CFCFG = 0xe0). M
//!   (bit 0) is 0 in both the old and new default; the bits
//!   only enable context-fault reporting.
//! - CB_TTBR0/TTBR1/TCR/MAIR0/MAIR1 are ZEROED by the program
//!   sequence (U-Boot behaviour), under the same readback-x2
//!   discipline as the five recipe registers, and are part of
//!   the snapshot/restore set. The free-CB proof still does
//!   not inspect them.
//!
//! Honest limits, inherited from the probe layer: two scans
//! through a systematically wrong register map agree with each
//! other; the layout check (IDR counts must equal the pinned
//! 76/45 and the window must fit the DT region) plus the
//! probe-2 pmOS cross-check are the guards, and the first
//! silicon readback remains a human cross-check before DMA.

use crate::physmap_plan::DmaMode;
use crate::sid_gate::{
    GateError, SidObservation, APPS_SMMU_CONTEXT_BANKS, APPS_SMMU_SMR_COUNT, DWC3_SID,
    S2CR_TYPE_TRANS,
};
use crate::smmu_probe::{
    dump_globals, s2cr_addr, smr_addr, Read32, SmmuProbe,
};
use crate::wiring::{probe_twice_and_verify, WiringError};

/// `apps_smmu` base (re-exported here for the glue's window
/// plan; the pinned value lives in `sid_gate`).
pub use crate::sid_gate::APPS_SMMU_BASE;

/// Length of the `iommu@15000000` DT region (`sdm845.dtsi`).
/// The full CB window must fit inside it.
pub const APPS_SMMU_DT_LEN: u64 = 0x8_0000;

/// Offset of CBAR0 within GR1 (`qcom-hyp-smmu.c`:
/// `ARM_SMMU_GR1_CBAR(n) (0x0 + ((n) << 2))`).
pub const GR1_CBAR_BASE_OFF: u64 = 0x000;
/// Offset of CBA2R0 within GR1 (`ARM_SMMU_GR1_CBA2R(n)
/// (0x800 + ((n) << 2))`).
pub const GR1_CBA2R_BASE_OFF: u64 = 0x800;
/// CBAR TYPE field position (bits [17:16]).
pub const CBAR_TYPE_SHIFT: u32 = 16;
/// CBAR TYPE encoding 0: stage-2-only bank. On an unused bank
/// this is the reset value, which is why the free-CB proof
/// reads it as "unassigned" (see module docs).
pub const CBAR_TYPE_S2_TRANS: u32 = 0;
/// CBAR TYPE encoding 1: stage-1 translation, stage-2 bypass —
/// the recipe's target configuration.
pub const CBAR_TYPE_S1_TRANS_S2_BYPASS: u32 = 1;
/// CBAR VMID field (bits [7:0], `qcom-hyp-smmu.c`).
pub const CBAR_VMID_MASK: u32 = 0xff;
/// CBA2R VA64 bit: select the 64-bit (48-bit VA) stage-1
/// format. The recipe SETS it and preserves every other bit
/// (CBA2R is a read-modify-write, like U-Boot's `gr1_setbits`).
pub const CBA2R_VA64: u32 = 1 << 0;

/// CB_SCTLR offset within a context-bank page.
pub const CB_SCTLR_OFF: u64 = 0x00;
/// CB_TTBR0 offset (snapshot/evidence only; never written).
pub const CB_TTBR0_OFF: u64 = 0x20;
/// CB_TTBR1 offset within the context bank page.
pub const CB_TTBR1_OFF: u64 = 0x28;
/// CB_TCR offset (snapshot/evidence only; never written).
pub const CB_TCR_OFF: u64 = 0x30;
/// CB_MAIR0 offset within the context bank page.
pub const CB_MAIR0_OFF: u64 = 0x38;
/// CB_MAIR1 offset within the context bank page.
pub const CB_MAIR1_OFF: u64 = 0x3c;
/// CB_FSR offset within a context-bank page
/// (Linux `arm-smmu.h`: `ARM_SMMU_CB_FSR 0x58`).
pub const CB_FSR_OFF: u64 = 0x58;

/// CB_SCTLR M bit (stage-1 translation enable for the bank).
/// The recipe's bypass behaviour comes from M == 0 paired with
/// CBAR TYPE = S1_TRANS_S2_BYPASS.
pub const SCTLR_M: u32 = 1 << 0;
/// CB_SCTLR CFRE bit (U-Boot recipe value component).
pub const SCTLR_CFRE: u32 = 1 << 5;
/// CB_SCTLR CFIE bit (U-Boot recipe value component).
pub const SCTLR_CFIE: u32 = 1 << 6;
/// CB_SCTLR CFCFG bit (U-Boot recipe value component).
pub const SCTLR_CFCFG: u32 = 1 << 7;

/// A source of 32-bit register writes. The Redox adapter backs
/// this with volatile writes over an RW physmap window; host
/// tests back it with a fake register file. Any closure
/// `FnMut(u64, u32)` is a source.
pub trait Write32 {
    fn write32(&mut self, addr: u64, value: u32);
}

impl<F: FnMut(u64, u32)> Write32 for F {
    fn write32(&mut self, addr: u64, value: u32) {
        self(addr, value)
    }
}

/// The recipe's tunable values. Everything else about the
/// programming sequence is fixed by the U-Boot recipe and the
/// verification discipline; these two are the points where the
/// operator's spec and the shipped U-Boot code diverge (see
/// module docs), so they are named, single, and flippable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CbRecipe {
    /// Value written to CB_SCTLR. Adopted default (root
    /// ruling, 2026-11-01): [`CbRecipe::SCTLR_UBOOT`] (M=0;
    /// fault reporting on). 0 remains available as
    /// [`CbRecipe::SCTLR_OFF`].
    pub cb_sctlr: u32,
    /// VMID written into CBAR (U-Boot writes 0).
    pub cbar_vmid: u8,
}

impl CbRecipe {
    /// The CB_SCTLR value the fork's U-Boot driver writes
    /// (CFIE | CFRE | CFCFG = 0xe0; M still 0).
    pub const SCTLR_UBOOT: u32 = SCTLR_CFCFG | SCTLR_CFIE | SCTLR_CFRE;

    /// CB_SCTLR = 0 (M=0, no fault reporting): the packet's
    /// original operator-spec value, kept as a named constant.
    pub const SCTLR_OFF: u32 = 0;

    /// The adopted recipe (root rulings, 2026-11-01):
    /// CB_SCTLR = SCTLR_UBOOT, VMID = 0.
    pub const fn operator() -> Self {
        CbRecipe {
            cb_sctlr: Self::SCTLR_UBOOT,
            cbar_vmid: 0,
        }
    }
}

impl Default for CbRecipe {
    fn default() -> Self {
        Self::operator()
    }
}

/// SMMU address layout derived from IDR0/IDR1 exactly the way
/// the U-Boot driver's probe derives it: pgshift from IDR1
/// PAGESIZE, CB page offset `1 << (NUMPAGENDXB + 1)` pages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SmmuLayout {
    pgshift: u32,
    num_smr: usize,
    num_cb: usize,
    cb_pg_offset: u64,
}

/// Raw IDR decode; no validation (see [`layout_checked`]).
pub fn layout_from_idr(idr0: u32, idr1: u32) -> SmmuLayout {
    let pgshift = if idr1 & (1 << 31) != 0 { 16 } else { 12 };
    let num_pagendxb = (idr1 >> 28) & 0x7;
    SmmuLayout {
        pgshift,
        num_smr: (idr0 & 0xff) as usize,
        num_cb: (idr1 & 0xff) as usize,
        cb_pg_offset: 1u64 << (num_pagendxb + 1),
    }
}

/// The layout this stage is willing to program against: the
/// IDR counts must equal the probe-2-pinned hardware facts
/// (76 stream-matching groups, 45 context banks) and the full
/// window (globals + GR1 + all CB pages) must fit the DT
/// region. Anything else means the register map is not the one
/// the constants describe — fail closed before any write.
pub fn layout_checked(idr0: u32, idr1: u32) -> Option<SmmuLayout> {
    let layout = layout_from_idr(idr0, idr1);
    if layout.num_smr != APPS_SMMU_SMR_COUNT
        || layout.num_cb != APPS_SMMU_CONTEXT_BANKS as usize
        || layout.window_len() > APPS_SMMU_DT_LEN
    {
        return None;
    }
    Some(layout)
}

impl SmmuLayout {
    /// Page size (1 << pgshift): 0x1000 on this hardware.
    pub fn page_size(&self) -> u64 {
        1u64 << self.pgshift
    }
    /// GR0 base: the SMMU register space itself. Both
    /// physmap windows the daemon opens (the RO probe window
    /// and the RW establishment window) start here; the
    /// verdict line's BASE field reports this value.
    pub fn gr0_base(&self) -> u64 {
        APPS_SMMU_BASE
    }
    /// GR1 base: GR1 is page 1 of the SMMU register space.
    pub fn gr1_base(&self) -> u64 {
        APPS_SMMU_BASE + self.page_size()
    }
    /// First context-bank page base.
    pub fn cb_base(&self) -> u64 {
        APPS_SMMU_BASE + self.cb_pg_offset * self.page_size()
    }
    /// Total window length covering globals, GR1 and every CB
    /// page: what the glue's RW window plan must span.
    pub fn window_len(&self) -> u64 {
        self.cb_base() - APPS_SMMU_BASE + self.num_cb as u64 * self.page_size()
    }
    /// Stream-matching group count from IDR0.
    pub fn num_smr(&self) -> usize {
        self.num_smr
    }
    /// Context-bank count from IDR1.
    pub fn num_cb(&self) -> usize {
        self.num_cb
    }
    /// Absolute address of CBAR`n`, or `None` out of range.
    pub fn cbar_addr(&self, n: usize) -> Option<u64> {
        if n >= self.num_cb {
            return None;
        }
        self.gr1_base()
            .checked_add(GR1_CBAR_BASE_OFF + 4 * n as u64)
    }
    /// Absolute address of CBA2R`n`, or `None` out of range.
    pub fn cba2r_addr(&self, n: usize) -> Option<u64> {
        if n >= self.num_cb {
            return None;
        }
        self.gr1_base()
            .checked_add(GR1_CBA2R_BASE_OFF + 4 * n as u64)
    }
    /// Absolute address of the register at `off` in CB page
    /// `n`, or `None` out of range (`off` must lie inside the
    /// page).
    pub fn cb_addr(&self, n: usize, off: u64) -> Option<u64> {
        if n >= self.num_cb || off >= self.page_size() {
            return None;
        }
        self.cb_base()
            .checked_add(n as u64 * self.page_size() + off)
    }
}

/// The SMR value that matches exactly `sid` and nothing else:
/// VALID set, MASK = 0x0000 (all StreamID bits compared —
/// the v4 discipline), ID = `sid`. EXIDVALID lives in S2CR
/// and stays 0 there (see [`s2cr_value_trans`]).
pub fn smr_value_exact(sid: u16) -> u32 {
    (1 << 31) | (sid as u32 & 0xffff)
}

/// The S2CR value routing a stream to context bank `cbndx`
/// for translation: TYPE = TRANS, CBNDX = `cbndx`, and
/// PRIVCFG / INSTCFG / EXIDVALID all 0 — the exact value
/// U-Boot's `configure_smr_s2cr` writes.
pub fn s2cr_value_trans(cbndx: usize) -> u32 {
    ((S2CR_TYPE_TRANS & 0x3) << 16) | (cbndx as u32 & 0xff)
}

/// CBAR TYPE field of a raw CBAR value.
pub fn cbar_type(cbar: u32) -> u32 {
    (cbar >> CBAR_TYPE_SHIFT) & 0x3
}

/// The CBAR value for the recipe: TYPE becomes
/// S1_TRANS_S2_BYPASS and VMID becomes `vmid`; every other
/// bit of the existing value is preserved (the bank may carry
/// implementation-defined state the recipe does not own).
pub fn cbar_value_bypass(old: u32, vmid: u8) -> u32 {
    (old & !(0x3 << CBAR_TYPE_SHIFT) & !CBAR_VMID_MASK)
        | (CBAR_TYPE_S1_TRANS_S2_BYPASS << CBAR_TYPE_SHIFT)
        | (vmid as u32 & CBAR_VMID_MASK)
}

/// The CBA2R value for the recipe: VA64 set (48-bit VA),
/// every other bit — including VMID16 — preserved. Mirrors
/// U-Boot's `gr1_setbits(CBA2R, VA64)`.
pub fn cba2r_value_48(old: u32) -> u32 {
    old | CBA2R_VA64
}

/// Context banks referenced by any LIVE stream with
/// TYPE=TRANS. Two conditions, both required: the SMR must be
/// VALID (an S2CR is don't-care state until its SMR is valid —
/// an unprogrammed S2CR reads TYPE=TRANS/CBNDX=0, which must
/// NOT count as a reference to bank 0), and the S2CR TYPE
/// must be TRANS (BYPASS and FAULT encodings ignore CBNDX
/// architecturally; the v4 discipline is explicit on this).
pub fn referenced_by_trans(
    smrs: &[u32],
    s2crs: &[u32],
    num_cb: usize,
) -> [bool; APPS_SMMU_CONTEXT_BANKS as usize] {
    let mut refs = [false; APPS_SMMU_CONTEXT_BANKS as usize];
    for (i, &s2cr) in s2crs.iter().enumerate() {
        let live = smrs.get(i).map_or(false, |&smr| smr & (1 << 31) != 0);
        if live && (s2cr >> 16) & 0x3 == S2CR_TYPE_TRANS {
            let cbndx = (s2cr & 0xff) as usize;
            if cbndx < num_cb && cbndx < APPS_SMMU_CONTEXT_BANKS as usize {
                refs[cbndx] = true;
            }
        }
    }
    refs
}

/// Lowest free SMR index: the first whose VALID bit is clear.
pub fn select_free_smr(smrs: &[u32]) -> Option<usize> {
    smrs.iter().position(|&v| v & (1 << 31) == 0)
}

/// Lowest free context bank under the three-part proof:
/// not referenced by any TRANS S2CR, CBAR TYPE == 0 (reset /
/// unassigned encoding), and CB FSR == 0. SCTLR.M is
/// deliberately NOT consulted (see module docs).
pub fn select_free_cb(
    smrs: &[u32],
    s2crs: &[u32],
    cbars: &[u32],
    fsrs: &[u32],
    num_cb: usize,
) -> Option<usize> {
    let refs = referenced_by_trans(smrs, s2crs, num_cb);
    (0..num_cb).find(|&n| !refs[n] && cbar_type(cbars[n]) == CBAR_TYPE_S2_TRANS && fsrs[n] == 0)
}

/// The registers the recipe writes, snapshotted before the
/// first write so any later failure can restore them: the
/// five recipe registers plus CB_TTBR0/TTBR1/TCR/MAIR0/MAIR1,
/// which the program sequence zeroes (root ruling,
/// 2026-11-01).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CbSnapshot {
    pub smr: u32,
    pub s2cr: u32,
    pub cbar: u32,
    pub cba2r: u32,
    pub cb_sctlr: u32,
    pub cb_ttbr0: u32,
    pub cb_ttbr1: u32,
    pub cb_tcr: u32,
    pub cb_mair0: u32,
    pub cb_mair1: u32,
}

/// Which recipe register a readback concerned (log/test
/// precision; addresses alone would force a human to re-derive
/// the register map).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegName {
    Smr,
    S2cr,
    Cbar,
    Cba2r,
    CbSctlr,
    CbTtbr0,
    CbTtbr1,
    CbTcr,
    CbMair0,
    CbMair1,
}

/// A double-readback disagreement: the register was read
/// twice; `first`/`second` are the two reads, at least one of
/// which differs from `expected`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadbackFailure {
    pub reg: RegName,
    pub addr: u64,
    pub expected: u32,
    pub first: u32,
    pub second: u32,
}

/// Every way CB-bypass establishment fails. All variants are
/// fail-closed: no token exists on any of these paths, and on
/// every path after the first write a restore was attempted
/// (a failed restore replaces the original error with
/// `RestoreFailed` / `RestoreScanMismatch` / `RestoreProbe`,
/// because a machine that could not put the SMMU back is the
/// more important fact).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CbBypassError {
    /// IDR counts/window disagree with the pinned hardware
    /// facts; the register map is not trusted. No writes.
    LayoutMismatch { idr0: u32, idr1: u32 },
    /// GFSR != 0 before programming. No writes.
    GlobalFault { gfsr: u32, gfsynr0: u32, gfsynr1: u32, gfsynr2: u32 },
    /// The baseline two-scan protocol itself failed.
    BaselineProbe(WiringError),
    /// Baseline scans agreed SID 0x740 is already matched (or
    /// faulted): not ours to disturb. No writes.
    BaselineMatched { obs: SidObservation },
    /// Every SMR is VALID. No writes.
    NoFreeSmr,
    /// No bank passes the three-part free proof. No writes.
    NoFreeCb,
    /// A written register did not read back twice as written;
    /// the snapshot was restored and verified. `smr`/`cb` are
    /// the indices the recipe had selected (verdict-line
    /// plumbing only; the failure itself is unchanged).
    ReadbackMismatch {
        smr: usize,
        cb: usize,
        failure: ReadbackFailure,
    },
    /// GFSR became nonzero after programming; the snapshot
    /// was restored and verified.
    FaultAfterProgram {
        smr: usize,
        cb: usize,
        gfsr: u32,
        gfsynr0: u32,
        gfsynr1: u32,
        gfsynr2: u32,
    },
    /// The post-program two-scan protocol itself failed; the
    /// snapshot was restored and verified.
    VerifyProbe {
        smr: usize,
        cb: usize,
        source: WiringError,
    },
    /// Post-program scans ran but did not both report our
    /// (smr, cb) Translated pair; the snapshot was restored
    /// and verified.
    VerifyMismatch {
        smr: usize,
        cb: usize,
        first: SidObservation,
        second: SidObservation,
    },
    /// A restore write did not read back as the snapshot.
    RestoreFailed {
        smr: usize,
        cb: usize,
        failure: ReadbackFailure,
    },
    /// The restore-verification scan protocol itself failed.
    RestoreProbe {
        smr: usize,
        cb: usize,
        source: WiringError,
    },
    /// Restore readbacks agreed but the scans do not report
    /// NotMatched again.
    RestoreScanMismatch {
        smr: usize,
        cb: usize,
        first: SidObservation,
        second: SidObservation,
    },
}

impl CbBypassError {
    /// The (SMR, CB) indices the recipe had selected when this
    /// failure occurred, or None when the failure preceded
    /// selection (layout, global fault, baseline, no-free-*).
    /// Verdict-line plumbing: one attended cycle must name
    /// the bank a fix has to reason about.
    pub fn tried(&self) -> Option<(usize, usize)> {
        match self {
            CbBypassError::ReadbackMismatch { smr, cb, .. }
            | CbBypassError::FaultAfterProgram { smr, cb, .. }
            | CbBypassError::VerifyProbe { smr, cb, .. }
            | CbBypassError::VerifyMismatch { smr, cb, .. }
            | CbBypassError::RestoreFailed { smr, cb, .. }
            | CbBypassError::RestoreProbe { smr, cb, .. }
            | CbBypassError::RestoreScanMismatch { smr, cb, .. } => Some((*smr, *cb)),
            _ => None,
        }
    }
}

/// The stage-1 exit token: proof that SID 0x740's stream was
/// established by THIS run of the CB-bypass recipe and
/// verified twice. Fields are private; the only constructor
/// is `establish_cb_bypass` after its full verification
/// sequence. `*_value` fields record the SECOND readback of
/// each register (the evidence values, per the v4 token
/// discipline), which equal the written values by
/// construction of the readback check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerifiedCbBypass {
    smr: usize,
    context_bank: usize,
    smr_value: u32,
    s2cr_value: u32,
    cbar_value: u32,
    cba2r_value: u32,
    cb_sctlr_value: u32,
}

impl VerifiedCbBypass {
    /// The SMR index programmed for SID 0x740.
    pub fn smr(&self) -> usize {
        self.smr
    }
    /// The context bank programmed for SID 0x740.
    pub fn context_bank(&self) -> usize {
        self.context_bank
    }
    /// Second-readback SMR value.
    pub fn smr_value(&self) -> u32 {
        self.smr_value
    }
    /// Second-readback S2CR value.
    pub fn s2cr_value(&self) -> u32 {
        self.s2cr_value
    }
    /// Second-readback CBAR value.
    pub fn cbar_value(&self) -> u32 {
        self.cbar_value
    }
    /// Second-readback CBA2R value.
    pub fn cba2r_value(&self) -> u32 {
        self.cba2r_value
    }
    /// Second-readback CB_SCTLR value.
    pub fn cb_sctlr_value(&self) -> u32 {
        self.cb_sctlr_value
    }
    /// One panel-capturable log line, mirroring the v4
    /// design's token log format.
    pub fn log_line(&self) -> String {
        format!(
            "SID 0x740 CB-BYPASS VERIFIED SMR={} CB={} SMR={:#010x} S2CR={:#010x} CBAR={:#010x} CBA2R={:#010x} CB_SCTLR={:#010x}",
            self.smr,
            self.context_bank,
            self.smr_value,
            self.s2cr_value,
            self.cbar_value,
            self.cba2r_value,
            self.cb_sctlr_value,
        )
    }
}

/// The SMMU globals a stage-1 verdict line reports. A field
/// is None when this outcome's path never read that register;
/// the line prints `-` for it rather than a fabricated value.
/// `base` is the GR0 phys base both daemon windows opened at
/// (the layout derivation's root). `gfar` / `gfsynr0` /
/// `gfsynr1` / `gfsynr2` are verdict-time reads: the glue fills them at
/// failure time, and they print only on ABORT / FAILED lines
/// whose GFSR is nonzero - they say WHICH stream faulted, so
/// our fault is not confused with an unrelated client's.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VerdictFacts {
    pub idr0: Option<u32>,
    pub idr1: Option<u32>,
    pub gfsr: Option<u32>,
    pub scr0: Option<u32>,
    pub base: Option<u64>,
    pub gfar: Option<u32>,
    pub gfsynr0: Option<u32>,
    pub gfsynr1: Option<u32>,
    pub gfsynr2: Option<u32>,
}

/// The non-VERIFIED stage-1 outcomes. The VERIFIED outcome
/// needs no verdict formatting: the token's `log_line`
/// already prints every value it programmed.
pub enum Stage1Verdict<'a> {
    /// The pinned-facts pre-check refused the register
    /// layout before any window was opened read/write.
    Refused,
    /// The daemon could not start the attempt (a window
    /// failed to open); the payload is the opener's error
    /// text.
    Aborted(&'a str),
    /// The establishment machine ran and failed fail-closed.
    Failed(&'a CbBypassError),
}

/// Format an optional register value for a verdict line:
/// `0x........` when read, `-` when never read on this path.
fn verdict_hex(value: Option<u32>) -> String {
    match value {
        Some(v) => format!("{v:#010x}"),
        None => "-".to_string(),
    }
}

/// Same discipline for the 64-bit BASE field.
fn verdict_hex64(value: Option<u64>) -> String {
    match value {
        Some(v) => format!("{v:#010x}"),
        None => "-".to_string(),
    }
}

/// The one self-contained verdict line for a non-VERIFIED
/// stage-1 outcome (without the daemon's `usbgadgetd: `
/// prefix; the glue adds it). Every line carries the raw
/// values the outcome judged - IDR0, IDR1, GFSR, SCR0 (the
/// SMMU's own global config: it separates a powered-off or
/// TZ-trapped register block, whose IDR reads come back 0
/// or all-ones with SCR0 implausible, from a genuinely
/// different chip) and BASE (the GR0 phys base the daemon
/// actually used, so a wrong-base probe is visible in the
/// line) when read, plus the SMR/CB indices tried (or `-`
/// when the failure preceded selection) and the reason, so
/// a single attended cycle tells the next fix without
/// another cycle. When an ABORT / FAILED line's GFSR is
/// nonzero it also carries the verdict-time fault block
/// (GFAR / GFSYNR0 / GFSYNR1 / GFSYNR2): which stream faulted.
/// Values carried by the error itself (a layout mismatch's
/// IDR pair, a global fault's GFSR and GFSYNR values) are the
/// ones the machine actually judged and win over the
/// caller's facts.
pub fn verdict_line(facts: VerdictFacts, verdict: &Stage1Verdict<'_>) -> String {
    let mut idr0 = facts.idr0;
    let mut idr1 = facts.idr1;
    let mut gfsr = facts.gfsr;
    let mut gfsynr0 = facts.gfsynr0;
    let mut gfsynr1 = facts.gfsynr1;
    let mut gfsynr2 = facts.gfsynr2;
    let mut tried: Option<(usize, usize)> = None;
    if let Stage1Verdict::Failed(err) = verdict {
        match err {
            CbBypassError::LayoutMismatch {
                idr0: e0,
                idr1: e1,
            } => {
                idr0 = Some(*e0);
                idr1 = Some(*e1);
            }
            CbBypassError::GlobalFault {
                gfsr: g,
                gfsynr0: s0,
                gfsynr1: s1,
                gfsynr2: s2,
            }
            | CbBypassError::FaultAfterProgram {
                gfsr: g,
                gfsynr0: s0,
                gfsynr1: s1,
                gfsynr2: s2,
                ..
            } => {
                gfsr = Some(*g);
                gfsynr0 = Some(*s0);
                gfsynr1 = Some(*s1);
                gfsynr2 = Some(*s2);
            }
            _ => {}
        }
        tried = err.tried();
    }
    let (smr, cb) = match tried {
        Some((s, c)) => (s.to_string(), c.to_string()),
        None => ("-".to_string(), "-".to_string()),
    };
    let head = format!(
        "IDR0={} IDR1={} GFSR={} SCR0={} BASE={} SMR={} CB={}",
        verdict_hex(idr0),
        verdict_hex(idr1),
        verdict_hex(gfsr),
        verdict_hex(facts.scr0),
        verdict_hex64(facts.base),
        smr,
        cb
    );
    // The fault block names WHICH stream faulted; it prints
    // only where a fault is actually being reported (ABORT /
    // FAILED with a nonzero GFSR). GFAR comes from the
    // caller's verdict-time read - no error variant carries
    // it - and prints `-` on paths that could not read it.
    let fault_tail = match verdict {
        Stage1Verdict::Aborted(_) | Stage1Verdict::Failed(_) => match gfsr {
            Some(g) if g != 0 => format!(
                " GFAR={} GFSYNR0={} GFSYNR1={} GFSYNR2={}",
                verdict_hex(facts.gfar),
                verdict_hex(gfsynr0),
                verdict_hex(gfsynr1),
                verdict_hex(gfsynr2)
            ),
            _ => String::new(),
        },
        Stage1Verdict::Refused => String::new(),
    };
    match verdict {
        Stage1Verdict::Refused => format!(
            "CB-BYPASS REFUSED: {head} IDR counts/window disagree with the pinned SDM845 facts; no writes"
        ),
        Stage1Verdict::Aborted(msg) => {
            format!("CB-BYPASS ABORT: {head}{fault_tail} {msg}; no writes, no DMA")
        }
        Stage1Verdict::Failed(err) => {
            format!("CB-BYPASS FAILED (fail-closed): {head}{fault_tail} {err:?}; no DMA attempted")
        }
    }
}

/// Read `addr` twice; both reads must equal `expected`.
/// Returns the second read (the evidence value).
fn readback2<B: Read32>(
    bus: &mut B,
    reg: RegName,
    addr: u64,
    expected: u32,
) -> Result<u32, ReadbackFailure> {
    let first = bus.read32(addr);
    let second = bus.read32(addr);
    if first == expected && second == expected {
        Ok(second)
    } else {
        Err(ReadbackFailure {
            reg,
            addr,
            expected,
            first,
            second,
        })
    }
}

/// One two-scan gate sequence over a fresh probe fed by
/// `bus` (the wiring layer's exact composition: scan ->
/// establish -> scan -> verify). The probe owns a closure
/// borrowing `bus`; it is dropped before this returns, so the
/// caller can write through `bus` immediately afterwards.
fn scan_pair<B: Read32>(
    bus: &mut B,
) -> Result<crate::wiring::LogOnlyReport, WiringError> {
    let mut probe = SmmuProbe::new(|addr: u64| bus.read32(addr));
    let report = probe_twice_and_verify(&mut probe);
    drop(probe);
    report
}

/// The written registers' second-readback values.
struct Readbacks {
    smr: u32,
    s2cr: u32,
    cbar: u32,
    cba2r: u32,
    cb_sctlr: u32,
    cb_ttbr0: u32,
    cb_ttbr1: u32,
    cb_tcr: u32,
    cb_mair0: u32,
    cb_mair1: u32,
}

/// Double-readback of all written registers: the five
/// recipe registers in the v4 Phase-4 listing order (SMR,
/// S2CR, CBAR, CBA2R, CB_SCTLR), then the zeroed translation
/// registers (TTBR0, TTBR1, TCR, MAIR0, MAIR1).
#[allow(clippy::too_many_arguments)]
fn readback_all<B: Read32>(
    bus: &mut B,
    layout: &SmmuLayout,
    smr_i: usize,
    cb_n: usize,
    want: &Readbacks,
) -> Result<Readbacks, ReadbackFailure> {
    // In-range by construction (indices came from the checked
    // layout and the probe-pinned SMR count).
    let smr = readback2(bus, RegName::Smr, smr_addr(smr_i).unwrap(), want.smr)?;
    let s2cr = readback2(bus, RegName::S2cr, s2cr_addr(smr_i).unwrap(), want.s2cr)?;
    let cbar = readback2(bus, RegName::Cbar, layout.cbar_addr(cb_n).unwrap(), want.cbar)?;
    let cba2r = readback2(
        bus,
        RegName::Cba2r,
        layout.cba2r_addr(cb_n).unwrap(),
        want.cba2r,
    )?;
    let cb_sctlr = readback2(
        bus,
        RegName::CbSctlr,
        layout.cb_addr(cb_n, CB_SCTLR_OFF).unwrap(),
        want.cb_sctlr,
    )?;
    let cb_ttbr0 = readback2(
        bus,
        RegName::CbTtbr0,
        layout.cb_addr(cb_n, CB_TTBR0_OFF).unwrap(),
        want.cb_ttbr0,
    )?;
    let cb_ttbr1 = readback2(
        bus,
        RegName::CbTtbr1,
        layout.cb_addr(cb_n, CB_TTBR1_OFF).unwrap(),
        want.cb_ttbr1,
    )?;
    let cb_tcr = readback2(
        bus,
        RegName::CbTcr,
        layout.cb_addr(cb_n, CB_TCR_OFF).unwrap(),
        want.cb_tcr,
    )?;
    let cb_mair0 = readback2(
        bus,
        RegName::CbMair0,
        layout.cb_addr(cb_n, CB_MAIR0_OFF).unwrap(),
        want.cb_mair0,
    )?;
    let cb_mair1 = readback2(
        bus,
        RegName::CbMair1,
        layout.cb_addr(cb_n, CB_MAIR1_OFF).unwrap(),
        want.cb_mair1,
    )?;
    Ok(Readbacks {
        smr,
        s2cr,
        cbar,
        cba2r,
        cb_sctlr,
        cb_ttbr0,
        cb_ttbr1,
        cb_tcr,
        cb_mair0,
        cb_mair1,
    })
}

/// Put the snapshot back (stream routing first, so SID 0x740
/// stops reaching the bank before the bank is un-configured),
/// then prove it: double readback of all five registers and a
/// two-scan gate sequence that must report NotMatched again.
fn restore<B: Read32 + Write32>(
    bus: &mut B,
    layout: &SmmuLayout,
    smr_i: usize,
    cb_n: usize,
    snap: &CbSnapshot,
) -> Result<(), CbBypassError> {
    bus.write32(s2cr_addr(smr_i).unwrap(), snap.s2cr);
    bus.write32(smr_addr(smr_i).unwrap(), snap.smr);
    bus.write32(layout.cb_addr(cb_n, CB_SCTLR_OFF).unwrap(), snap.cb_sctlr);
    bus.write32(layout.cb_addr(cb_n, CB_TTBR0_OFF).unwrap(), snap.cb_ttbr0);
    bus.write32(layout.cb_addr(cb_n, CB_TTBR1_OFF).unwrap(), snap.cb_ttbr1);
    bus.write32(layout.cb_addr(cb_n, CB_TCR_OFF).unwrap(), snap.cb_tcr);
    bus.write32(layout.cb_addr(cb_n, CB_MAIR0_OFF).unwrap(), snap.cb_mair0);
    bus.write32(layout.cb_addr(cb_n, CB_MAIR1_OFF).unwrap(), snap.cb_mair1);
    bus.write32(layout.cba2r_addr(cb_n).unwrap(), snap.cba2r);
    bus.write32(layout.cbar_addr(cb_n).unwrap(), snap.cbar);
    let want = Readbacks {
        smr: snap.smr,
        s2cr: snap.s2cr,
        cbar: snap.cbar,
        cba2r: snap.cba2r,
        cb_sctlr: snap.cb_sctlr,
        cb_ttbr0: snap.cb_ttbr0,
        cb_ttbr1: snap.cb_ttbr1,
        cb_tcr: snap.cb_tcr,
        cb_mair0: snap.cb_mair0,
        cb_mair1: snap.cb_mair1,
    };
    readback_all(bus, layout, smr_i, cb_n, &want)
        .map_err(|failure| CbBypassError::RestoreFailed {
            smr: smr_i,
            cb: cb_n,
            failure,
        })?;
    let report = scan_pair(bus).map_err(|source| CbBypassError::RestoreProbe {
        smr: smr_i,
        cb: cb_n,
        source,
    })?;
    if report.mode() != Err(GateError::SidNotMatched)
        || report.first().obs() != SidObservation::NotMatched
        || report.second().obs() != SidObservation::NotMatched
    {
        return Err(CbBypassError::RestoreScanMismatch {
            smr: smr_i,
            cb: cb_n,
            first: report.first().obs(),
            second: report.second().obs(),
        });
    }
    Ok(())
}

/// Establish SID 0x740 via the U-Boot CB-bypass recipe, with
/// the full verification discipline in the module docs. The
/// bus is the only hardware touch; no DMA is attempted here
/// or implied by the returned token.
pub fn establish_cb_bypass<B: Read32 + Write32>(
    bus: &mut B,
    recipe: &CbRecipe,
) -> Result<VerifiedCbBypass, CbBypassError> {
    // Phase 0 — globals, layout cross-check, baseline scans.
    let globals = dump_globals(bus);
    if globals.gfsr != 0 {
        return Err(CbBypassError::GlobalFault {
            gfsr: globals.gfsr,
            gfsynr0: globals.gfsynr0,
            gfsynr1: globals.gfsynr1,
            gfsynr2: globals.gfsynr2,
        });
    }
    let layout = layout_checked(globals.idr0, globals.idr1).ok_or(CbBypassError::LayoutMismatch {
        idr0: globals.idr0,
        idr1: globals.idr1,
    })?;
    let baseline = scan_pair(bus).map_err(CbBypassError::BaselineProbe)?;
    if baseline.mode() != Err(GateError::SidNotMatched) {
        return Err(CbBypassError::BaselineMatched {
            obs: baseline.second().obs(),
        });
    }

    // Selection — reads only. Full register arrays first, so
    // the free proofs run over one consistent read set.
    let smrs: Vec<u32> = (0..layout.num_smr)
        .map(|i| bus.read32(smr_addr(i).unwrap()))
        .collect();
    let s2crs: Vec<u32> = (0..layout.num_smr)
        .map(|i| bus.read32(s2cr_addr(i).unwrap()))
        .collect();
    let cbars: Vec<u32> = (0..layout.num_cb)
        .map(|n| bus.read32(layout.cbar_addr(n).unwrap()))
        .collect();
    let fsrs: Vec<u32> = (0..layout.num_cb)
        .map(|n| bus.read32(layout.cb_addr(n, CB_FSR_OFF).unwrap()))
        .collect();
    let smr_i = select_free_smr(&smrs).ok_or(CbBypassError::NoFreeSmr)?;
    let cb_n = select_free_cb(&smrs, &s2crs, &cbars, &fsrs, layout.num_cb)
        .ok_or(CbBypassError::NoFreeCb)?;

    // Snapshot every register the recipe will write (the two
    // not already in the arrays), before the first write.
    let snap = CbSnapshot {
        smr: smrs[smr_i],
        s2cr: s2crs[smr_i],
        cbar: cbars[cb_n],
        cba2r: bus.read32(layout.cba2r_addr(cb_n).unwrap()),
        cb_sctlr: bus.read32(layout.cb_addr(cb_n, CB_SCTLR_OFF).unwrap()),
        cb_ttbr0: bus.read32(layout.cb_addr(cb_n, CB_TTBR0_OFF).unwrap()),
        cb_ttbr1: bus.read32(layout.cb_addr(cb_n, CB_TTBR1_OFF).unwrap()),
        cb_tcr: bus.read32(layout.cb_addr(cb_n, CB_TCR_OFF).unwrap()),
        cb_mair0: bus.read32(layout.cb_addr(cb_n, CB_MAIR0_OFF).unwrap()),
        cb_mair1: bus.read32(layout.cb_addr(cb_n, CB_MAIR1_OFF).unwrap()),
    };

    // Fault re-check immediately before programming (v4:
    // GFSR checked before and after each phase).
    let globals = dump_globals(bus);
    if globals.gfsr != 0 {
        return Err(CbBypassError::GlobalFault {
            gfsr: globals.gfsr,
            gfsynr0: globals.gfsynr0,
            gfsynr1: globals.gfsynr1,
            gfsynr2: globals.gfsynr2,
        });
    }

    // Program, in U-Boot's order: the bank is configured
    // first; the stream goes live (SMR/S2CR) last.
    let cbar_new = cbar_value_bypass(snap.cbar, recipe.cbar_vmid);
    let cba2r_new = cba2r_value_48(snap.cba2r);
    let smr_new = smr_value_exact(DWC3_SID);
    let s2cr_new = s2cr_value_trans(cb_n);
    bus.write32(layout.cbar_addr(cb_n).unwrap(), cbar_new);
    bus.write32(layout.cba2r_addr(cb_n).unwrap(), cba2r_new);
    // Zero the translation registers, per the U-Boot recipe
    // (root ruling, 2026-11-01): inert while SCTLR.M=0, and
    // no stale ABL state survives into stage 2.
    bus.write32(layout.cb_addr(cb_n, CB_TTBR0_OFF).unwrap(), 0);
    bus.write32(layout.cb_addr(cb_n, CB_TTBR1_OFF).unwrap(), 0);
    bus.write32(layout.cb_addr(cb_n, CB_TCR_OFF).unwrap(), 0);
    bus.write32(layout.cb_addr(cb_n, CB_MAIR0_OFF).unwrap(), 0);
    bus.write32(layout.cb_addr(cb_n, CB_MAIR1_OFF).unwrap(), 0);
    bus.write32(layout.cb_addr(cb_n, CB_SCTLR_OFF).unwrap(), recipe.cb_sctlr);
    bus.write32(smr_addr(smr_i).unwrap(), smr_new);
    bus.write32(s2cr_addr(smr_i).unwrap(), s2cr_new);

    // Verify 1 — double readback of all five registers.
    let want = Readbacks {
        smr: smr_new,
        s2cr: s2cr_new,
        cbar: cbar_new,
        cba2r: cba2r_new,
        cb_sctlr: recipe.cb_sctlr,
        cb_ttbr0: 0,
        cb_ttbr1: 0,
        cb_tcr: 0,
        cb_mair0: 0,
        cb_mair1: 0,
    };
    let readbacks = match readback_all(bus, &layout, smr_i, cb_n, &want) {
        Ok(rb) => rb,
        Err(f) => {
            restore(bus, &layout, smr_i, cb_n, &snap)?;
            return Err(CbBypassError::ReadbackMismatch {
                smr: smr_i,
                cb: cb_n,
                failure: f,
            });
        }
    };

    // Verify 2 — no global fault from the programming.
    let globals = dump_globals(bus);
    if globals.gfsr != 0 {
        let err = CbBypassError::FaultAfterProgram {
            smr: smr_i,
            cb: cb_n,
            gfsr: globals.gfsr,
            gfsynr0: globals.gfsynr0,
            gfsynr1: globals.gfsynr1,
            gfsynr2: globals.gfsynr2,
        };
        restore(bus, &layout, smr_i, cb_n, &snap)?;
        return Err(err);
    }

    // Verify 3 — two fresh scans must both report OUR
    // (smr, cb) pair as Translated, and the gate must agree.
    let verify = match scan_pair(bus) {
        Ok(v) => v,
        Err(e) => {
            restore(bus, &layout, smr_i, cb_n, &snap)?;
            return Err(CbBypassError::VerifyProbe {
                smr: smr_i,
                cb: cb_n,
                source: e,
            });
        }
    };
    let want_obs = SidObservation::Translated {
        smr: smr_i,
        context_bank: cb_n as u8,
    };
    if verify.first().obs() != want_obs
        || verify.second().obs() != want_obs
        || verify.mode() != Ok(DmaMode::Translated)
    {
        let err = CbBypassError::VerifyMismatch {
            smr: smr_i,
            cb: cb_n,
            first: verify.first().obs(),
            second: verify.second().obs(),
        };
        restore(bus, &layout, smr_i, cb_n, &snap)?;
        return Err(err);
    }

    Ok(VerifiedCbBypass {
        smr: smr_i,
        context_bank: cb_n,
        smr_value: readbacks.smr,
        s2cr_value: readbacks.s2cr,
        cbar_value: readbacks.cbar,
        cba2r_value: readbacks.cba2r,
        cb_sctlr_value: readbacks.cb_sctlr,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::smmu_probe::{global_addr, GFSR_OFF, IDR0_OFF, IDR1_OFF, IDR2_OFF, SCR0_OFF};
    use std::collections::{HashMap, VecDeque};

    /// Probe-2 hardware values (design doc section 2).
    const IDR0_HW: u32 = 0x4c01_7e4c;
    const IDR1_HW: u32 = 0x5000_002d;

    /// Fake SMMU register file over the machine's bus traits.
    /// Unprogrammed addresses read 0. A per-address FIFO
    /// script can override the next reads of one address
    /// (used to simulate readbacks that disagree with the
    /// stored value, scan-visible state that differs from the
    /// readback state, and faults that appear mid-sequence).
    #[derive(Default)]
    struct FakeBus {
        regs: HashMap<u64, u32>,
        writes: Vec<(u64, u32)>,
        read_script: HashMap<u64, VecDeque<u32>>,
    }

    impl FakeBus {
        /// Globals as read back by probe-2 on the real phone;
        /// every other register 0 (all SMRs invalid, all CBARs
        /// TYPE=0, all FSRs 0, GFSR=0).
        fn realistic() -> Self {
            let mut bus = FakeBus::default();
            bus.regs.insert(global_addr(SCR0_OFF), 0x002a_0406);
            bus.regs.insert(global_addr(IDR0_OFF), IDR0_HW);
            bus.regs.insert(global_addr(IDR1_OFF), IDR1_HW);
            bus.regs.insert(global_addr(IDR2_OFF), 0x0000_5555);
            bus
        }

        fn program(&mut self, index: usize, id: u16, mask: u16, ty: u32, cbndx: u8) {
            let smr = (1u32 << 31) | (((mask as u32) & 0x7fff) << 16) | id as u32;
            let s2cr = (ty << 16) | cbndx as u32;
            self.regs.insert(smr_addr(index).unwrap(), smr);
            self.regs.insert(s2cr_addr(index).unwrap(), s2cr);
        }

        fn script(&mut self, addr: u64, values: &[u32]) {
            self.read_script
                .insert(addr, values.iter().copied().collect());
        }

        fn stored(&self, addr: u64) -> u32 {
            *self.regs.get(&addr).unwrap_or(&0)
        }
    }

    impl Read32 for FakeBus {
        fn read32(&mut self, addr: u64) -> u32 {
            if let Some(queue) = self.read_script.get_mut(&addr) {
                if let Some(v) = queue.pop_front() {
                    return v;
                }
            }
            *self.regs.get(&addr).unwrap_or(&0)
        }
    }

    impl Write32 for FakeBus {
        fn write32(&mut self, addr: u64, value: u32) {
            self.writes.push((addr, value));
            self.regs.insert(addr, value);
        }
    }

    fn layout() -> SmmuLayout {
        layout_checked(IDR0_HW, IDR1_HW).unwrap()
    }

    #[test]
    fn layout_math_pinned_to_probe2() {
        let l = layout();
        assert_eq!(l.num_smr(), 76);
        assert_eq!(l.num_cb(), 45);
        assert_eq!(l.page_size(), 0x1000);
        assert_eq!(l.gr0_base(), 0x1500_0000);
        assert_eq!(l.gr1_base(), 0x1500_1000);
        assert_eq!(l.cb_base(), 0x1504_0000);
        assert_eq!(l.window_len(), 0x6d_000);
        assert_eq!(l.cbar_addr(0), Some(0x1500_1000));
        assert_eq!(l.cbar_addr(44), Some(0x1500_10b0));
        assert_eq!(l.cba2r_addr(0), Some(0x1500_1800));
        assert_eq!(l.cba2r_addr(44), Some(0x1500_18b0));
        assert_eq!(l.cb_addr(0, CB_SCTLR_OFF), Some(0x1504_0000));
        assert_eq!(l.cb_addr(44, CB_FSR_OFF), Some(0x1506_c058));
        assert_eq!(l.cbar_addr(45), None);
        assert_eq!(l.cba2r_addr(45), None);
        assert_eq!(l.cb_addr(45, CB_SCTLR_OFF), None);
        assert_eq!(l.cb_addr(0, 0x1000), None);
        // Count mismatches refuse the layout entirely.
        assert_eq!(layout_checked(IDR0_HW, 0x5000_002c), None); // 44 banks
        assert_eq!(layout_checked(0x4c01_7e4b, IDR1_HW), None); // 75 groups
    }

    #[test]
    fn value_builders_pinned() {
        assert_eq!(smr_value_exact(0x740), 0x8000_0740);
        assert_eq!(smr_value_exact(0x0000), 0x8000_0000);
        assert_eq!(s2cr_value_trans(3), 3);
        assert_eq!(s2cr_value_trans(0), 0);
        assert_eq!(cbar_type(0x0002_0000), 2);
        assert_eq!(cbar_type(0x0001_0000), 1);
        // TYPE and VMID are replaced; all other bits survive.
        assert_eq!(cbar_value_bypass(0xffff_ffff, 0), 0xfffd_ff00);
        assert_eq!(cbar_value_bypass(0, 0xab), 0x0001_00ab);
        // VA64 is set; everything else (incl. VMID16) survives.
        assert_eq!(cba2r_value_48(0x1234_0000), 0x1234_0001);
        assert_eq!(cba2r_value_48(0), 1);
    }

    #[test]
    fn free_cb_proof_is_three_part() {
        // SMR0 -> TRANS cb9 (reference blocks 9). SMR1 ->
        // BYPASS cb10 and SMR2 -> FAULT cb11: neither blocks
        // its CBNDX, because only TRANS references count.
        let mut smrs = [0u32; APPS_SMMU_SMR_COUNT];
        let mut s2crs = [0u32; APPS_SMMU_SMR_COUNT];
        smrs[0] = (1 << 31) | 0x100;
        s2crs[0] = (S2CR_TYPE_TRANS << 16) | 9;
        smrs[1] = (1 << 31) | 0x200;
        s2crs[1] = (1 << 16) | 10; // BYPASS
        smrs[2] = (1 << 31) | 0x300;
        s2crs[2] = (2 << 16) | 11; // FAULT
        let mut cbars = [0u32; APPS_SMMU_CONTEXT_BANKS as usize];
        let mut fsrs = [0u32; APPS_SMMU_CONTEXT_BANKS as usize];
        let refs = referenced_by_trans(&smrs, &s2crs, 45);
        assert!(refs[9]);
        assert!(!refs[10]);
        assert!(!refs[11]);
        // Regression pin: every OTHER index has an invalid
        // SMR and an all-zero S2CR (TYPE reads TRANS, CBNDX
        // reads 0). That must not count as a reference to
        // bank 0 — an S2CR is don't-care until its SMR is
        // VALID.
        assert!(!refs[0]);
        assert_eq!(select_free_cb(&smrs, &s2crs, &cbars, &fsrs, 45), Some(0));
        // Bank 0 blocked by TYPE != 0; bank 1 is next.
        cbars[0] = CBAR_TYPE_S1_TRANS_S2_BYPASS << CBAR_TYPE_SHIFT;
        assert_eq!(select_free_cb(&smrs, &s2crs, &cbars, &fsrs, 45), Some(1));
        // Bank 1 blocked by FSR != 0; bank 2 is next.
        fsrs[1] = 0x8;
        assert_eq!(select_free_cb(&smrs, &s2crs, &cbars, &fsrs, 45), Some(2));
        // SCTLR.M is never consulted: a bank whose SCTLR has
        // M set is still selectable if the three parts pass —
        // that state is not representable here precisely
        // because selection never reads SCTLR (documented in
        // the module header; the machine-level tests cover
        // the SCTLR snapshot/restore behaviour).
    }

    #[test]
    fn happy_path_issues_token() {
        let l = layout();
        let mut bus = FakeBus::realistic();
        // Two other streams: SMR5 -> TRANS cb7, SMR6 -> TRANS
        // cb2 (reference is cb2's ONLY block).
        bus.program(5, 0x100, 0, S2CR_TYPE_TRANS, 7);
        bus.program(6, 0x200, 0, S2CR_TYPE_TRANS, 2);
        // cb0 blocked by FSR, cb1 blocked by CBAR TYPE.
        bus.regs.insert(l.cb_addr(0, CB_FSR_OFF).unwrap(), 0x1);
        bus.regs
            .insert(l.cbar_addr(1).unwrap(), 2 << CBAR_TYPE_SHIFT);
        // cb3 is the lowest fully-free bank. Give it state the
        // recipe must preserve (CBAR bit 20, CBA2R high bits)
        // and an SCTLR the recipe must fully overwrite.
        bus.regs.insert(l.cbar_addr(3).unwrap(), 1 << 20);
        bus.regs.insert(l.cba2r_addr(3).unwrap(), 0x0007_0000);
        bus.regs.insert(l.cb_addr(3, CB_SCTLR_OFF).unwrap(), 0xffff_ffff);
        // Stale translation state the recipe must zero.
        bus.regs.insert(l.cb_addr(3, CB_TTBR0_OFF).unwrap(), 0xdead_beef);
        bus.regs.insert(l.cb_addr(3, CB_TCR_OFF).unwrap(), 0x1234_5678);

        let token = establish_cb_bypass(&mut bus, &CbRecipe::operator()).unwrap();
        assert_eq!(token.smr(), 0);
        assert_eq!(token.context_bank(), 3);
        assert_eq!(token.smr_value(), 0x8000_0740);
        assert_eq!(token.s2cr_value(), 3);
        assert_eq!(token.cbar_value(), 0x0011_0000);
        assert_eq!(token.cba2r_value(), 0x0007_0001);
        assert_eq!(token.cb_sctlr_value(), 0xe0);
        // Program order: bank first (CBAR, CBA2R, the five
        // zeroed translation registers, SCTLR), stream last
        // (SMR, S2CR) — and nothing else was written at all.
        assert_eq!(
            bus.writes,
            vec![
                (l.cbar_addr(3).unwrap(), 0x0011_0000),
                (l.cba2r_addr(3).unwrap(), 0x0007_0001),
                (l.cb_addr(3, CB_TTBR0_OFF).unwrap(), 0),
                (l.cb_addr(3, CB_TTBR1_OFF).unwrap(), 0),
                (l.cb_addr(3, CB_TCR_OFF).unwrap(), 0),
                (l.cb_addr(3, CB_MAIR0_OFF).unwrap(), 0),
                (l.cb_addr(3, CB_MAIR1_OFF).unwrap(), 0),
                (l.cb_addr(3, CB_SCTLR_OFF).unwrap(), 0xe0),
                (smr_addr(0).unwrap(), 0x8000_0740),
                (s2cr_addr(0).unwrap(), 3),
            ]
        );
        // Final state is the programmed state.
        assert_eq!(bus.stored(l.cb_addr(3, CB_SCTLR_OFF).unwrap()), 0xe0);
        assert_eq!(bus.stored(l.cb_addr(3, CB_TTBR0_OFF).unwrap()), 0);
        assert_eq!(bus.stored(l.cb_addr(3, CB_TCR_OFF).unwrap()), 0);
        assert_eq!(bus.stored(smr_addr(0).unwrap()), 0x8000_0740);
        assert!(token.log_line().contains("CB-BYPASS VERIFIED SMR=0 CB=3"));
    }

    #[test]
    fn off_sctlr_recipe_flips_the_sctlr_write() {
        // The adopted default is SCTLR_UBOOT (asserted by the
        // happy-path test); an explicit SCTLR_OFF recipe must
        // flip the write back to 0.
        let l = layout();
        let mut bus = FakeBus::realistic();
        let recipe = CbRecipe {
            cb_sctlr: CbRecipe::SCTLR_OFF,
            cbar_vmid: 0,
        };
        let token = establish_cb_bypass(&mut bus, &recipe).unwrap();
        assert_eq!(token.cb_sctlr_value(), 0);
        assert!(bus
            .writes
            .contains(&(l.cb_addr(0, CB_SCTLR_OFF).unwrap(), 0)));
    }

    #[test]
    fn all_banks_referenced_refuses_without_writes() {
        let mut bus = FakeBus::realistic();
        for i in 0..APPS_SMMU_CONTEXT_BANKS as usize {
            bus.program(i, 0x100 + i as u16, 0, S2CR_TYPE_TRANS, i as u8);
        }
        assert_eq!(
            establish_cb_bypass(&mut bus, &CbRecipe::operator()),
            Err(CbBypassError::NoFreeCb)
        );
        assert!(bus.writes.is_empty());
    }

    #[test]
    fn no_free_smr_refuses_without_writes() {
        let mut bus = FakeBus::realistic();
        for i in 0..APPS_SMMU_SMR_COUNT {
            bus.program(i, 0x100 + i as u16, 0, S2CR_TYPE_TRANS, 0);
        }
        assert_eq!(
            establish_cb_bypass(&mut bus, &CbRecipe::operator()),
            Err(CbBypassError::NoFreeSmr)
        );
        assert!(bus.writes.is_empty());
    }

    #[test]
    fn baseline_already_matched_refuses_without_writes() {
        let mut bus = FakeBus::realistic();
        bus.program(2, DWC3_SID, 0, 1, 0); // BYPASS already
        assert_eq!(
            establish_cb_bypass(&mut bus, &CbRecipe::operator()),
            Err(CbBypassError::BaselineMatched {
                obs: SidObservation::Bypass { smr: 2 }
            })
        );
        assert!(bus.writes.is_empty());
    }

    #[test]
    fn layout_mismatch_refuses_without_writes() {
        let mut bus = FakeBus::realistic();
        bus.regs.insert(global_addr(IDR1_OFF), 0x5000_002c); // 44 banks
        assert_eq!(
            establish_cb_bypass(&mut bus, &CbRecipe::operator()),
            Err(CbBypassError::LayoutMismatch {
                idr0: IDR0_HW,
                idr1: 0x5000_002c
            })
        );
        assert!(bus.writes.is_empty());
    }

    #[test]
    fn gfsr_nonzero_at_baseline_refuses_without_writes() {
        let mut bus = FakeBus::realistic();
        bus.regs.insert(global_addr(GFSR_OFF), 0x4);
        assert_eq!(
            establish_cb_bypass(&mut bus, &CbRecipe::operator()),
            Err(CbBypassError::GlobalFault {
                gfsr: 0x4,
                gfsynr0: 0,
                gfsynr1: 0,
                gfsynr2: 0
            })
        );
        assert!(bus.writes.is_empty());
    }

    #[test]
    fn readback_mismatch_restores_and_reports() {
        let l = layout();
        let mut bus = FakeBus::realistic();
        // Minimal fixture: free smr = 0, free cb = 0, snapshot
        // all zero. CBAR reads: selection sees the stored 0;
        // both readback reads lie.
        bus.script(l.cbar_addr(0).unwrap(), &[0, 0xbad, 0xbad]);
        assert_eq!(
            establish_cb_bypass(&mut bus, &CbRecipe::operator()),
            Err(CbBypassError::ReadbackMismatch {
                smr: 0,
                cb: 0,
                failure: ReadbackFailure {
                    reg: RegName::Cbar,
                    addr: l.cbar_addr(0).unwrap(),
                    expected: 0x0001_0000,
                    first: 0xbad,
                    second: 0xbad,
                }
            })
        );
        // Restore put every written register back to the
        // snapshot (all zero here).
        for addr in [
            smr_addr(0).unwrap(),
            s2cr_addr(0).unwrap(),
            l.cbar_addr(0).unwrap(),
            l.cba2r_addr(0).unwrap(),
            l.cb_addr(0, CB_SCTLR_OFF).unwrap(),
        ] {
            assert_eq!(bus.stored(addr), 0, "addr {addr:#x}");
        }
    }

    #[test]
    fn restore_failure_dominates_the_original_error() {
        let l = layout();
        let mut bus = FakeBus::realistic();
        // Trigger: CBAR readback lies (as above). During the
        // restore, the SMR readback lies too — the restore
        // failure is the reported error. SMR read sequence:
        // baseline scans x2, selection, program readback x2
        // (truthful), restore readback x2 (lies).
        bus.script(l.cbar_addr(0).unwrap(), &[0, 0xbad, 0xbad]);
        bus.script(
            smr_addr(0).unwrap(),
            &[0, 0, 0, 0x8000_0740, 0x8000_0740, 0xbad, 0xbad],
        );
        assert_eq!(
            establish_cb_bypass(&mut bus, &CbRecipe::operator()),
            Err(CbBypassError::RestoreFailed {
                smr: 0,
                cb: 0,
                failure: ReadbackFailure {
                    reg: RegName::Smr,
                    addr: smr_addr(0).unwrap(),
                    expected: 0,
                    first: 0xbad,
                    second: 0xbad,
                }
            })
        );
    }

    #[test]
    fn rescan_still_notmatched_restores() {
        let l = layout();
        let mut bus = FakeBus::realistic();
        // Readbacks tell the truth (the written SMR reads
        // back), but the verification scans see the SMR as
        // invalid again — the stream never took effect at the
        // matching level. SMR read sequence: baseline x2,
        // selection, readback x2, verify scans x2.
        bus.script(
            smr_addr(0).unwrap(),
            &[0, 0, 0, 0x8000_0740, 0x8000_0740, 0, 0],
        );
        assert_eq!(
            establish_cb_bypass(&mut bus, &CbRecipe::operator()),
            Err(CbBypassError::VerifyMismatch {
                smr: 0,
                cb: 0,
                first: SidObservation::NotMatched,
                second: SidObservation::NotMatched,
            })
        );
        // Restored: the SMR holds its snapshot value and the
        // write log shows program-then-restore for it.
        assert_eq!(bus.stored(smr_addr(0).unwrap()), 0);
        let smr_writes: Vec<u32> = bus
            .writes
            .iter()
            .filter(|(a, _)| *a == smr_addr(0).unwrap())
            .map(|(_, v)| *v)
            .collect();
        assert_eq!(smr_writes, vec![0x8000_0740, 0]);
        let _ = l;
    }

    #[test]
    fn gfsr_after_program_restores() {
        let l = layout();
        let mut bus = FakeBus::realistic();
        // GFSR reads: baseline dump 0, pre-write dump 0,
        // post-program dump 0x10.
        bus.script(global_addr(GFSR_OFF), &[0, 0, 0x10]);
        assert_eq!(
            establish_cb_bypass(&mut bus, &CbRecipe::operator()),
            Err(CbBypassError::FaultAfterProgram {
                smr: 0,
                cb: 0,
                gfsr: 0x10,
                gfsynr0: 0,
                gfsynr1: 0,
                gfsynr2: 0
            })
        );
        assert_eq!(bus.stored(smr_addr(0).unwrap()), 0);
        assert_eq!(bus.stored(l.cbar_addr(0).unwrap()), 0);
    }

    #[test]
    fn verdict_line_refused_is_self_contained() {
        // The exact printed text: raw IDR values and GFSR in
        // the verdict line itself, SMR/CB dashed (the refusal
        // precedes selection).
        let facts = VerdictFacts {
            idr0: Some(IDR0_HW),
            idr1: Some(IDR1_HW),
            gfsr: Some(0),
            scr0: Some(0x002a_0406),
            base: Some(APPS_SMMU_BASE),
            ..Default::default()
        };
        assert_eq!(
            verdict_line(facts, &Stage1Verdict::Refused),
            "CB-BYPASS REFUSED: IDR0=0x4c017e4c IDR1=0x5000002d \
             GFSR=0x00000000 SCR0=0x002a0406 BASE=0x15000000 \
             SMR=- CB=- IDR counts/window disagree \
             with the pinned SDM845 facts; no writes"
        );
    }

    #[test]
    fn verdict_line_aborted_dashes_values_never_read() {
        // The early abort (window never opened) read nothing:
        // every value prints as `-`, never a fabricated number.
        assert_eq!(
            verdict_line(
                VerdictFacts::default(),
                &Stage1Verdict::Aborted("window open failed")
            ),
            "CB-BYPASS ABORT: IDR0=- IDR1=- GFSR=- SCR0=- BASE=- \
             SMR=- CB=- window open failed; no writes, no DMA"
        );
    }

    #[test]
    fn verdict_line_failed_baseline_matched_names_facts() {
        // A baseline that is already matched aborts the
        // attempt before selection: the line names the facts
        // and dashes the indices.
        let mut bus = FakeBus::realistic();
        bus.program(2, DWC3_SID, 0, 1, 0); // BYPASS already
        let err = establish_cb_bypass(&mut bus, &CbRecipe::operator()).unwrap_err();
        let line = verdict_line(
            VerdictFacts {
                idr0: Some(IDR0_HW),
                idr1: Some(IDR1_HW),
                gfsr: Some(0),
                scr0: Some(0x002a_0406),
                base: Some(APPS_SMMU_BASE),
                ..Default::default()
            },
            &Stage1Verdict::Failed(&err),
        );
        assert!(line.starts_with("CB-BYPASS FAILED (fail-closed): "), "{line}");
        assert!(line.contains("IDR0=0x4c017e4c"), "{line}");
        assert!(line.contains("IDR1=0x5000002d"), "{line}");
        assert!(line.contains("GFSR=0x00000000"), "{line}");
        assert!(line.contains("SCR0=0x002a0406"), "{line}");
        assert!(line.contains("BASE=0x15000000"), "{line}");
        assert!(!line.contains("GFAR"), "{line}");
        assert!(line.contains("SMR=- CB=-"), "{line}");
        assert!(line.contains("BaselineMatched"), "{line}");
        assert!(line.ends_with("; no DMA attempted"), "{line}");
    }

    #[test]
    fn verdict_line_failed_readback_names_tried_indices() {
        // Mid-program failure: the line names the SMR/CB the
        // recipe tried, with the raw failure values.
        let l = layout();
        let mut bus = FakeBus::realistic();
        bus.script(l.cbar_addr(0).unwrap(), &[0, 0xbad, 0xbad]);
        let err = establish_cb_bypass(&mut bus, &CbRecipe::operator()).unwrap_err();
        let line = verdict_line(
            VerdictFacts {
                idr0: Some(IDR0_HW),
                idr1: Some(IDR1_HW),
                gfsr: Some(0),
                scr0: Some(0x002a_0406),
                base: Some(APPS_SMMU_BASE),
                ..Default::default()
            },
            &Stage1Verdict::Failed(&err),
        );
        assert!(line.contains("IDR0=0x4c017e4c"), "{line}");
        assert!(line.contains("SMR=0 CB=0"), "{line}");
        assert!(line.contains("ReadbackMismatch"), "{line}");
        assert!(line.contains("expected: 65536"), "{line}");
        assert!(line.contains("first: 2989"), "{line}");
    }

    #[test]
    fn verdict_line_failed_restore_names_tried_indices() {
        // A restore failure dominates; its line still names
        // the tried pair and the restore failure itself.
        let l = layout();
        let mut bus = FakeBus::realistic();
        bus.script(l.cbar_addr(0).unwrap(), &[0, 0xbad, 0xbad]);
        bus.script(
            smr_addr(0).unwrap(),
            &[0, 0, 0, 0x8000_0740, 0x8000_0740, 0xbad, 0xbad],
        );
        let err = establish_cb_bypass(&mut bus, &CbRecipe::operator()).unwrap_err();
        let line = verdict_line(
            VerdictFacts {
                idr0: Some(IDR0_HW),
                idr1: Some(IDR1_HW),
                gfsr: Some(0),
                scr0: Some(0x002a_0406),
                base: Some(APPS_SMMU_BASE),
                ..Default::default()
            },
            &Stage1Verdict::Failed(&err),
        );
        assert!(line.contains("SMR=0 CB=0"), "{line}");
        assert!(line.contains("RestoreFailed"), "{line}");
    }

    #[test]
    fn verdict_line_failed_layout_uses_the_errors_idr_values() {
        // The IDR pair inside a LayoutMismatch is what the
        // machine judged; it wins over the caller's facts.
        let err = CbBypassError::LayoutMismatch {
            idr0: IDR0_HW,
            idr1: 0x5000_002c,
        };
        let line = verdict_line(
            VerdictFacts {
                idr0: Some(0),
                idr1: Some(0),
                gfsr: Some(0),
                scr0: Some(0x002a_0406),
                base: Some(APPS_SMMU_BASE),
                ..Default::default()
            },
            &Stage1Verdict::Failed(&err),
        );
        assert!(line.contains("IDR0=0x4c017e4c"), "{line}");
        assert!(line.contains("IDR1=0x5000002c"), "{line}");
        assert!(line.contains("SMR=- CB=-"), "{line}");
    }

    #[test]
    fn verdict_line_aborted_with_fault_dashes_unread_fault_block() {
        // An abort after the pre-check, with a global fault
        // already showing: the tail prints, but the fault
        // block was never read at failure time on this path,
        // so its fields dash rather than fabricate.
        let facts = VerdictFacts {
            idr0: Some(IDR0_HW),
            idr1: Some(IDR1_HW),
            gfsr: Some(0x0000_0004),
            scr0: Some(0x002a_0406),
            base: Some(APPS_SMMU_BASE),
            ..Default::default()
        };
        assert_eq!(
            verdict_line(facts, &Stage1Verdict::Aborted("RW window open failed")),
            "CB-BYPASS ABORT: IDR0=0x4c017e4c IDR1=0x5000002d \
             GFSR=0x00000004 SCR0=0x002a0406 BASE=0x15000000 \
             SMR=- CB=- GFAR=- GFSYNR0=- GFSYNR1=- GFSYNR2=- \
             RW window open failed; no writes, no DMA"
        );
    }

    #[test]
    fn verdict_line_failed_fault_prints_verdict_time_fault_block() {
        // A fault after programming: the head GFSR and the
        // GFSYNR values come from the error (what the machine
        // judged at fault time); GFAR comes from the caller's
        // verdict-time read - no error variant carries it.
        let err = CbBypassError::FaultAfterProgram {
            smr: 0,
            cb: 0,
            gfsr: 0x10,
            gfsynr0: 0x740,
            gfsynr1: 0x20,
            gfsynr2: 0x30,
        };
        let line = verdict_line(
            VerdictFacts {
                idr0: Some(IDR0_HW),
                idr1: Some(IDR1_HW),
                gfsr: Some(0),
                scr0: Some(0x002a_0406),
                base: Some(APPS_SMMU_BASE),
                gfar: Some(0x0001_8000),
                gfsynr0: Some(0x9999),
                gfsynr1: Some(0x8888),
                gfsynr2: Some(0x7777),
            },
            &Stage1Verdict::Failed(&err),
        );
        assert!(line.contains("GFSR=0x00000010"), "{line}");
        assert!(line.contains("SMR=0 CB=0"), "{line}");
        assert!(
            line.contains("GFAR=0x00018000 GFSYNR0=0x00000740 GFSYNR1=0x00000020 GFSYNR2=0x00000030"),
            "{line}"
        );
        assert!(line.contains("FaultAfterProgram"), "{line}");
    }

    #[test]
    fn verdict_line_refused_never_prints_fault_tail() {
        // The refusal precedes any programming; even with a
        // stale global fault showing, the fault block is not
        // part of a REFUSED line (it belongs to ABORT/FAILED).
        let facts = VerdictFacts {
            idr0: Some(IDR0_HW),
            idr1: Some(IDR1_HW),
            gfsr: Some(0x0000_0004),
            scr0: Some(0x002a_0406),
            base: Some(APPS_SMMU_BASE),
            gfar: Some(0x0001_8000),
            gfsynr0: Some(0x740),
            gfsynr1: Some(0x20),
            gfsynr2: Some(0x40),
        };
        let line = verdict_line(facts, &Stage1Verdict::Refused);
        assert!(!line.contains("GFAR"), "{line}");
    }
}
