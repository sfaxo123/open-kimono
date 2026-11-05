// Host tests over the actual src/dtb/irq_dispatch.rs and src/sync/irq_lock.rs.
// Mock core/controllers exercise the generic UNWIRED prototype, not real adapters.
use crate::irq_dispatch::*;
use crate::sync::irq_lock::{MaskPolicy, Saved};
use alloc::sync::Arc;
use std::cell::Cell;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};

thread_local!(static IRQ_ON: Cell<bool> = const { Cell::new(true) });
fn irq_on() -> bool { IRQ_ON.with(|c| c.get()) }
fn set_irq(on: bool) { IRQ_ON.with(|c| c.set(on)) }

struct FakeMask;
unsafe impl MaskPolicy for FakeMask {
    fn save_and_disable() -> Saved {
        let prior_enabled = irq_on();
        set_irq(false);
        Saved { prior_enabled }
    }
    unsafe fn restore(saved: Saved) { set_irq(saved.prior_enabled) }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ev { Ack, Handle(u32), Scheme(u16), Diag(u32), Eoi(usize, u32), Clear(usize), Enable(usize), Disable(usize) }

struct Tok;
type Trace = Arc<Mutex<Vec<Ev>>>;
type Reg = IrqRegistry<Mock, FakeMask>;
fn push(t: &Trace, e: Ev) { t.lock().unwrap().push(e) }

enum Ctrl {
    Bcm { claim: u32, virq: Option<usize> },
    Tlmm { enabled: Vec<bool>, status: Vec<Cell<bool>>, base: usize, probe_only: bool },
    Plic { claim: u32, virq: Option<usize> },
}

struct Mock {
    slots: Vec<Option<Arc<dyn IrqHandler<Tok>>>>,
    child: Vec<Option<usize>>,
    used: Vec<bool>,
    route: Vec<Option<u32>>,
    ctrls: Vec<Ctrl>,
    trace: Trace,
}

fn target(virq: usize) -> Completion { Completion { ic_idx: 9, hwirq: virq as u32 + 1000 } }

impl DispatchCore<Tok> for Mock {
    type Handler = dyn IrqHandler<Tok>;
    fn route(&mut self, virq: u32) -> Route<Self::Handler> {
        assert!(!irq_on(), "core accessed with IRQs enabled");
        let v = virq as usize;
        match (&self.slots[v], self.child[v]) {
            (Some(h), _) => Route::Handler { handler: h.clone(), completion: target(v) },
            (None, Some(ic_idx)) => Route::Child { ic_idx },
            (None, None) => Route::Scheme,
        }
    }
    fn handler(&mut self, virq: usize) -> Option<Arc<Self::Handler>> { self.slots[virq].clone() }
    fn slot(&mut self, virq: usize) -> &mut Option<Arc<Self::Handler>> { &mut self.slots[virq] }
    fn begin(&mut self, ic_idx: usize, _virq: u32) -> ControllerStep {
        assert!(!irq_on());
        let t = self.trace.clone();
        match &self.ctrls[ic_idx] {
            Ctrl::Bcm { claim, virq } => { push(&t, Ev::Ack); bcm2835_begin(*claim, *virq) }
            Ctrl::Plic { claim, virq } => { push(&t, Ev::Ack); plic_begin(*claim, *virq) }
            Ctrl::Tlmm { probe_only: true, .. } => ControllerStep::Done,
            Ctrl::Tlmm { enabled, status, base, .. } => tlmm_scan(enabled, 0, *base,
                |p| status[p].get(), |p| { status[p].set(false); push(&t, Ev::Clear(p)) }),
        }
    }
    fn resume(&mut self, ic_idx: usize, cursor: u32) -> ControllerStep {
        assert!(!irq_on());
        let t = self.trace.clone();
        match &self.ctrls[ic_idx] {
            Ctrl::Bcm { .. } | Ctrl::Plic { .. } => complete_claim(cursor, |c| push(&t, Ev::Eoi(ic_idx, c))),
            Ctrl::Tlmm { enabled, status, base, .. } => tlmm_scan(enabled, cursor as usize, *base,
                |p| status[p].get(), |p| { status[p].set(false); push(&t, Ev::Clear(p)) }),
        }
    }
    fn complete(&mut self, c: Completion) { assert!(!irq_on()); push(&self.trace, Ev::Eoi(c.ic_idx, c.hwirq)) }
    fn is_reserved(&mut self, v: usize) -> bool { self.used[v] || self.slots[v].is_some() }
    fn set_reserved(&mut self, v: usize, reserved: bool) {
        self.used[v] = reserved;
        if reserved { push(&self.trace, Ev::Enable(v)) } else {
            push(&self.trace, Ev::Disable(v));
            self.route[v] = None;
        }
    }
}

fn mock(ctrls: Vec<Ctrl>) -> (&'static Reg, Trace) {
    let trace: Trace = Arc::default();
    let core = Mock { slots: (0..VIRQ_COUNT).map(|_| None).collect(), child: vec![None; VIRQ_COUNT],
        used: vec![false; VIRQ_COUNT], route: vec![Some(7); VIRQ_COUNT], ctrls, trace: trace.clone() };
    let reg: &'static Reg = Box::leak(Box::new(Reg::new()));
    assert!(unsafe { reg.publish(core) }.is_ok());
    (reg, trace)
}
fn with_core<R>(reg: &Reg, f: impl FnOnce(&mut Mock) -> R) -> R { f(reg.lock_for_test().lock().as_mut().unwrap()) }

struct Sk { reg: &'static Reg, trace: Trace }
impl Sink<Tok> for Sk {
    fn scheme(&self, virq: u16, _: &mut Tok) {
        assert!(self.reg.lock_for_test().try_lock().is_some(), "scheme event under core guard");
        push(&self.trace, Ev::Scheme(virq))
    }
    fn diagnostic(&self, d: Diag) {
        assert!(self.reg.lock_for_test().try_lock().is_some(), "diagnostic under core guard");
        let Diag::UnexpectedClaim(c) = d;
        push(&self.trace, Ev::Diag(c))
    }
}

struct H { reg: &'static Reg, trace: Trace, eoi: bool, reenter: AtomicU32, free: Arc<Mutex<Vec<bool>>>, irq: Arc<Mutex<Vec<bool>>> }
impl IrqHandler<Tok> for H {
    fn handle(&self, virq: u32, tok: &mut Tok) {
        self.free.lock().unwrap().push(self.reg.lock_for_test().try_lock().is_some());
        self.irq.lock().unwrap().push(irq_on());
        push(&self.trace, Ev::Handle(virq));
        if self.reenter.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1)).is_ok() {
            let sink = Sk { reg: self.reg, trace: self.trace.clone() };
            unsafe { self.reg.dispatch(virq, tok, &sink) }.unwrap();
        }
    }
    fn eoi_after(&self) -> bool { self.eoi }
}
fn handler(reg: &'static Reg, trace: &Trace, eoi: bool, reenter: u32) -> Arc<H> {
    Arc::new(H { reg, trace: trace.clone(), eoi, reenter: AtomicU32::new(reenter), free: Arc::default(), irq: Arc::default() })
}
fn run(reg: &'static Reg, trace: &Trace, virq: u32) -> Vec<Ev> {
    let sink = Sk { reg, trace: trace.clone() };
    unsafe { reg.dispatch(virq, &mut Tok, &sink) }.unwrap();
    core::mem::take(&mut *trace.lock().unwrap())
}

#[test]
fn callback_runs_outside_core_guard() {
    let (reg, t) = mock(vec![]);
    let h = handler(reg, &t, false, 0);
    unsafe { reg.register(386, h.clone()) }.map_err(|e| e.0).unwrap();
    run(reg, &t, 386);
    assert_eq!(*h.free.lock().unwrap(), vec![true], "callback invoked under core guard");
}

#[test]
fn opt_in_eoi_completes_copied_target_once() {
    let (reg, t) = mock(vec![]);
    unsafe { reg.register(386, handler(reg, &t, true, 0)) }.map_err(|e| e.0).unwrap();
    assert_eq!(run(reg, &t, 386), vec![Ev::Handle(386), Ev::Eoi(9, 1386)]);
}

#[test]
fn default_false_handler_does_not_eoi_and_fallbacks_hold() {
    let (reg, t) = mock(vec![]);
    unsafe { reg.register(40, handler(reg, &t, false, 0)) }.map_err(|e| e.0).unwrap();
    assert_eq!(run(reg, &t, 40), vec![Ev::Handle(40)]);
    assert_eq!(run(reg, &t, 41), vec![Ev::Scheme(41)]);
    assert_eq!(run(reg, &t, 2048), vec![]);
}

#[test]
fn same_virq_reentry_completes_each_invocation() {
    let (reg, t) = mock(vec![]);
    let h = handler(reg, &t, true, 1);
    unsafe { reg.register(77, h.clone()) }.map_err(|e| e.0).unwrap();
    let e = run(reg, &t, 77);
    assert_eq!(e, vec![Ev::Handle(77), Ev::Handle(77), Ev::Eoi(9, 1077), Ev::Eoi(9, 1077)]);
    assert_eq!(*h.free.lock().unwrap(), vec![true, true]);
}

#[test]
fn bcm_registered_only_then_self_eoi() {
    let (reg, t) = mock(vec![Ctrl::Bcm { claim: 5, virq: Some(300) }, Ctrl::Bcm { claim: 6, virq: Some(301) },
        Ctrl::Bcm { claim: 99, virq: None }]);
    with_core(reg, |c| { c.child[30] = Some(0); c.child[31] = Some(1); c.child[32] = Some(2) });
    unsafe { reg.register(300, handler(reg, &t, true, 0)) }.map_err(|e| e.0).unwrap();
    assert_eq!(run(reg, &t, 30), vec![Ev::Ack, Ev::Handle(300), Ev::Eoi(0, 5)]);
    assert_eq!(run(reg, &t, 31), vec![Ev::Ack, Ev::Eoi(1, 6)]);
    assert_eq!(run(reg, &t, 32), vec![Ev::Ack, Ev::Diag(99), Ev::Eoi(2, 99)]);
}

#[test]
fn tlmm_ascending_clear_before_each_event() {
    let mut enabled = vec![false; 12];
    for p in [3, 5, 7] { enabled[p] = true; }
    let status: Vec<Cell<bool>> = (0..12).map(|p| Cell::new([3, 7, 9].contains(&p))).collect();
    let (reg, t) = mock(vec![Ctrl::Tlmm { enabled: enabled.clone(), status, base: 500, probe_only: false },
        Ctrl::Tlmm { enabled, status: (0..12).map(|_| Cell::new(true)).collect(), base: 600, probe_only: true }]);
    with_core(reg, |c| { c.child[20] = Some(0); c.child[21] = Some(1) });
    assert_eq!(run(reg, &t, 20), vec![Ev::Clear(3), Ev::Scheme(503), Ev::Clear(7), Ev::Scheme(507)]);
    with_core(reg, |c| match &c.ctrls[0] { Ctrl::Tlmm { status, .. } => assert!(status[9].get()), _ => unreachable!() });
    assert_eq!(run(reg, &t, 21), vec![]);
}

#[test]
fn plic_translated_nested_without_extra_eoi() {
    let (reg, t) = mock(vec![Ctrl::Plic { claim: 5, virq: Some(40) }, Ctrl::Plic { claim: 6, virq: None }]);
    with_core(reg, |c| { c.child[10] = Some(0); c.child[11] = Some(1) });
    unsafe { reg.register(40, handler(reg, &t, false, 0)) }.map_err(|e| e.0).unwrap();
    assert_eq!(run(reg, &t, 10), vec![Ev::Ack, Ev::Handle(40)]);
    assert_eq!(run(reg, &t, 11), vec![Ev::Ack, Ev::Diag(6), Ev::Eoi(1, 6)]);
}

#[test]
fn not_ready_and_publish_once() {
    let reg: &'static Reg = Box::leak(Box::new(Reg::new()));
    set_irq(true);
    let sink = Sk { reg, trace: Arc::default() };
    assert_eq!(unsafe { reg.dispatch(3, &mut Tok, &sink) }, Err(RegistryError::NotReady));
    assert_eq!(unsafe { reg.is_reserved(3) }, Err(RegistryError::NotReady));
    assert!(irq_on(), "NotReady path left IRQs masked");
    let (r2, _) = mock(vec![]);
    let again = Mock { slots: vec![], child: vec![], used: vec![], route: vec![], ctrls: vec![], trace: Arc::default() };
    assert!(unsafe { r2.publish(again) }.is_err());
}

#[test]
fn duplicate_registration_is_atomic() {
    let (reg, t) = mock(vec![]);
    let oks: Vec<bool> = (0..8).map(|_| {
        let h = handler(reg, &t, false, 0);
        std::thread::spawn(move || unsafe { reg.register(55, h) }.is_ok())
    }).collect::<Vec<_>>().into_iter().map(|j| j.join().unwrap()).collect();
    assert_eq!(oks.iter().filter(|o| **o).count(), 1);
    let err = unsafe { reg.register(55, handler(reg, &t, false, 0)) }.map_err(|e| e.0);
    assert_eq!(err, Err(RegistryError::Exists));
    assert_eq!(unsafe { reg.register(4096, handler(reg, &t, false, 0)) }.map_err(|e| e.0), Err(RegistryError::OutOfRange));
}

#[test]
fn reservation_keeps_immediate_policy() {
    let (reg, t) = mock(vec![]);
    assert_eq!(unsafe { reg.is_reserved(12) }, Ok(false));
    unsafe { reg.set_reserved(12, true) }.unwrap();
    assert_eq!(unsafe { reg.is_reserved(12) }, Ok(true));
    unsafe { reg.set_reserved(12, false) }.unwrap();
    assert_eq!(unsafe { reg.is_reserved(12) }, Ok(false));
    assert_eq!(with_core(reg, |c| c.route[12]), None);
    assert_eq!(*t.lock().unwrap(), vec![Ev::Enable(12), Ev::Disable(12)]);
    unsafe { reg.register(13, handler(reg, &t, false, 0)) }.map_err(|e| e.0).unwrap();
    assert_eq!(unsafe { reg.is_reserved(13) }, Ok(true));
}

#[test]
fn irq_mask_restored_including_masked_entry() {
    let (reg, t) = mock(vec![]);
    let h = handler(reg, &t, true, 0);
    unsafe { reg.register(90, h.clone()) }.map_err(|e| e.0).unwrap();
    set_irq(true);
    run(reg, &t, 90);
    assert!(irq_on());
    set_irq(false);
    run(reg, &t, 90);
    assert!(!irq_on(), "masked entry was unmasked");
    set_irq(true);
    assert_eq!(*h.irq.lock().unwrap(), vec![true, false], "callback saw guard mask instead of entry state");
}

#[test]
fn duplicate_reservation_is_atomic() {
    let (reg, t) = mock(vec![]);
    let barrier = Arc::new(std::sync::Barrier::new(8));
    let joins: Vec<_> = (0..8).map(|_| {
        let barrier = barrier.clone();
        std::thread::spawn(move || {
            barrier.wait();
            // SAFETY: thread-local fake IRQ state; no scheduling inside the closure.
            unsafe { reg.try_reserve::<Tok>(44) }
        })
    }).collect();
    let results: Vec<_> = joins.into_iter().map(|j| j.join().unwrap()).collect();
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(results.iter().filter(|r| **r == Err(RegistryError::Exists)).count(), 7);
    assert_eq!(*t.lock().unwrap(), vec![Ev::Enable(44)]);
    assert_eq!(unsafe { reg.try_reserve::<Tok>(4096) }, Err(RegistryError::OutOfRange));
}
