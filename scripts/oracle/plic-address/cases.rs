//! Case tables, runner and spec-derived oracle shared by the parity tests.

use core::num::NonZero;
use plic_address_harness::hook::{self, Access};
use plic_address_harness::{Accessor, Adapter, PlicOps};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Once;

pub const WORDS: usize = 0x24_0000 / 4;
pub const SEEDS: [u32; 4] = [0, u32::MAX, 0xA5A5_5A5A, 0x5A5A_A5A5];
pub const UNWRAP_NONE: &str = "called `Option::unwrap()` on a `None` value";

// Spec formulas from the task, independent of either source.
pub fn prio_off(irq: usize) -> usize { 4 * irq }
pub fn pending_off(lane: usize) -> usize { 0x1000 + 4 * lane }
pub fn enable_off(ctx: usize, lane: usize) -> usize { 0x2000 + 4 * (ctx * 32 + lane) }
pub fn thresh_off(ctx: usize) -> usize { 0x20_0000 + 0x1000 * ctx }
pub fn claim_off(ctx: usize) -> usize { thresh_off(ctx) + 4 }

#[derive(Clone, Debug, PartialEq)]
pub struct Outcome {
    pub trace: Vec<Access>,
    pub result: Result<String, String>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Setup { PreInit, Null, Installed }

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Op { Ack, Eoi(u32), Enable(u32), Disable(u32), InitTail }

#[derive(Clone, Copy, Debug)]
pub struct Case { pub setup: Setup, pub ndev: usize, pub ctx: usize, pub seed: u32, pub op: Op }

#[derive(Clone, Copy, Debug)]
pub enum AccOp {
    SetPriority(usize, usize),
    Pending(usize, u32),
    Enable(usize, usize, bool, u32),
    Threshold(usize, usize),
    Claim(usize, u32),
    Complete(usize, usize),
}

thread_local! {
    static IN_CASE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Silences panics raised inside a traced case; test assertion panics still print.
fn quiet() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let default = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if !IN_CASE.with(|c| c.get()) {
                default(info)
            }
        }))
    });
}

fn message(e: Box<dyn std::any::Any + Send>) -> String {
    match e.downcast::<String>() {
        Ok(s) => *s,
        Err(e) => e.downcast::<&str>().map(|s| s.to_string()).unwrap_or_else(|_| "<non-string panic>".into()),
    }
}

/// Seeds a fresh register buffer, then traces only `f`'s volatile accesses.
fn run(seeds: &[(usize, u32)], f: impl FnOnce(usize) -> String) -> Outcome {
    quiet();
    let mut mem = vec![0u32; WORDS];
    for (i, w) in mem.iter_mut().enumerate() {
        *w = (i as u32).wrapping_mul(0x9E37_79B9) ^ 0x3C3C_C3C3;
    }
    for &(off, v) in seeds {
        if off / 4 < WORDS {
            mem[off / 4] = v;
        }
    }
    let base = mem.as_mut_ptr() as usize;
    hook::reset(base);
    IN_CASE.with(|c| c.set(true));
    let result = catch_unwind(AssertUnwindSafe(|| f(base))).map_err(message);
    IN_CASE.with(|c| c.set(false));
    let trace = hook::take();
    drop(mem);
    Outcome { trace, result }
}

fn case_seeds(c: &Case) -> Vec<(usize, u32)> {
    let mut s = vec![(claim_off(c.ctx), c.seed)];
    if let Op::Enable(irq) | Op::Disable(irq) = c.op {
        s.push((enable_off(c.ctx, irq as usize / 32), c.seed));
    }
    s
}

pub fn run_plic<P: Adapter + PlicOps>(c: &Case) -> Outcome {
    let c = *c;
    run(&case_seeds(&c), move |base| {
        let mut p = P::fresh();
        match c.setup {
            Setup::PreInit => {}
            Setup::Null => p.install(0),
            Setup::Installed => p.install(base),
        }
        p.set_fields(c.ndev, c.ctx);
        match c.op {
            Op::Ack => format!("{}", p.irq_ack()),
            Op::Eoi(i) => { p.irq_eoi(i); "()".into() }
            Op::Enable(i) => { p.irq_enable(i); "()".into() }
            Op::Disable(i) => { p.irq_disable(i); "()".into() }
            Op::InitTail => { p.init_tail(); "()".into() }
        }
    })
}

pub fn run_acc<A: Accessor>(op: &AccOp) -> Outcome {
    let op = *op;
    let seeds = match op {
        AccOp::Pending(l, s) => vec![(pending_off(l), s)],
        AccOp::Enable(c, irq, _, s) => vec![(enable_off(c, irq / 32), s)],
        AccOp::Claim(c, s) => vec![(claim_off(c), s)],
        _ => vec![],
    };
    run(&seeds, move |base| {
        let mut a = unsafe { A::at(base) };
        let nz = |v: usize| NonZero::new(v).unwrap();
        match op {
            AccOp::SetPriority(i, p) => { a.set_priority(i, p); "()".into() }
            AccOp::Pending(l, _) => format!("{}", a.pending(l)),
            AccOp::Enable(c, i, e, _) => { a.enable(c, nz(i), e); "()".into() }
            AccOp::Threshold(c, p) => { a.set_priority_threshold(c, p); "()".into() }
            AccOp::Claim(c, _) => format!("{:?}", a.claim(c).map(|v| v.get())),
            AccOp::Complete(c, v) => { a.complete(c, nz(v)); "()".into() }
        }
    })
}

pub fn plic_cases() -> Vec<Case> {
    let mut v = Vec::new();
    let mut push = |setup, ndev, ctx, seed, op| v.push(Case { setup, ndev, ctx, seed, op });
    for setup in [Setup::PreInit, Setup::Null] {
        for ndev in [0, 1023] {
            for op in [Op::Ack, Op::Eoi(1), Op::Enable(1), Op::Disable(1), Op::InitTail] {
                push(setup, ndev, 0, 7, op);
            }
        }
    }
    for ctx in [0, 1, 63] {
        push(Setup::Installed, 1023, ctx, 0, Op::InitTail);
        for seed in [0, 1, 31, 32, 1023, 0x8000_0000, u32::MAX] {
            push(Setup::Installed, 1023, ctx, seed, Op::Ack);
        }
        for irq in [0, 1, 31, 32, 1023, 1024, u32::MAX] {
            push(Setup::Installed, 1023, ctx, 9, Op::Eoi(irq));
        }
        for irq in [0, 1, 31, 32, 1023, 1024] {
            for seed in SEEDS {
                push(Setup::Installed, 1023, ctx, seed, Op::Enable(irq));
                push(Setup::Installed, 1023, ctx, seed, Op::Disable(irq));
            }
        }
    }
    for op in [Op::Enable(1024), Op::Disable(1024)] {
        push(Setup::Installed, 1024, 0, 0, op);
    }
    for op in [Op::InitTail, Op::Ack, Op::Eoi(1), Op::Enable(1), Op::Disable(1)] {
        push(Setup::Installed, 1023, 64, 5, op);
    }
    v
}

pub fn acc_cases() -> Vec<AccOp> {
    let mut v = Vec::new();
    for irq in [0, 1, 31, 32, 1023, 1024] {
        for p in [0, 1, 7, 8] {
            v.push(AccOp::SetPriority(irq, p));
        }
    }
    for lane in [0, 1, 31, 32] {
        v.push(AccOp::Pending(lane, 0xDEAD_0000 | lane as u32));
    }
    for ctx in [0, 1, 63, 64] {
        for irq in [1, 31, 32, 1023, 1024] {
            for seed in SEEDS {
                v.push(AccOp::Enable(ctx, irq, true, seed));
                v.push(AccOp::Enable(ctx, irq, false, seed));
            }
        }
        for p in [0, 7, 8] {
            v.push(AccOp::Threshold(ctx, p));
        }
        for seed in [0, 5, u32::MAX] {
            v.push(AccOp::Claim(ctx, seed));
        }
        for val in [1, 1023, 0x1_0000_0001, usize::MAX] {
            v.push(AccOp::Complete(ctx, val));
        }
    }
    v
}

pub fn r(off: usize, val: u32) -> Access { Access::R { off, bits: 32, val: val as u64 } }
pub fn w(off: usize, val: u32) -> Access { Access::W { off, bits: 32, val: val as u64 } }

pub fn panicked_with(o: &Outcome, needle: &str) -> bool {
    matches!(&o.result, Err(m) if m.contains(needle))
}
