//! Fake MMIO + event sink. State is thread-local and disjoint from any Tlmm,
//! so event hooks never alias or mutate the handler's `self`.
use std::cell::RefCell;
use std::collections::HashMap;

pub const FAKE_BASE: usize = 0x4000_0000;
pub const EVENT_BOUND: usize = 1000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ev {
    Read(usize),
    Write(usize, u32),
    Trigger(u16),
}

/// Action applied to fake MMIO when an event for `on_virq` fires.
#[derive(Clone, Copy, Debug)]
pub struct Hook {
    pub on_virq: u16,
    pub status_offset: usize,
    pub set: bool,
}

#[derive(Default)]
pub struct State {
    pub regs: HashMap<usize, u32>,
    pub trace: Vec<Ev>,
    pub hooks: Vec<Hook>,
    pub events: usize,
}

thread_local! {
    pub static STATE: RefCell<State> = RefCell::new(State::default());
}

pub struct CleanLockToken;

pub trait InterruptHandler {
    fn irq_handler(&mut self, irq: u32, token: &mut CleanLockToken);
}

/// Status registers (offset 0xc within a pin page) are modeled write-1-to-clear.
fn is_status(off: usize) -> bool {
    off & 0xfff == 0xc
}

pub unsafe fn read_volatile(p: *const u32) -> u32 {
    let off = p as usize - FAKE_BASE;
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.trace.push(Ev::Read(off));
        s.regs.get(&off).copied().unwrap_or(0)
    })
}

pub unsafe fn write_volatile(p: *mut u32, v: u32) {
    let off = p as usize - FAKE_BASE;
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.trace.push(Ev::Write(off, v));
        let r = s.regs.entry(off).or_insert(0);
        if is_status(off) {
            *r &= !v;
        } else {
            *r = v;
        }
    })
}

pub fn irq_trigger(virq: u16, _token: &mut CleanLockToken) {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.events += 1;
        assert!(s.events <= EVENT_BOUND, "event bound exceeded");
        s.trace.push(Ev::Trigger(virq));
        let hooks: Vec<Hook> = s.hooks.iter().copied().filter(|h| h.on_virq == virq).collect();
        for h in hooks {
            let r = s.regs.entry(h.status_offset).or_insert(0);
            if h.set {
                *r |= 1;
            } else {
                *r &= !1;
            }
        }
    })
}
