//! Accepted26 `private_rd` (extracted from the pinned fixture) vs current wrapper: same
//! Option result, same captured log lines in order, same MPIDR read count.
use crate::generated::{baseline, current};
use crate::model::RdRegistry;
use crate::shim;

const RD0: usize = 0x080a_0000;
const RD1: usize = RD0 + 0x2_0000;

type Observed = (Option<usize>, Vec<String>, usize);

fn registry(entries: &[(u32, usize)]) -> RdRegistry {
    let mut reg = RdRegistry::new();
    for &(aff, rd) in entries {
        reg.insert(aff, rd);
    }
    reg
}

fn observe(call: impl Fn() -> Option<usize>, mpidr: u64) -> Observed {
    shim::reset(mpidr);
    let out = call();
    (out, shim::take_logs(), shim::mpidr_reads())
}

const MAPS: &[&[(u32, usize)]] = &[&[], &[(0, RD0)], &[(0, RD0), (0x0700_0001, RD1)], &[(0x1, RD1)]];
const MPIDRS: &[u64] = &[
    0x8000_0000,
    0x8000_0001,
    0x8000_0001 | (1 << 30) | (1 << 24),
    (7 << 32) | 0x8000_0001,
    (0xff << 32) | 0x80ff_ffff,
];
const INTIDS: &[u32] = &[0, 15, 16, 27, 30, 31];

fn compare(filter: impl Fn(&Observed) -> bool) -> usize {
    let mut cases = 0;
    for map in MAPS {
        let b = baseline::GicV3 { rd_registry: registry(map) };
        let c = current::GicV3 { rd_registry: registry(map) };
        for &mpidr in MPIDRS {
            for &intid in INTIDS {
                let want = observe(|| b.call_private_rd(intid), mpidr);
                if !filter(&want) {
                    continue;
                }
                let got = observe(|| c.call_private_rd(intid), mpidr);
                assert_eq!(got, want, "map={map:x?} mpidr={mpidr:#x} intid={intid}");
                cases += 1;
            }
        }
    }
    cases
}

#[test]
fn parity_mapped_cases() {
    let n = compare(|o| o.0.is_some());
    assert!(n > 0);
    eprintln!("parity_mapped_cases: {n}");
}

#[test]
fn parity_missing_cases() {
    let n = compare(|o| o.0.is_none());
    assert!(n > 0);
    eprintln!("parity_missing_cases: {n}");
}

#[test]
fn baseline_legacy_text_is_the_expected_literal() {
    let b = baseline::GicV3 { rd_registry: registry(&[]) };
    let o = observe(|| b.call_private_rd(30), (7 << 32) | 0x8000_0001);
    assert_eq!(o.0, None);
    assert_eq!(o.1, ["gicv3: private INTID 30 on affinity 0x7000001 has no redistributor mapping"]);
    assert_eq!(o.2, 1);
}
