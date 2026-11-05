//! Old-vs-new trace parity plus spec-oracle checks applied to BOTH sources.
mod common;

use common::*;
use plic_address_harness::hook::Access;
use plic_address_harness::{new, old};
use std::io::Write;

fn both(c: &Case) -> [(&'static str, Outcome); 2] {
    [("old", run_plic::<old::Plic>(c)), ("new", run_plic::<new::Plic>(c))]
}

fn dump(name: &str, lines: &[String]) {
    if let Ok(dir) = std::env::var("PLIC_TRACE_DIR") {
        let mut f = std::fs::File::create(format!("{dir}/{name}.txt")).unwrap();
        for l in lines {
            writeln!(f, "{l}").unwrap();
        }
    }
}

#[test]
fn parity_plic_methods() {
    let cases = plic_cases();
    let (mut accesses, mut panics, mut lines) = (0, 0, Vec::new());
    for c in &cases {
        let [(_, o), (_, n)] = both(c);
        lines.push(format!("{c:?}\n  old={o:?}\n  new={n:?}"));
        assert_eq!(o, n, "PARITY plic_methods: {c:?}");
        assert!(o.trace.iter().all(|a| matches!(a, Access::R { bits: 32, .. } | Access::W { bits: 32, .. })), "WIDTH32: {c:?}");
        accesses += o.trace.len();
        panics += o.result.is_err() as usize;
    }
    dump("plic-methods", &lines);
    println!("PLIC_METHOD_CASES={} accesses={} panicking={}", cases.len(), accesses, panics);
}

#[test]
fn parity_accessors() {
    let cases = acc_cases();
    let (mut accesses, mut panics, mut lines) = (0, 0, Vec::new());
    for c in &cases {
        let o = run_acc::<old::PlicAcc>(c);
        let n = run_acc::<new::PlicAcc>(c);
        lines.push(format!("{c:?}\n  old={o:?}\n  new={n:?}"));
        assert_eq!(o, n, "PARITY accessors: {c:?}");
        assert!(o.trace.iter().all(|a| matches!(a, Access::R { bits: 32, .. } | Access::W { bits: 32, .. })), "WIDTH32: {c:?}");
        accesses += o.trace.len();
        panics += o.result.is_err() as usize;
    }
    dump("accessors", &lines);
    println!("ACCESSOR_CASES={} accesses={} panicking={}", cases.len(), accesses, panics);
}

#[test]
fn oracle_context_stride() {
    for ctx in [0, 1, 63] {
        let init = Case { setup: Setup::Installed, ndev: 1023, ctx, seed: 0, op: Op::InitTail };
        let ack = Case { op: Op::Ack, seed: 3, ..init };
        let eoi = Case { op: Op::Eoi(3), ..init };
        for (who, o) in both(&init) {
            assert_eq!(o.trace, vec![w(thresh_off(ctx), 0)], "CONTEXT_STRIDE threshold {who} ctx={ctx}");
        }
        for (who, o) in both(&ack) {
            assert_eq!(o.trace, vec![r(claim_off(ctx), 3)], "CONTEXT_STRIDE claim {who} ctx={ctx}");
        }
        for (who, o) in both(&eoi) {
            assert_eq!(o.trace, vec![w(claim_off(ctx), 3)], "CONTEXT_STRIDE complete {who} ctx={ctx}");
        }
    }
}

#[test]
fn oracle_claim_read_once_and_zero_claim_panics() {
    for c in plic_cases().iter().filter(|c| c.op == Op::Ack && c.setup == Setup::Installed && c.ctx < 64) {
        for (who, o) in both(c) {
            assert_eq!(o.trace, vec![r(claim_off(c.ctx), c.seed)], "CLAIM_READ_ONCE {who} {c:?}");
            if c.seed == 0 {
                assert!(panicked_with(&o, UNWRAP_NONE), "ZERO_CLAIM_PANIC {who} {c:?} {o:?}");
            } else {
                assert_eq!(o.result, Ok(c.seed.to_string()), "CLAIM_RETURN {who} {c:?}");
            }
        }
    }
}

#[test]
fn oracle_complete_writes_once() {
    for c in plic_cases().iter().filter(|c| matches!(c.op, Op::Eoi(_)) && c.setup == Setup::Installed && c.ctx < 64) {
        let Op::Eoi(irq) = c.op else { unreachable!() };
        for (who, o) in both(c) {
            if irq == 0 {
                assert!(o.trace.is_empty() && panicked_with(&o, UNWRAP_NONE), "EOI_ZERO_NO_WRITE {who} {o:?}");
            } else {
                assert_eq!(o.trace, vec![w(claim_off(c.ctx), irq)], "COMPLETE_WRITE_ONCE {who} {c:?}");
            }
        }
    }
}

#[test]
fn oracle_enable_disable_priority_then_single_rmw() {
    for c in plic_cases().iter().filter(|c| c.setup == Setup::Installed && c.ctx < 64 && c.ndev == 1023) {
        let (irq, on) = match c.op { Op::Enable(i) => (i, true), Op::Disable(i) => (i, false), _ => continue };
        if irq == 0 || irq > 1023 {
            continue;
        }
        let (lane, bit) = (irq as usize / 32, 1u32 << (irq % 32));
        let want = if on { c.seed | bit } else { c.seed & !bit };
        for (who, o) in both(c) {
            assert_eq!(o.result, Ok("()".into()), "ENABLE_OK {who} {c:?}");
            assert_eq!(o.trace.first(), Some(&w(prio_off(irq as usize), 1)), "PRIORITY_WRITE_FIRST {who} {c:?} {o:?}");
            assert_eq!(o.trace.len(), 3, "SINGLE_RMW {who} {c:?} {o:?}");
            assert_eq!(o.trace[1], r(enable_off(c.ctx, lane), c.seed), "RMW_READ {who} {c:?}");
            assert_eq!(o.trace[2], w(enable_off(c.ctx, lane), want), "RMW_ONLY_TARGET_BIT {who} {c:?} {o:?}");
        }
    }
}

#[test]
fn oracle_preinit_null_and_invalid_fail_before_mmio() {
    for c in plic_cases() {
        let invalid_irq = match c.op {
            Op::Enable(i) | Op::Disable(i) => i == 0 || i as usize > c.ndev || i > 1023,
            _ => false,
        };
        let expect_empty = c.setup != Setup::Installed
            || invalid_irq
            || (c.ctx >= 64 && !matches!(c.op, Op::Enable(_) | Op::Disable(_)));
        for (who, o) in both(&c) {
            if expect_empty {
                assert!(o.result.is_err() && o.trace.is_empty(), "FAIL_BEFORE_MMIO {who} {c:?} {o:?}");
            }
            if c.ctx >= 64 && !invalid_irq && c.setup == Setup::Installed {
                if let Op::Enable(i) | Op::Disable(i) = c.op {
                    assert_eq!(o.trace, vec![w(prio_off(i as usize), 1)], "CTX64_PRIORITY_THEN_ASSERT {who}");
                    assert!(panicked_with(&o, "context < MAX_CONTEXTS"), "CTX64_ASSERT {who} {o:?}");
                }
            }
        }
    }
}

#[test]
fn oracle_accessor_offsets() {
    for op in acc_cases() {
        let o = run_acc::<old::PlicAcc>(&op);
        let n = run_acc::<new::PlicAcc>(&op);
        let want: Option<Vec<Access>> = match op {
            AccOp::SetPriority(i, p) if i > 0 && i <= 1023 && p < 8 => Some(vec![w(prio_off(i), p as u32)]),
            AccOp::Pending(l, s) if l < 32 => Some(vec![r(pending_off(l), s)]),
            AccOp::Threshold(c, p) if c < 64 && p <= 7 => Some(vec![w(thresh_off(c), p as u32)]),
            AccOp::Claim(c, s) if c < 64 => Some(vec![r(claim_off(c), s)]),
            AccOp::Complete(c, v) if c < 64 => Some(vec![w(claim_off(c), v as u32)]),
            AccOp::Enable(c, i, e, s) if c < 64 && i <= 1023 => {
                let b = 1u32 << (i % 32);
                Some(vec![r(enable_off(c, i / 32), s), w(enable_off(c, i / 32), if e { s | b } else { s & !b })])
            }
            _ => None,
        };
        for (who, out) in [("old", &o), ("new", &n)] {
            match &want {
                Some(t) => assert_eq!(&out.trace, t, "ACCESSOR_OFFSETS {who} {op:?}"),
                None => assert!(out.result.is_err() && out.trace.is_empty(), "ACCESSOR_ASSERT_BEFORE_MMIO {who} {op:?} {out:?}"),
            }
        }
    }
}
