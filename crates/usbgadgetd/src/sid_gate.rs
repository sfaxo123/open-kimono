//! Payload-v2 predecessor — SID 0x740 establish-and-verify gate
//! for `usbgadgetd`.
//!
//! Pure, host-testable decision layer that payload-v2 `main`
//! wiring will consume BEFORE any DMA address is handed to the
//! DWC3 core. It exists because of one hard fact chain:
//!
//! 1. The DWC3 controller DMAs through `apps_smmu` stream ID
//!    0x740 (`sdm845.dtsi:4236`, `iommus = <&apps_smmu 0x740 0>`).
//!    Its bus addresses are IOVAs, equal to physical addresses
//!    ONLY if that stream is in bypass (specs/dwc3-gadget.md §5,
//!    docs/GOTCHAS.md).
//! 2. The V3 oracle (docs/evidence/v3-oracle-transcript.txt)
//!    proved the stream is TRANSLATED under Linux (iommu_group
//!    12, stage-1). That is LINUX's configuration, not the boot
//!    chain's: what ABL leaves in `apps_smmu` when Redox boots is
//!    a separate question, so identity DMA must never be assumed
//!    from the oracle — the stream state must be read on the
//!    device, in the payload, before device DMA is enabled.
//! 3. First-hand pmOS facts recorded 2026-10-30 (read-only SSH,
//!    dmesg + sysfs): `apps_smmu` at 0x1500_0000 is
//!    `qcom,sdm845-smmu-500` / `arm,mmu-500`, SMMUv2, stage-1
//!    translation only, stream matching with 76 register groups,
//!    45 context banks (0 stage-2 only), 48-bit VA -> 48-bit IPA.
//!    Raw SMR/S2CR readback from pmOS userspace is BLOCKED:
//!    /dev/mem open returns EPERM even as root and arm-smmu
//!    exposes no debugfs dump — so register-level evidence must
//!    come from the payload's own probe, not from pmOS.
//!
//! Register facts used by the decoder below are ARM SMMUv2
//! (MMU-500) architecture facts, clean-room from the public
//! standard: in the global register space, SMRn is at
//! 0x800 + 4*n and S2CRn at 0xc00 + 4*n; SMR has VALID at bit 31,
//! MASK at bits [30:16] and ID at bits [15:0] (a stream matches
//! when `(id & !mask) == (sid & !mask)`); S2CR has TYPE at bits
//! [17:16] (0 = TRANS, 1 = BYPASS, 2 = FAULT) and CBNDX at bits
//! [7:0]. These offsets/fields are to be confirmed by the first
//! on-device readback. Honest limit (Wonder diff review of
//! 1ee8945): the establish/verify protocol guards against a
//! transient, flapping or single-read-wrong stream state — it
//! cannot detect a SYSTEMATICALLY wrong register map, because
//! two reads through the same wrong decoder agree with each
//! other. If the offsets above were wrong, a wrong Bypass
//! observation could verify; that is why the first on-device
//! readback must be cross-checked against an independent source
//! (the pmOS sysfs/dmesg record in this header) before any DMA
//! is enabled on hardware.
//!
//! What this module is NOT: it does not read or program any
//! SMMU register (the platform probe that feeds it real MMIO
//! reads — and, in a later separately-gated packet, establishes
//! bypass — is future work), and it is not wired into `main`
//! yet. What it guarantees, structurally:
//!
//! - Identity DMA (`DmaMode::Bypass`) is issued by this gate
//!   ONLY from a verified `Bypass` observation: two observations
//!   from DISTINCT probe reads (monotonically increasing read
//!   sequence numbers — the same read submitted twice is a
//!   verification mismatch) that agree exactly. The gate issues
//!   it as a `VerifiedMode` token, and `plan_dma_gated` is the
//!   composition payload-v2 wiring must use; the raw part-1
//!   `plan_dma` remains a low-level constructor (the gate, the
//!   host model path and tests use it directly), so the
//!   exclusivity is structural for the wiring path, not a claim
//!   that no other code in the crate can name `DmaMode::Bypass`.
//! - A missing establish step, a missing verify step, or a
//!   verification mismatch all refuse DMA, and a mismatch fails
//!   the gate closed (later `dma_mode` calls keep refusing).
//! - `Translated` never yields a bus address by itself: the
//!   caller must still pass an explicit translated bus through
//!   `plan_dma`, which refuses without one (part-1 gate).

use crate::physmap_plan::{plan_dma, DmaMode, DmaPlan, PlanError};

/// DWC3 stream ID in `apps_smmu` (`sdm845.dtsi:4236`).
pub const DWC3_SID: u16 = 0x740;
/// `apps_smmu` MMIO base (`sdm845.dtsi`, iommu@15000000).
pub const APPS_SMMU_BASE: u64 = 0x1500_0000;
/// Stream-matching register groups (dmesg, 2026-10-30).
pub const APPS_SMMU_SMR_COUNT: usize = 76;
/// Context banks, all stage-1 capable (dmesg, 2026-10-30).
pub const APPS_SMMU_CONTEXT_BANKS: u8 = 45;

/// S2CR TYPE field values (ARM SMMUv2).
pub const S2CR_TYPE_TRANS: u32 = 0;
pub const S2CR_TYPE_BYPASS: u32 = 1;
pub const S2CR_TYPE_FAULT: u32 = 2;

/// Does raw SMR value `smr` match stream `sid`?
/// VALID must be set; ID/MASK compare per the architecture.
pub fn smr_matches(smr: u32, sid: u16) -> bool {
    if smr & (1 << 31) == 0 {
        return false;
    }
    let id = (smr & 0xffff) as u16;
    let mask = ((smr >> 16) & 0x7fff) as u16;
    (id & !mask) == (sid & !mask)
}

/// What a platform probe observed for SID 0x740's stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidObservation {
    /// No valid SMR matches SID 0x740: the stream would fault.
    NotMatched,
    /// S2CR TYPE = BYPASS for the matching SMR.
    Bypass { smr: usize },
    /// S2CR TYPE = TRANS, routed to `context_bank`.
    Translated { smr: usize, context_bank: u8 },
    /// S2CR TYPE = FAULT (or a reserved TYPE encoding).
    Fault { smr: usize },
}

/// Decode one SMR/S2CR pair into an observation for SID 0x740.
/// `None` when the SMR does not match the SID (the probe scans
/// all `APPS_SMMU_SMR_COUNT` pairs and keeps the matching one;
/// `NotMatched` is what it reports when no pair matches).
pub fn observation_from_registers(smr_index: usize, smr: u32, s2cr: u32) -> Option<SidObservation> {
    if smr_index >= APPS_SMMU_SMR_COUNT {
        return None;
    }
    if !smr_matches(smr, DWC3_SID) {
        return None;
    }
    let ty = (s2cr >> 16) & 0x3;
    let cbndx = (s2cr & 0xff) as u8;
    Some(match ty {
        S2CR_TYPE_TRANS => SidObservation::Translated {
            smr: smr_index,
            context_bank: cbndx,
        },
        S2CR_TYPE_BYPASS => SidObservation::Bypass { smr: smr_index },
        // FAULT and reserved encodings both fail closed.
        _ => SidObservation::Fault { smr: smr_index },
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateError {
    /// `dma_mode` called before any establish step.
    Unestablished,
    /// Established (observed once) but not yet verified.
    Unverified,
    /// The verify observation disagreed with the established
    /// one; the gate has failed closed.
    VerificationMismatch,
    /// establish/verify called out of order.
    OutOfOrder,
    /// Verified: no SMR matches SID 0x740.
    SidNotMatched,
    /// Verified: SID 0x740's stream is in FAULT state.
    SidFault,
    /// Verified Translated observation names a context bank
    /// the hardware does not have.
    ContextBankOutOfRange,
    /// Observation names an SMR index the hardware does not
    /// have (>= APPS_SMMU_SMR_COUNT).
    SmrOutOfRange,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GateState {
    Unestablished,
    Observed { obs: SidObservation, read_seq: u64 },
    Verified(SidObservation),
    Failed,
}

/// The establish-and-verify sequence for SID 0x740.
///
/// Protocol: `establish(obs, seq)` records the platform
/// probe's first full observation, tagged with the probe's
/// read sequence number; `verify(obs, seq)` requires a second
/// observation from a LATER, distinct read (`seq` strictly
/// greater — replaying the same read is a mismatch) that
/// agrees EXACTLY (same variant, same SMR index, same context
/// bank). Only then will `dma_mode()` answer — and the answer
/// for anything other than a verified Bypass/Translated
/// observation is a refusal.
#[derive(Debug)]
pub struct SidGate {
    state: GateState,
}

impl Default for SidGate {
    fn default() -> Self {
        Self::new()
    }
}

impl SidGate {
    pub fn new() -> Self {
        SidGate {
            state: GateState::Unestablished,
        }
    }

    /// Step 1 — record the established stream state from probe
    /// read `read_seq`.
    pub fn establish(&mut self, obs: SidObservation, read_seq: u64) -> Result<(), GateError> {
        match self.state {
            GateState::Unestablished => {
                self.state = GateState::Observed { obs, read_seq };
                Ok(())
            }
            _ => Err(GateError::OutOfOrder),
        }
    }

    /// Step 2 — an independent re-observation must agree
    /// exactly. A mismatch fails the gate closed: the state is
    /// latched to `Failed` and `dma_mode` keeps refusing, so a
    /// flapping stream can never be talked into DMA.
    pub fn verify(&mut self, obs: SidObservation, read_seq: u64) -> Result<(), GateError> {
        match self.state {
            GateState::Observed {
                obs: prev,
                read_seq: prev_seq,
            } => {
                // Independence, pinned: the verify observation
                // must come from a later, distinct probe read.
                // Replaying the establish read (same or older
                // sequence) is a mismatch, not a verification.
                if read_seq <= prev_seq {
                    self.state = GateState::Failed;
                    return Err(GateError::VerificationMismatch);
                }
                if prev != obs {
                    self.state = GateState::Failed;
                    return Err(GateError::VerificationMismatch);
                }
                let smr = match obs {
                    SidObservation::Bypass { smr }
                    | SidObservation::Translated { smr, .. }
                    | SidObservation::Fault { smr } => Some(smr),
                    SidObservation::NotMatched => None,
                };
                if let Some(smr) = smr {
                    if smr >= APPS_SMMU_SMR_COUNT {
                        self.state = GateState::Failed;
                        return Err(GateError::SmrOutOfRange);
                    }
                }
                if let SidObservation::Translated { context_bank, .. } = obs {
                    if context_bank >= APPS_SMMU_CONTEXT_BANKS {
                        self.state = GateState::Failed;
                        return Err(GateError::ContextBankOutOfRange);
                    }
                }
                self.state = GateState::Verified(obs);
                Ok(())
            }
            _ => Err(GateError::OutOfOrder),
        }
    }

    /// The DMA mode the verified stream state permits.
    /// This is the ONLY way payload-v2 wiring may obtain a
    /// `DmaMode` for real hardware: identity DMA (`Bypass`)
    /// comes exclusively from a verified Bypass observation.
    pub fn dma_mode(&self) -> Result<DmaMode, GateError> {
        match self.state {
            GateState::Unestablished => Err(GateError::Unestablished),
            GateState::Observed { .. } => Err(GateError::Unverified),
            GateState::Failed => Err(GateError::VerificationMismatch),
            GateState::Verified(SidObservation::NotMatched) => Err(GateError::SidNotMatched),
            GateState::Verified(SidObservation::Fault { .. }) => Err(GateError::SidFault),
            GateState::Verified(SidObservation::Bypass { .. }) => Ok(DmaMode::Bypass),
            GateState::Verified(SidObservation::Translated { .. }) => Ok(DmaMode::Translated),
        }
    }

    /// The verified DMA mode as an opaque token: the only
    /// value payload-v2 wiring may turn into a DMA plan (via
    /// `plan_dma_gated`). Its field is private and `SidGate`
    /// is its only source, so an unverified or refused state
    /// cannot be dressed up as verified.
    pub fn verified_mode(&self) -> Result<VerifiedMode, GateError> {
        Ok(VerifiedMode {
            mode: self.dma_mode()?,
        })
    }
}

/// A `DmaMode` vouched for by a verified `SidGate`. Opaque:
/// constructible only inside this module, from a gate that
/// completed establish + verify.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerifiedMode {
    mode: DmaMode,
}

impl VerifiedMode {
    /// The gate-verified DMA mode.
    pub fn mode(&self) -> DmaMode {
        self.mode
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatedPlanError {
    Gate(GateError),
    Plan(PlanError),
}

/// The payload-v2 composition point: a DMA plan exists only
/// if `gate` has verified the SID 0x740 stream state, and the
/// plan itself still passes the part-1 `plan_dma` checks (a
/// verified Translated stream without an explicit bus is
/// refused there — identity DMA is never substituted).
pub fn plan_dma_gated(
    gate: &SidGate,
    phys: u64,
    translated_bus: Option<u64>,
) -> Result<DmaPlan, GatedPlanError> {
    let verified = gate.verified_mode().map_err(GatedPlanError::Gate)?;
    plan_dma(phys, verified.mode(), translated_bus).map_err(GatedPlanError::Plan)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physmap_plan::{plan_dma, PlanError};

    fn smr_raw(id: u16, mask: u16) -> u32 {
        (1u32 << 31) | ((mask as u32 & 0x7fff) << 16) | id as u32
    }
    fn s2cr_raw(ty: u32, cbndx: u8) -> u32 {
        (ty << 16) | cbndx as u32
    }
    fn verified(obs: SidObservation) -> SidGate {
        let mut g = SidGate::new();
        g.establish(obs, 1).unwrap();
        g.verify(obs, 2).unwrap();
        g
    }

    #[test]
    fn smmu_hardware_facts_pinned() {
        // First-hand pmOS facts, 2026-10-30 (dmesg/sysfs, read-only):
        // apps_smmu at 0x1500_0000, 76 stream-matching groups, 45
        // context banks. The probe scan and the context-bank range
        // check are sized from these numbers; if a future capture
        // disagrees, this pin is the thing to revisit first.
        assert_eq!(APPS_SMMU_BASE, 0x1500_0000);
        assert_eq!(APPS_SMMU_SMR_COUNT, 76);
        assert_eq!(APPS_SMMU_CONTEXT_BANKS, 45);
        assert_eq!(DWC3_SID, 0x740);
    }

    #[test]
    fn smr_match_exact_masked_and_invalid() {
        assert!(smr_matches(smr_raw(0x740, 0), DWC3_SID));
        assert!(!smr_matches(smr_raw(0x741, 0), DWC3_SID));
        // Masked: ID 0x740 with mask 0xf matches the 0x740-0x74f
        // range, including 0x740 itself.
        assert!(smr_matches(smr_raw(0x740, 0xf), DWC3_SID));
        // VALID clear never matches, whatever ID says.
        assert!(!smr_matches(0x740, DWC3_SID));
    }

    #[test]
    fn decode_bypass_translated_fault_and_nonmatching() {
        assert_eq!(
            observation_from_registers(9, smr_raw(0x740, 0), s2cr_raw(S2CR_TYPE_BYPASS, 0)),
            Some(SidObservation::Bypass { smr: 9 })
        );
        assert_eq!(
            observation_from_registers(9, smr_raw(0x740, 0), s2cr_raw(S2CR_TYPE_TRANS, 12)),
            Some(SidObservation::Translated {
                smr: 9,
                context_bank: 12
            })
        );
        assert_eq!(
            observation_from_registers(9, smr_raw(0x740, 0), s2cr_raw(S2CR_TYPE_FAULT, 0)),
            Some(SidObservation::Fault { smr: 9 })
        );
        // A different stream's pair decodes to nothing here.
        assert_eq!(
            observation_from_registers(9, smr_raw(0x100, 0), s2cr_raw(S2CR_TYPE_BYPASS, 0)),
            None
        );
        // Reserved S2CR TYPE 3 fails closed as Fault (Wonder
        // diff review of 1ee8945: only TYPE 2 was pinned).
        assert_eq!(
            observation_from_registers(9, smr_raw(0x740, 0), s2cr_raw(3, 0)),
            Some(SidObservation::Fault { smr: 9 })
        );
        // An SMR index the hardware does not have decodes to
        // nothing, even with a matching ID.
        assert_eq!(
            observation_from_registers(
                APPS_SMMU_SMR_COUNT,
                smr_raw(0x740, 0),
                s2cr_raw(S2CR_TYPE_BYPASS, 0)
            ),
            None
        );
    }

    #[test]
    fn missing_sid_step_is_refused() {
        // Negative case (Wonder payload-v2 spec): DMA requested
        // with NO establish/verify step at all must refuse.
        let g = SidGate::new();
        assert_eq!(g.dma_mode(), Err(GateError::Unestablished));
    }

    #[test]
    fn establish_without_verify_is_refused() {
        let mut g = SidGate::new();
        g.establish(SidObservation::Bypass { smr: 9 }, 1).unwrap();
        assert_eq!(g.dma_mode(), Err(GateError::Unverified));
    }

    #[test]
    fn verify_mismatch_fails_closed() {
        let mut g = SidGate::new();
        g.establish(SidObservation::Bypass { smr: 9 }, 1).unwrap();
        assert_eq!(
            g.verify(
                SidObservation::Translated {
                    smr: 9,
                    context_bank: 12
                },
                2
            ),
            Err(GateError::VerificationMismatch)
        );
        // Latched: even though the first observation was Bypass,
        // identity DMA stays refused after the mismatch.
        assert_eq!(g.dma_mode(), Err(GateError::VerificationMismatch));
    }

    #[test]
    fn out_of_order_calls_are_refused() {
        let mut g = SidGate::new();
        assert_eq!(
            g.verify(SidObservation::Bypass { smr: 9 }, 1),
            Err(GateError::OutOfOrder)
        );
        let mut g = verified(SidObservation::Bypass { smr: 9 });
        assert_eq!(
            g.establish(SidObservation::Bypass { smr: 9 }, 3),
            Err(GateError::OutOfOrder)
        );
    }

    #[test]
    fn replayed_read_cannot_verify() {
        // Independence, pinned (Wonder diff review of 1ee8945):
        // the SAME probe read (same sequence number) submitted
        // as both establish and verify is a mismatch, and the
        // gate fails closed — one read can never verify itself.
        let mut g = SidGate::new();
        g.establish(SidObservation::Bypass { smr: 9 }, 7).unwrap();
        assert_eq!(
            g.verify(SidObservation::Bypass { smr: 9 }, 7),
            Err(GateError::VerificationMismatch)
        );
        assert_eq!(g.dma_mode(), Err(GateError::VerificationMismatch));
        // An older read is equally unusable as verification.
        let mut g = SidGate::new();
        g.establish(SidObservation::Bypass { smr: 9 }, 7).unwrap();
        assert_eq!(
            g.verify(SidObservation::Bypass { smr: 9 }, 3),
            Err(GateError::VerificationMismatch)
        );
    }

    #[test]
    fn smr_index_out_of_range_is_refused() {
        // Wonder diff review of 1ee8945: Bypass { smr: 76 }
        // (== APPS_SMMU_SMR_COUNT) must NOT yield identity DMA.
        let mut g = SidGate::new();
        let obs = SidObservation::Bypass {
            smr: APPS_SMMU_SMR_COUNT,
        };
        g.establish(obs, 1).unwrap();
        assert_eq!(g.verify(obs, 2), Err(GateError::SmrOutOfRange));
        assert_eq!(g.dma_mode(), Err(GateError::VerificationMismatch));
    }

    #[test]
    fn gated_plan_requires_verified_gate() {
        // Exclusivity composition (Wonder diff review of
        // 1ee8945): the wiring composition point refuses an
        // unverified gate, and serves a verified one.
        let g = SidGate::new();
        assert_eq!(
            plan_dma_gated(&g, 0x8000_0000, None),
            Err(GatedPlanError::Gate(GateError::Unestablished))
        );
        let g = verified(SidObservation::Translated {
            smr: 9,
            context_bank: 12,
        });
        assert_eq!(
            plan_dma_gated(&g, 0x8000_0000, None),
            Err(GatedPlanError::Plan(PlanError::MissingTranslatedBus))
        );
        let plan = plan_dma_gated(&g, 0x8000_0000, Some(0x9000_0000)).unwrap();
        assert_eq!(plan.bus(), 0x9000_0000);
        let g = verified(SidObservation::Bypass { smr: 9 });
        let plan = plan_dma_gated(&g, 0x8000_0000, None).unwrap();
        assert_eq!(plan.bus(), plan.phys());
    }

    #[test]
    fn verified_bypass_yields_identity_dma() {
        let g = verified(SidObservation::Bypass { smr: 9 });
        let mode = g.dma_mode().unwrap();
        assert_eq!(mode, DmaMode::Bypass);
        let plan = plan_dma(0x8000_0000, mode, None).unwrap();
        assert_eq!(plan.bus(), plan.phys());
    }

    #[test]
    fn verified_translated_never_yields_identity_dma() {
        // The V3-oracle case: the stream is translated, so even
        // with a known physical address the plan refuses until
        // an explicit translated bus is supplied.
        let g = verified(SidObservation::Translated {
            smr: 9,
            context_bank: 12,
        });
        let mode = g.dma_mode().unwrap();
        assert_eq!(mode, DmaMode::Translated);
        assert_eq!(
            plan_dma(0x8000_0000, mode, None),
            Err(PlanError::MissingTranslatedBus)
        );
        let plan = plan_dma(0x8000_0000, mode, Some(0x9000_0000)).unwrap();
        assert_eq!(plan.phys(), 0x8000_0000);
        assert_eq!(plan.bus(), 0x9000_0000);
        assert_ne!(plan.bus(), plan.phys());
    }

    #[test]
    fn verified_not_matched_and_fault_are_refused() {
        let g = verified(SidObservation::NotMatched);
        assert_eq!(g.dma_mode(), Err(GateError::SidNotMatched));
        let g = verified(SidObservation::Fault { smr: 9 });
        assert_eq!(g.dma_mode(), Err(GateError::SidFault));
    }

    #[test]
    fn translated_context_bank_out_of_range_fails_closed() {
        let mut g = SidGate::new();
        let obs = SidObservation::Translated {
            smr: 9,
            context_bank: APPS_SMMU_CONTEXT_BANKS,
        };
        g.establish(obs, 1).unwrap();
        assert_eq!(g.verify(obs, 2), Err(GateError::ContextBankOutOfRange));
        assert_eq!(g.dma_mode(), Err(GateError::VerificationMismatch));
    }
}
