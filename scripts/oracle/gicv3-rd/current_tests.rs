//! Tests against the CURRENT extracted items (child module, so private
//! `MissingRd` fields and `lookup_private_rd` are reachable as in-source).
use super::*;
use crate::shim;

const RD0: usize = 0x080a_0000;
const RD1: usize = RD0 + 0x2_0000;
/// Aff3=7, RES1 (bit 31), MT (bit 24), Aff0=1: packs to 0x0700_0001.
const MPIDR_HI: u64 = (7 << 32) | (1 << 31) | (1 << 24) | 0x1;
const AFF_HI: u32 = 0x0700_0001;

fn registry(entries: &[(u32, usize)]) -> RdRegistry {
    let mut reg = RdRegistry::new();
    for &(aff, rd) in entries {
        reg.insert(aff, rd);
    }
    reg
}

fn gic(entries: &[(u32, usize)]) -> GicV3 {
    GicV3 { rd_registry: registry(entries) }
}

#[test]
fn helper_mapped_returns_rd_without_log() {
    shim::reset(0);
    let reg = registry(&[(0, RD0), (AFF_HI, RD1)]);
    assert_eq!(lookup_private_rd(&reg, 30, 0), Ok(RD0));
    assert_eq!(lookup_private_rd(&reg, 30, AFF_HI), Ok(RD1));
    assert!(shim::take_logs().is_empty());
    assert_eq!(shim::mpidr_reads(), 0);
}

#[test]
fn helper_missing_returns_owned_diagnostic_without_log() {
    shim::reset(0);
    let reg = registry(&[(AFF_HI, RD1)]);
    assert_eq!(lookup_private_rd(&reg, 27, 0x1), Err(MissingRd { intid: 27, affinity: 0x1 }));
    assert!(shim::take_logs().is_empty());
    assert_eq!(shim::mpidr_reads(), 0);
}

#[test]
fn helper_preserves_intid_and_affinity_in_diagnostic() {
    shim::reset(0);
    let reg = registry(&[(0, RD0)]);
    for intid in [0, 15, 16, 27, 30, 31, u32::MAX] {
        for aff in [1, 0xff, 0x0100_0000, AFF_HI, 0xff00_0000, 0xffff_ffff] {
            let d = lookup_private_rd(&reg, intid, aff).unwrap_err();
            assert_eq!((d.intid, d.affinity), (intid, aff));
        }
    }
    assert!(shim::take_logs().is_empty());
}

#[test]
fn diagnostic_survives_registry_mutation() {
    let mut reg = registry(&[(0, RD0)]);
    let d = lookup_private_rd(&reg, 30, 1).unwrap_err();
    reg.insert(1, RD1);
    assert_eq!(lookup_private_rd(&reg, 30, 1), Ok(RD1));
    reg.clear();
    drop(reg);
    assert_eq!(d, MissingRd { intid: 30, affinity: 1 });
}

#[test]
fn missing_rd_is_owned_static_copy() {
    fn owned<T: 'static + Copy + Send + Sync>() {}
    owned::<MissingRd>();
    assert_eq!(core::mem::size_of::<MissingRd>(), 8);
}

#[test]
fn wrapper_mapped_returns_some_without_log() {
    let g = gic(&[(AFF_HI, RD1), (0, RD0)]);
    shim::reset(MPIDR_HI);
    assert_eq!(g.private_rd(30), Some(RD1));
    assert!(shim::take_logs().is_empty());
    assert_eq!(shim::mpidr_reads(), 1);
    shim::reset(0x8000_0000);
    assert_eq!(g.private_rd(16), Some(RD0));
    assert!(shim::take_logs().is_empty());
    assert_eq!(shim::mpidr_reads(), 1);
}

#[test]
fn wrapper_missing_logs_exact_legacy_line_once() {
    let g = gic(&[(0, RD0)]);
    shim::reset(MPIDR_HI);
    assert_eq!(g.private_rd(30), None);
    assert_eq!(
        shim::take_logs(),
        ["gicv3: private INTID 30 on affinity 0x7000001 has no redistributor mapping"]
    );
    assert_eq!(shim::mpidr_reads(), 1);
}

#[test]
fn wrapper_logs_in_call_order() {
    let g = gic(&[(0, RD0)]);
    shim::reset(0x8000_0001);
    assert_eq!(g.private_rd(27), None);
    assert_eq!(g.private_rd(3), None);
    shim::reset_mpidr_only(0x8000_0000);
    assert_eq!(g.private_rd(30), Some(RD0));
    assert_eq!(
        shim::take_logs(),
        [
            "gicv3: private INTID 27 on affinity 0x1 has no redistributor mapping",
            "gicv3: private INTID 3 on affinity 0x1 has no redistributor mapping",
        ]
    );
}

#[test]
fn wrapper_uses_packed_mpidr_affinity() {
    let g = gic(&[(0x1, RD1)]);
    // U (30), MT (24) and RES1 (31) are dropped by mpidr_affinity.
    shim::reset(0x8000_0001 | (1 << 30) | (1 << 24));
    assert_eq!(g.private_rd(30), Some(RD1));
    assert!(shim::take_logs().is_empty());
    // Aff3 is kept: same low bits, different CPU.
    shim::reset((7 << 32) | 0x8000_0001);
    assert_eq!(g.private_rd(30), None);
    assert_eq!(
        shim::take_logs(),
        ["gicv3: private INTID 30 on affinity 0x7000001 has no redistributor mapping"]
    );
}
