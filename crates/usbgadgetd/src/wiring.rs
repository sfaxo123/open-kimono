//! Payload v2 — main-wiring decision layer for `usbgadgetd`.
//!
//! Pure, host-testable composition of the two predecessor layers:
//! the SMMU platform probe (`smmu_probe.rs`, producer) and the
//! SID 0x740 establish-and-verify gate (`sid_gate.rs`, decider).
//! This is the exact sequence `main` runs on the Redox target
//! (through the physmap-backed reader in `redox_smmu.rs`):
//!
//!   scan -> SidGate::establish -> scan -> SidGate::verify
//!        -> gate's DMA-mode answer
//!
//! FIRST WIRED RUN IS LOG-ONLY (Wonder progress review
//! 20261030-0955, gate-lite criterion): this layer computes and
//! reports the verified mode, and `main` then REFUSES DMA in
//! every case — it does not call `plan_dma_gated`, does not
//! construct `HwWindows`, and does not init the DWC3 core. The
//! on-device readback (matched SMR/S2CR indices and raw values,
//! sequence numbers) must be cross-checked against the
//! independent pmOS record (MMU-500 at 0x15000000/0x8000, 76
//! groups, 45 banks, group 12 TRANSLATED under Linux) before
//! any later packet may enable DMA.
//!
//! Two failure shapes are kept distinct, because they mean
//! different things on the serial log:
//! - `WiringError`: the protocol itself failed — a scan errored
//!   (e.g. two SMRs match: `ProbeError::Ambiguous`) or the gate
//!   rejected the pair (a second scan that disagrees with the
//!   first is a `VerificationMismatch`, latched closed). No
//!   mode was ever verified.
//! - A verified refusal carried INSIDE the report
//!   (`mode: Err(SidNotMatched)` / `Err(SidFault)` / ...): two
//!   independent scans AGREED, and the agreed stream state is
//!   one DMA must never be attempted under. This is the
//!   binary-path negative case: on QEMU (no SMMU behind the
//!   window, reads return 0) the expected verified result is
//!   `NotMatched` -> `SidNotMatched`, and the daemon exits
//!   cleanly having attempted no DMA.

use crate::physmap_plan::DmaMode;
use crate::sid_gate::{GateError, SidGate, SidObservation};
use crate::smmu_probe::{s2cr_addr, smr_addr, ProbeError, ProbeRead, Read32, SmmuProbe};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WiringError {
    /// A scan itself failed (sequence number still consumed).
    Probe(ProbeError),
    /// The gate rejected the establish/verify pair.
    Gate(GateError),
}

/// The outcome of one log-only wiring run: both probe reads
/// (with their sequence numbers) and the gate's DMA-mode answer
/// for the verified pair. `mode` is `Err` for a VERIFIED refusal
/// (NotMatched / Fault / out-of-range) — the gate did its job;
/// that is not a wiring failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogOnlyReport {
    first: ProbeRead,
    second: ProbeRead,
    mode: Result<DmaMode, GateError>,
}

impl LogOnlyReport {
    /// First scan (the establish read).
    pub fn first(&self) -> ProbeRead {
        self.first
    }
    /// Second, independent scan (the verify read).
    pub fn second(&self) -> ProbeRead {
        self.second
    }
    /// The gate's answer: a verified `DmaMode`, or the refusal
    /// the verified stream state mandates.
    pub fn mode(&self) -> Result<DmaMode, GateError> {
        self.mode
    }
}

/// Run the wiring sequence over `probe`: two scans, establish
/// then verify, then read the gate's mode answer. Performs no
/// DMA planning and no MMIO of its own — the probe's `Read32`
/// source is the only hardware touch, and it is read-only.
pub fn probe_twice_and_verify<R: Read32>(
    probe: &mut SmmuProbe<R>,
) -> Result<LogOnlyReport, WiringError> {
    let first = probe.scan().map_err(WiringError::Probe)?;
    let mut gate = SidGate::new();
    gate.establish(first.obs(), first.seq())
        .map_err(WiringError::Gate)?;
    let second = probe.scan().map_err(WiringError::Probe)?;
    gate.verify(second.obs(), second.seq())
        .map_err(WiringError::Gate)?;
    Ok(LogOnlyReport {
        first,
        second,
        mode: gate.dma_mode(),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchedPairError {
    /// The observation names an SMR index that has no register:
    /// there are no raw values to report, and inventing them
    /// (e.g. reading address 0) would fabricate exactly the
    /// evidence the gate-lite cross-check relies on.
    SmrOutOfRange(usize),
}

/// Read back the matched pair's raw SMR/S2CR values for the log
/// line. This is a LOGGING read only — never a gate input (the
/// gate consumed exactly the two sequenced probe scans).
/// `None` for `NotMatched` (there is no pair); `Err` for an
/// out-of-range index, instead of fabricated values.
pub fn matched_pair<R: Read32>(
    reader: &mut R,
    obs: SidObservation,
) -> Option<Result<(usize, u32, u32), MatchedPairError>> {
    let smr = match obs {
        SidObservation::Bypass { smr }
        | SidObservation::Translated { smr, .. }
        | SidObservation::Fault { smr } => smr,
        SidObservation::NotMatched => return None,
    };
    let (Some(sa), Some(ca)) = (smr_addr(smr), s2cr_addr(smr)) else {
        return Some(Err(MatchedPairError::SmrOutOfRange(smr)));
    };
    Some(Ok((smr, reader.read32(sa), reader.read32(ca))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sid_gate::{SidObservation, DWC3_SID};
    use crate::smmu_probe::{s2cr_addr, smr_addr};
    use std::collections::HashMap;

    #[derive(Default)]
    struct FakeSmmu {
        regs: HashMap<u64, u32>,
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
            *self.regs.get(&addr).unwrap_or(&0)
        }
    }

    fn run_with(fake: FakeSmmu) -> Result<LogOnlyReport, WiringError> {
        let mut probe = SmmuProbe::new(fake);
        probe_twice_and_verify(&mut probe)
    }

    #[test]
    fn verified_bypass_reports_bypass_with_both_seqs() {
        let mut fake = FakeSmmu::default();
        fake.program(9, DWC3_SID, 0, 1, 0); // BYPASS
        let report = run_with(fake).unwrap();
        assert_eq!(report.first().seq(), 1);
        assert_eq!(report.second().seq(), 2);
        assert_eq!(report.first().obs(), SidObservation::Bypass { smr: 9 });
        assert_eq!(report.mode(), Ok(DmaMode::Bypass));
    }

    #[test]
    fn verified_translated_reports_translated() {
        let mut fake = FakeSmmu::default();
        fake.program(12, DWC3_SID, 0, 0, 12); // TRANS cb12: the pmOS record
        let report = run_with(fake).unwrap();
        assert_eq!(report.mode(), Ok(DmaMode::Translated));
    }

    #[test]
    fn verified_not_matched_is_a_refusal_not_a_wiring_error() {
        // The QEMU binary-path negative case: no SMR matches, two
        // scans agree, and the gate's answer is a refusal carried
        // in the report — DMA is refused, the run is not an error.
        let report = run_with(FakeSmmu::default()).unwrap();
        assert_eq!(report.first().obs(), SidObservation::NotMatched);
        assert_eq!(report.mode(), Err(GateError::SidNotMatched));
    }

    #[test]
    fn verified_fault_is_a_refusal() {
        let mut fake = FakeSmmu::default();
        fake.program(9, DWC3_SID, 0, 2, 0); // FAULT
        let report = run_with(fake).unwrap();
        assert_eq!(report.mode(), Err(GateError::SidFault));
    }

    /// Register file whose SID 0x740 entry flips from BYPASS to
    /// TRANSLATED after the first full scan (152 reads = 76
    /// SMR/S2CR pairs): the two scans disagree.
    struct FlipSmmu {
        reads: usize,
    }

    impl Read32 for FlipSmmu {
        fn read32(&mut self, addr: u64) -> u32 {
            self.reads += 1;
            let second_scan = self.reads > 152;
            if addr == smr_addr(9).unwrap() {
                (1u32 << 31) | DWC3_SID as u32
            } else if addr == s2cr_addr(9).unwrap() {
                if second_scan {
                    0 // TRANS cb 0
                } else {
                    1 << 16 // BYPASS
                }
            } else {
                0
            }
        }
    }

    #[test]
    fn disagreeing_second_scan_fails_the_gate_closed() {
        let mut probe = SmmuProbe::new(FlipSmmu { reads: 0 });
        assert_eq!(
            probe_twice_and_verify(&mut probe),
            Err(WiringError::Gate(GateError::VerificationMismatch))
        );
    }

    #[test]
    fn matched_pair_reads_back_the_raw_values() {
        let mut fake = FakeSmmu::default();
        fake.program(12, DWC3_SID, 0, 0, 12);
        let got = matched_pair(&mut fake, SidObservation::Translated {
            smr: 12,
            context_bank: 12,
        });
        assert_eq!(got, Some(Ok((12, (1u32 << 31) | DWC3_SID as u32, 12))));
    }

    #[test]
    fn matched_pair_none_for_not_matched() {
        let mut fake = FakeSmmu::default();
        assert_eq!(matched_pair(&mut fake, SidObservation::NotMatched), None);
    }

    #[test]
    fn matched_pair_out_of_range_is_an_error_not_fabricated_values() {
        let mut fake = FakeSmmu::default();
        assert_eq!(
            matched_pair(&mut fake, SidObservation::Bypass { smr: 999 }),
            Some(Err(MatchedPairError::SmrOutOfRange(999)))
        );
    }

    #[test]
    fn ambiguous_scan_is_a_probe_error() {
        let mut fake = FakeSmmu::default();
        fake.program(3, DWC3_SID, 0, 1, 0);
        fake.program(9, 0x740, 0xf, 1, 0);
        assert_eq!(
            run_with(fake),
            Err(WiringError::Probe(ProbeError::Ambiguous {
                first: 3,
                second: 9
            }))
        );
    }
}
