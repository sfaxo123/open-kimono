//! Payload-v2 predecessor — SMMU platform probe for `usbgadgetd`.
//!
//! Pure, host-testable scan layer that produces the observations
//! the SID 0x740 gate (`sid_gate.rs`) consumes. The gate exists
//! but, until now, nothing in the crate could produce a
//! `SidObservation` from register values in scan order: the
//! decoder handled one SMR/S2CR pair at a time and the sequencing
//! discipline (distinct reads, monotonically increasing sequence
//! numbers) lived only in the gate's protocol docs. This module
//! is the missing producer half:
//!
//! - It pins the `apps_smmu` window and the global-space register
//!   addresses: SMRn at `APPS_SMMU_BASE + 0x800 + 4*n`, S2CRn at
//!   `APPS_SMMU_BASE + 0xc00 + 4*n` for `n` in
//!   `0..APPS_SMMU_SMR_COUNT` (ARM SMMUv2 / MMU-500 architecture
//!   facts, already recorded in `sid_gate.rs`; the window length
//!   0x8000 is the `iommu@15000000` reg size from `sdm845.dtsi`,
//!   confirmed first-hand by the 2026-10-30 pmOS capture:
//!   76 stream-matching groups, 45 context banks).
//! - It scans ALL pairs through a `Read32` source and reduces
//!   them to exactly one observation. More than one matching SMR
//!   is a hard error (`ProbeError::Ambiguous`), never a silent
//!   first-match win: with two matches the architecture's
//!   behaviour depends on match priority, and a probe that
//!   guesses could feed the gate a Bypass the hardware does not
//!   apply to this stream. Failing closed is the only honest
//!   answer until the on-device readback shows whether multiple
//!   matches can occur at all.
//! - Every scan consumes one sequence number from a strictly
//!   increasing counter — including a scan that errors — so two
//!   `ProbeRead`s from one probe can feed `SidGate::establish` /
//!   `verify` directly, and a failed scan can never be replayed
//!   as its own verification.
//!
//! What this module is NOT: it performs no MMIO itself (the
//! `Read32` source is an abstraction; the Redox physmap-backed
//! adapter belongs to the main-wiring packet, where the SMMU
//! window mapping lands next to `redox_glue`'s DWC3 windows),
//! it writes no SMMU register, and it is not wired into `main`
//! yet. Honest limit, inherited from the gate: the scan cannot
//! detect a SYSTEMATICALLY wrong register map — two scans
//! through the same wrong offsets agree with each other — so
//! the first on-device readback must still be cross-checked
//! against the independent pmOS record (group 12 TRANSLATED
//! under Linux; 76 groups / 45 banks) before any DMA is enabled.

use crate::physmap_plan::{plan_window, PlanError, WindowPlan};
use crate::sid_gate::{
    observation_from_registers, SidObservation, APPS_SMMU_BASE, APPS_SMMU_SMR_COUNT,
};

/// `apps_smmu` global register space length (`sdm845.dtsi`
/// `iommu@15000000` reg size; the 2026-10-30 pmOS capture's
/// resource record agrees).
pub const APPS_SMMU_LEN: u64 = 0x8000;
/// Offset of SMR0 in the global register space (SMMUv2).
pub const SMR_BASE_OFF: u64 = 0x800;
/// Offset of S2CR0 in the global register space (SMMUv2).
pub const S2CR_BASE_OFF: u64 = 0xc00;

/// Absolute address of SMR`index`, or `None` when `index` names
/// a register group the hardware does not have.
pub fn smr_addr(index: usize) -> Option<u64> {
    if index >= APPS_SMMU_SMR_COUNT {
        return None;
    }
    APPS_SMMU_BASE.checked_add(SMR_BASE_OFF + 4 * index as u64)
}

/// Absolute address of S2CR`index`; same range discipline as
/// [`smr_addr`].
pub fn s2cr_addr(index: usize) -> Option<u64> {
    if index >= APPS_SMMU_SMR_COUNT {
        return None;
    }
    APPS_SMMU_BASE.checked_add(S2CR_BASE_OFF + 4 * index as u64)
}

/// The part-1 window plan for mapping the SMMU global space:
/// page-aligned base, so the mapping starts exactly at
/// `APPS_SMMU_BASE` with no in-page offset.
pub fn smmu_window_plan() -> Result<WindowPlan, PlanError> {
    plan_window(APPS_SMMU_BASE, APPS_SMMU_LEN)
}

/// Offset of SMMU_sCR0 in the global register space (SMMUv2).
pub const SCR0_OFF: u64 = 0x00;
/// Offset of SMMU_SCR1 in the global register space (SMMUv2).
pub const SCR1_OFF: u64 = 0x04;
/// Offset of SMMU_SCR2 in the global register space (SMMUv2).
pub const SCR2_OFF: u64 = 0x08;
/// Offset of SMMU_IDR0 in the global register space (SMMUv2).
pub const IDR0_OFF: u64 = 0x20;
/// Offset of SMMU_IDR1 in the global register space (SMMUv2).
pub const IDR1_OFF: u64 = 0x24;
/// Offset of SMMU_IDR2 in the global register space (SMMUv2).
pub const IDR2_OFF: u64 = 0x28;
/// Offset of SMMU_GFAR (low half of the 64-bit global fault
/// address register pair sGFAR lo/hi) in the global register
/// space. Register map per AMD UG1087 (SMMU500 register
/// summary) and the Linux arm-smmu GR0 map (whose comment
/// also pins the non-secure aliases nsGFSR 0x448 /
/// nsGFSYNR0 0x450): sGFAR lo 0x40, sGFAR hi 0x44,
/// GFSR 0x48, GFSRRESTORE 0x4c (WRITE-ONLY), GFSYNR0 0x50,
/// GFSYNR1 0x54, GFSYNR2 0x58.
///
/// CORRECTED 2026-11-02: this tree previously pinned the
/// fault block shifted - GFSR_OFF was 0x50, GFSYNR0_OFF
/// 0x54, GFSYNR1_OFF 0x58, and GFAR_OFF 0x4c. In the real
/// map 0x50/0x54/0x58 are GFSYNR0/1/2, and 0x4c is
/// GFSRRESTORE, a write-only register: never read it (a
/// read is meaningless) and never write it either - the
/// whole fault block is read-only for us. Historical probe
/// readings labelled "GFSR" were actually GFSYNR0 (benign
/// while idle: both read 0, so no gate decision was ever
/// affected), and GFAR was never validly read. GFAR feeds
/// verdict-line diagnostics only (a read-only print at
/// failure time), never a gate input.
pub const GFAR_OFF: u64 = 0x40;
/// Offset of SMMU_GFSR in the global register space (SMMUv2).
pub const GFSR_OFF: u64 = 0x48;
/// Offset of SMMU_GFSYNR0 in the global register space (SMMUv2).
pub const GFSYNR0_OFF: u64 = 0x50;
/// Offset of SMMU_GFSYNR1 in the global register space (SMMUv2).
pub const GFSYNR1_OFF: u64 = 0x54;
/// Offset of SMMU_GFSYNR2 in the global register space (SMMUv2).
pub const GFSYNR2_OFF: u64 = 0x58;

/// Absolute address of the global register at `off` (all global
/// offsets above lie inside the pinned window by construction).
pub fn global_addr(off: u64) -> u64 {
    APPS_SMMU_BASE + off
}

/// Probe iteration 2 — raw dump of the SMMU global registers the
/// Attempt #2 readback did not cover. LOG-ONLY, like the matched
/// pair readback: these values are never a gate input. They settle
/// (a) whether the register map is the real one — IDR0/IDR1 decode
/// to the stream-matching group and context-bank counts, which
/// must cross-check against the independent pmOS record (76
/// groups, 45 banks) before the map is trusted — and (b) the raw
/// global config (sCR0/SCR1/SCR2) and global fault state
/// (GFSR/GFSYNR0/1/2) from which unmatched-stream behaviour under
/// the ABL -> U-Boot -> Redox chain is read off by a human
/// against the ARM SMMUv2 architecture, before payload v3
/// establishes SID 0x740 deliberately. No sCR0 bit semantics are
/// decoded in code: a wrong in-code decode would be worse than
/// raw values a reviewer can check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GlobalDump {
    pub scr0: u32,
    pub scr1: u32,
    pub scr2: u32,
    pub idr0: u32,
    pub idr1: u32,
    pub idr2: u32,
    pub gfsr: u32,
    pub gfsynr0: u32,
    pub gfsynr1: u32,
    pub gfsynr2: u32,
}

impl GlobalDump {
    /// Stream-matching group count from IDR0 (bits [7:0];
    /// ARM SMMUv2 IDR0 NUMSMRG[7:0] - IHI 0062B sec. 10.6 and
    /// the MMU-500 TRM (DDI 0517B): "SMMU_IDR0.NUMSMRG[7:0] =
    /// SMR DEPTH". Bits [23:16] of IDR0 are NUMIRPT (the
    /// context-fault interrupt count), NOT the group count:
    /// the first probe-2 version decoded bits [26:16] and
    /// would have printed the interrupt count as "groups" on
    /// device, failing the 76 cross-check against a healthy
    /// SMMU. Cross-check target: 76 (pmOS).
    pub fn num_smr_groups(&self) -> u32 {
        self.idr0 & 0xff
    }
    /// Context-bank count from IDR1 (bits [7:0]; ARM SMMUv2
    /// IDR1 NUMCB). Cross-check target: 45 (pmOS).
    pub fn num_context_banks(&self) -> u32 {
        self.idr1 & 0xff
    }
    /// One panel-capturable log block: every register on ONE
    /// line, so a single photo carries the whole dump (a missing
    /// register in the capture = UNVERIFIED, per gate-lite).
    pub fn log_line(&self) -> String {
        format!(
            "sCR0={:#010x} SCR1={:#010x} SCR2={:#010x} IDR0={:#010x} IDR1={:#010x} IDR2={:#010x} GFSR={:#010x} GFSYNR0={:#010x} GFSYNR1={:#010x} GFSYNR2={:#010x} groups={} banks={}",
            self.scr0, self.scr1, self.scr2, self.idr0, self.idr1, self.idr2,
            self.gfsr, self.gfsynr0, self.gfsynr1, self.gfsynr2,
            self.num_smr_groups(), self.num_context_banks(),
        )
    }
}

/// Read the global registers once through `reader`. Read-only;
/// consumes no probe sequence number (it is not a gate scan).
pub fn dump_globals<R: Read32>(reader: &mut R) -> GlobalDump {
    GlobalDump {
        scr0: reader.read32(global_addr(SCR0_OFF)),
        scr1: reader.read32(global_addr(SCR1_OFF)),
        scr2: reader.read32(global_addr(SCR2_OFF)),
        idr0: reader.read32(global_addr(IDR0_OFF)),
        idr1: reader.read32(global_addr(IDR1_OFF)),
        idr2: reader.read32(global_addr(IDR2_OFF)),
        gfsr: reader.read32(global_addr(GFSR_OFF)),
        gfsynr0: reader.read32(global_addr(GFSYNR0_OFF)),
        gfsynr1: reader.read32(global_addr(GFSYNR1_OFF)),
        gfsynr2: reader.read32(global_addr(GFSYNR2_OFF)),
    }
}

/// A source of 32-bit register reads. The Redox adapter will
/// back this with volatile reads over the physmap'ed SMMU
/// window; host tests back it with a fake register file. Any
/// closure `FnMut(u64) -> u32` is a source.
pub trait Read32 {
    fn read32(&mut self, addr: u64) -> u32;
}

impl<F: FnMut(u64) -> u32> Read32 for F {
    fn read32(&mut self, addr: u64) -> u32 {
        self(addr)
    }
}

/// One completed scan: the observation plus the sequence number
/// of the read set that produced it, ready to hand to
/// `SidGate::establish` / `SidGate::verify`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProbeRead {
    seq: u64,
    obs: SidObservation,
}

impl ProbeRead {
    /// Sequence number of this scan (strictly increasing per
    /// probe, starting at 1).
    pub fn seq(&self) -> u64 {
        self.seq
    }
    /// What the scan observed for SID 0x740.
    pub fn obs(&self) -> SidObservation {
        self.obs
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeError {
    /// Two SMRs match SID 0x740. The probe refuses to guess
    /// which one governs the stream.
    Ambiguous { first: usize, second: usize },
}

/// The SMMU scanner. Owns the read source and the sequence
/// counter; one probe instance is the unit of read independence
/// the gate's protocol assumes (its scans are distinct reads by
/// construction).
pub struct SmmuProbe<R: Read32> {
    reader: R,
    next_seq: u64,
}

impl<R: Read32> SmmuProbe<R> {
    pub fn new(reader: R) -> Self {
        SmmuProbe {
            reader,
            next_seq: 1,
        }
    }

    /// Scan every SMR/S2CR pair once and reduce to the single
    /// observation for SID 0x740 (`NotMatched` when no valid SMR
    /// matches). The scan's sequence number is consumed even
    /// when the scan errors, so sequence numbers are never
    /// reused.
    pub fn scan(&mut self) -> Result<ProbeRead, ProbeError> {
        let seq = self.next_seq;
        self.next_seq += 1;
        let mut found: Option<SidObservation> = None;
        for index in 0..APPS_SMMU_SMR_COUNT {
            // In-range by construction (index < count).
            let smr = self.reader.read32(smr_addr(index).unwrap());
            let s2cr = self.reader.read32(s2cr_addr(index).unwrap());
            if let Some(obs) = observation_from_registers(index, smr, s2cr) {
                if let Some(prev) = found {
                    let first = match prev {
                        SidObservation::Bypass { smr }
                        | SidObservation::Translated { smr, .. }
                        | SidObservation::Fault { smr } => smr,
                        SidObservation::NotMatched => unreachable!(),
                    };
                    return Err(ProbeError::Ambiguous {
                        first,
                        second: index,
                    });
                }
                found = Some(obs);
            }
        }
        Ok(ProbeRead {
            seq,
            obs: found.unwrap_or(SidObservation::NotMatched),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physmap_plan::PlanError;
    use crate::sid_gate::{plan_dma_gated, GateError, GatedPlanError, SidGate, DWC3_SID};
    use std::collections::HashMap;

    /// Fake SMMU register file: unprogrammed addresses read 0
    /// (VALID clear, so they never match).
    #[derive(Default)]
    struct FakeSmmu {
        regs: HashMap<u64, u32>,
        /// Every address read, in order (test observability for
        /// the globals dump's read-once contract).
        reads: Vec<u64>,
    }

    impl FakeSmmu {
        fn program(&mut self, index: usize, id: u16, mask: u16, ty: u32, cbndx: u8) {
            let smr = (1u32 << 31) | (((mask as u32) & 0x7fff) << 16) | id as u32;
            let s2cr = (ty << 16) | cbndx as u32;
            self.regs.insert(smr_addr(index).unwrap(), smr);
            self.regs.insert(s2cr_addr(index).unwrap(), s2cr);
        }
    }

    impl Read32 for FakeSmmu {
        fn read32(&mut self, addr: u64) -> u32 {
            self.reads.push(addr);
            *self.regs.get(&addr).unwrap_or(&0)
        }
    }

    #[test]
    fn register_addresses_pinned() {
        assert_eq!(smr_addr(0), Some(0x1500_0800));
        assert_eq!(smr_addr(75), Some(0x1500_092c));
        assert_eq!(s2cr_addr(0), Some(0x1500_0c00));
        assert_eq!(s2cr_addr(75), Some(0x1500_0d2c));
        assert_eq!(smr_addr(APPS_SMMU_SMR_COUNT), None);
        assert_eq!(s2cr_addr(APPS_SMMU_SMR_COUNT), None);
        // Every register the scan can name lies inside the window.
        let plan = smmu_window_plan().unwrap();
        for index in 0..APPS_SMMU_SMR_COUNT {
            for addr in [smr_addr(index).unwrap(), s2cr_addr(index).unwrap()] {
                assert!(plan.translate(addr, 4).is_some(), "addr {addr:#x}");
            }
        }
    }

    #[test]
    fn window_plan_pinned() {
        let plan = smmu_window_plan().unwrap();
        assert_eq!(plan.phys_base(), APPS_SMMU_BASE);
        assert_eq!(plan.len(), APPS_SMMU_LEN);
        assert_eq!(plan.map_base(), APPS_SMMU_BASE);
        assert_eq!(plan.map_len(), APPS_SMMU_LEN);
        assert_eq!(plan.in_page_offset(), 0);
    }

    #[test]
    fn scan_not_matched_when_no_valid_smr() {
        let mut probe = SmmuProbe::new(FakeSmmu::default());
        let read = probe.scan().unwrap();
        assert_eq!(read.obs(), SidObservation::NotMatched);
        assert_eq!(read.seq(), 1);
    }

    #[test]
    fn scan_finds_bypass_translated_and_fault() {
        let mut fake = FakeSmmu::default();
        fake.program(9, DWC3_SID, 0, 1, 0); // BYPASS
        let mut probe = SmmuProbe::new(fake);
        assert_eq!(
            probe.scan().unwrap().obs(),
            SidObservation::Bypass { smr: 9 }
        );

        let mut fake = FakeSmmu::default();
        fake.program(9, DWC3_SID, 0, 0, 12); // TRANS, cb 12 (the Linux record)
        let mut probe = SmmuProbe::new(fake);
        assert_eq!(
            probe.scan().unwrap().obs(),
            SidObservation::Translated {
                smr: 9,
                context_bank: 12
            }
        );

        let mut fake = FakeSmmu::default();
        fake.program(9, DWC3_SID, 0, 2, 0); // FAULT
        let mut probe = SmmuProbe::new(fake);
        assert_eq!(
            probe.scan().unwrap().obs(),
            SidObservation::Fault { smr: 9 }
        );
    }

    #[test]
    fn scan_ignores_other_streams() {
        let mut fake = FakeSmmu::default();
        fake.program(3, 0x100, 0, 1, 0); // a different stream, BYPASS
        fake.program(9, DWC3_SID, 0, 0, 12);
        let mut probe = SmmuProbe::new(fake);
        assert_eq!(
            probe.scan().unwrap().obs(),
            SidObservation::Translated {
                smr: 9,
                context_bank: 12
            }
        );
    }

    #[test]
    fn ambiguous_second_match_fails_closed() {
        // An exact match at index 3 and a masked-range match at
        // index 9 both cover SID 0x740: the probe must refuse,
        // not pick one.
        let mut fake = FakeSmmu::default();
        fake.program(3, DWC3_SID, 0, 1, 0);
        fake.program(9, 0x740, 0xf, 1, 0);
        let mut probe = SmmuProbe::new(fake);
        assert_eq!(
            probe.scan(),
            Err(ProbeError::Ambiguous {
                first: 3,
                second: 9
            })
        );
    }

    #[test]
    fn sequences_increase_and_errors_consume_theirs() {
        let mut probe = SmmuProbe::new(FakeSmmu::default());
        assert_eq!(probe.scan().unwrap().seq(), 1);
        assert_eq!(probe.scan().unwrap().seq(), 2);

        let mut fake = FakeSmmu::default();
        fake.program(3, DWC3_SID, 0, 1, 0);
        fake.program(9, 0x740, 0xf, 1, 0);
        let mut probe = SmmuProbe::new(fake);
        assert!(probe.scan().is_err()); // consumes seq 1
        // Clear the ambiguity by hand: the next scan is seq 2,
        // never a reuse of the failed scan's number.
        probe.reader.regs.remove(&smr_addr(9).unwrap());
        assert_eq!(probe.scan().unwrap().seq(), 2);
    }

    #[test]
    fn probe_feeds_gate_end_to_end_bypass() {
        let mut fake = FakeSmmu::default();
        fake.program(9, DWC3_SID, 0, 1, 0);
        let mut probe = SmmuProbe::new(fake);
        let mut gate = SidGate::new();
        let first = probe.scan().unwrap();
        gate.establish(first.obs(), first.seq()).unwrap();
        let second = probe.scan().unwrap();
        gate.verify(second.obs(), second.seq()).unwrap();
        let plan = plan_dma_gated(&gate, 0x8000_0000, None).unwrap();
        assert_eq!(plan.bus(), plan.phys());
    }

    #[test]
    fn probe_feeds_gate_translated_refuses_identity() {
        let mut fake = FakeSmmu::default();
        fake.program(9, DWC3_SID, 0, 0, 12);
        let mut probe = SmmuProbe::new(fake);
        let mut gate = SidGate::new();
        let first = probe.scan().unwrap();
        gate.establish(first.obs(), first.seq()).unwrap();
        let second = probe.scan().unwrap();
        gate.verify(second.obs(), second.seq()).unwrap();
        assert_eq!(
            plan_dma_gated(&gate, 0x8000_0000, None),
            Err(GatedPlanError::Plan(PlanError::MissingTranslatedBus))
        );
    }

    #[test]
    fn global_addresses_pinned_inside_window() {
        assert_eq!(global_addr(SCR0_OFF), 0x1500_0000);
        assert_eq!(global_addr(SCR1_OFF), 0x1500_0004);
        assert_eq!(global_addr(SCR2_OFF), 0x1500_0008);
        assert_eq!(global_addr(IDR0_OFF), 0x1500_0020);
        assert_eq!(global_addr(IDR1_OFF), 0x1500_0024);
        assert_eq!(global_addr(IDR2_OFF), 0x1500_0028);
        assert_eq!(global_addr(GFAR_OFF), 0x1500_0040);
        assert_eq!(global_addr(GFSR_OFF), 0x1500_0048);
        assert_eq!(global_addr(GFSYNR0_OFF), 0x1500_0050);
        assert_eq!(global_addr(GFSYNR1_OFF), 0x1500_0054);
        assert_eq!(global_addr(GFSYNR2_OFF), 0x1500_0058);
        let plan = smmu_window_plan().unwrap();
        for off in [SCR0_OFF, SCR1_OFF, SCR2_OFF, IDR0_OFF, IDR1_OFF,
                    IDR2_OFF, GFAR_OFF, GFSR_OFF, GFSYNR0_OFF, GFSYNR1_OFF, GFSYNR2_OFF] {
            assert!(plan.translate(global_addr(off), 4).is_some(), "off {off:#x}");
        }
    }

    #[test]
    fn dump_globals_reads_each_register_once_at_its_address() {
        let mut fake = FakeSmmu::default();
        // Ten distinct sentinels, one per register, so a swapped
        // or duplicated read cannot hide behind a zero value.
        fake.regs.insert(global_addr(SCR0_OFF), 0x1111_0000);
        fake.regs.insert(global_addr(SCR1_OFF), 0x2222_0000);
        fake.regs.insert(global_addr(SCR2_OFF), 0x3333_0000);
        // IDR0: NUMSMRG[7:0] = 76 (the pmOS group count) with
        // NUMIRPT[23:16] set to a DIFFERENT value (45): a decode
        // that reads the [23:16] field as the group count fails.
        fake.regs.insert(global_addr(IDR0_OFF), (45 << 16) | 76);
        fake.regs.insert(global_addr(IDR1_OFF), 45);
        fake.regs.insert(global_addr(IDR2_OFF), 0x6666_0000);
        fake.regs.insert(global_addr(GFSR_OFF), 0x7777_0000);
        fake.regs.insert(global_addr(GFSYNR0_OFF), 0x740);
        fake.regs.insert(global_addr(GFSYNR1_OFF), 0x9999_0000);
        fake.regs.insert(global_addr(GFSYNR2_OFF), 0xaaaa_0000);
        let dump = dump_globals(&mut fake);
        assert_eq!(dump.scr0, 0x1111_0000);
        assert_eq!(dump.scr1, 0x2222_0000);
        assert_eq!(dump.scr2, 0x3333_0000);
        assert_eq!(dump.idr0, (45 << 16) | 76);
        assert_eq!(dump.idr1, 45);
        assert_eq!(dump.idr2, 0x6666_0000);
        assert_eq!(dump.gfsr, 0x7777_0000);
        assert_eq!(dump.gfsynr0, 0x740);
        assert_eq!(dump.gfsynr1, 0x9999_0000);
        assert_eq!(dump.gfsynr2, 0xaaaa_0000);
        // Exactly one read per register, at its own address, in
        // dump order - the contract the test name claims.
        assert_eq!(
            fake.reads,
            vec![
                global_addr(SCR0_OFF),
                global_addr(SCR1_OFF),
                global_addr(SCR2_OFF),
                global_addr(IDR0_OFF),
                global_addr(IDR1_OFF),
                global_addr(IDR2_OFF),
                global_addr(GFSR_OFF),
                global_addr(GFSYNR0_OFF),
                global_addr(GFSYNR1_OFF),
                global_addr(GFSYNR2_OFF),
            ]
        );
        // The pmOS cross-check values decode back out.
        assert_eq!(dump.num_smr_groups(), 76);
        assert_eq!(dump.num_context_banks(), 45);
        let line = dump.log_line();
        assert!(line.contains("groups=76 banks=45"), "{line}");
        assert!(line.contains("GFSYNR1=0x99990000"), "{line}");
        assert!(line.contains("GFSYNR2=0xaaaa0000"), "{line}");
    }

    #[test]
    fn single_scan_cannot_verify_itself_through_the_gate() {
        let mut fake = FakeSmmu::default();
        fake.program(9, DWC3_SID, 0, 1, 0);
        let mut probe = SmmuProbe::new(fake);
        let mut gate = SidGate::new();
        let only = probe.scan().unwrap();
        gate.establish(only.obs(), only.seq()).unwrap();
        // Replaying the same ProbeRead as the verify step is a
        // mismatch: one scan can never verify itself.
        assert_eq!(
            gate.verify(only.obs(), only.seq()),
            Err(GateError::VerificationMismatch)
        );
    }
}
