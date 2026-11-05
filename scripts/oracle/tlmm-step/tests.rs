use crate::shim::*;
use crate::{new, old, qcom_irq};

pub(crate) const N: usize = 150;

#[derive(Clone, Default)]
pub(crate) struct Sc {
    pub probe_only: bool,
    pub virq_base: usize,
    pub enabled: Vec<usize>,
    pub pending: Vec<usize>,
    // (event pin, target pin, set?) applied to fake MMIO during that event
    pub hooks: Vec<(usize, usize, bool)>,
}

pub(crate) fn status(pin: usize) -> usize {
    qcom_irq::tlmm_pin_offset(pin as u32, 0xc).unwrap()
}

pub(crate) fn all() -> Vec<usize> {
    (0..N).collect()
}

pub(crate) fn setup(sc: &Sc) -> [bool; N] {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        *s = State::default();
        for &p in &sc.pending {
            s.regs.insert(status(p), 1);
        }
        for &(on, target, set) in &sc.hooks {
            s.hooks.push(Hook {
                on_virq: (sc.virq_base + on) as u16,
                status_offset: status(target),
                set,
            });
        }
    });
    let mut en = [false; N];
    for &p in &sc.enabled {
        en[p] = true;
    }
    en
}

pub(crate) fn take_trace() -> Vec<Ev> {
    STATE.with(|s| std::mem::take(&mut s.borrow_mut().trace))
}

fn run_old(sc: &Sc) -> Vec<Ev> {
    let en = setup(sc);
    let mut t = old::make(FAKE_BASE, sc.virq_base, en, sc.probe_only);
    InterruptHandler::irq_handler(&mut t, 0, &mut CleanLockToken);
    take_trace()
}

fn run_new(sc: &Sc) -> Vec<Ev> {
    let en = setup(sc);
    let mut t = new::make(FAKE_BASE, sc.virq_base, en, sc.probe_only);
    InterruptHandler::irq_handler(&mut t, 0, &mut CleanLockToken);
    take_trace()
}

fn triggers(trace: &[Ev]) -> Vec<u16> {
    trace
        .iter()
        .filter_map(|e| match e {
            Ev::Trigger(v) => Some(*v),
            _ => None,
        })
        .collect()
}

fn scenarios() -> Vec<(&'static str, Sc, Vec<usize>)> {
    // (name, scenario, expected triggered pins from original semantics)
    vec![
        ("probe_only", Sc { probe_only: true, enabled: all(), pending: all(), ..Sc::default() }, vec![]),
        ("empty", Sc::default(), vec![]),
        ("all_disabled_all_pending", Sc { pending: all(), ..Sc::default() }, vec![]),
        ("all_enabled_nonpending", Sc { enabled: all(), ..Sc::default() }, vec![]),
        ("two_pending_and_last149", Sc { enabled: all(), pending: vec![3, 7, 149], ..Sc::default() }, vec![3, 7, 149]),
        ("adjacent_pending", Sc { enabled: all(), pending: vec![50, 51, 52], ..Sc::default() }, vec![50, 51, 52]),
        (
            "tile_spanning",
            Sc { enabled: all(), pending: vec![3, 4, 11, 12, 37, 38, 39, 40, 126, 127], ..Sc::default() },
            vec![3, 4, 11, 12, 37, 38, 39, 40, 126, 127],
        ),
        (
            "pending_but_disabled_odds",
            Sc { enabled: (0..N).step_by(2).collect(), pending: all(), ..Sc::default() },
            (0..N).step_by(2).collect(),
        ),
        ("first_pin0_only", Sc { enabled: all(), pending: vec![0], ..Sc::default() }, vec![0]),
        (
            "event_sets_later_pin",
            Sc { enabled: all(), pending: vec![5], hooks: vec![(5, 9, true)], ..Sc::default() },
            vec![5, 9],
        ),
        (
            "event_clears_later_pin",
            Sc { enabled: all(), pending: vec![5, 9], hooks: vec![(5, 9, false)], ..Sc::default() },
            vec![5],
        ),
        (
            "event_sets_earlier_pin_not_rescanned",
            Sc { enabled: all(), pending: vec![20], hooks: vec![(20, 10, true)], ..Sc::default() },
            vec![20],
        ),
        (
            "event_sets_last149",
            Sc { enabled: all(), pending: vec![0], hooks: vec![(0, 149, true)], ..Sc::default() },
            vec![0, 149],
        ),
        (
            "virq_base_cast_truncation",
            Sc { virq_base: 65530, enabled: all(), pending: vec![0, 10, 149], ..Sc::default() },
            vec![0, 10, 149],
        ),
    ]
}

fn expected_virqs(sc: &Sc, pins: &[usize]) -> Vec<u16> {
    pins.iter().map(|p| (sc.virq_base + p) as u16).collect()
}

/// Characterizes the ACTUAL ff56c966 handler; writes its traces for the record.
#[test]
fn characterize_original() {
    let mut out = String::new();
    for (name, sc, pins) in scenarios() {
        let t = run_old(&sc);
        assert_eq!(triggers(&t), expected_virqs(&sc, &pins), "original {name}");
        let reads = t.iter().filter(|e| matches!(e, Ev::Read(_))).count();
        let writes = t.iter().filter(|e| matches!(e, Ev::Write(..))).count();
        out += &format!("{name}: reads={reads} writes={writes} triggers={:?}\n", triggers(&t));
        if sc.probe_only {
            assert!(t.is_empty(), "probe_only touches nothing");
        }
    }
    if let Ok(p) = std::env::var("TLMM_TRACE_OUT") {
        std::fs::write(p, out).unwrap();
    }
}

/// Full read/write/event trace of new handler equals the original's.
#[test]
fn new_trace_equals_original() {
    for (name, sc, _) in scenarios() {
        assert_eq!(run_new(&sc), run_old(&sc), "trace mismatch in {name}");
    }
}

/// Every event is immediately preceded by the exactly-1 clear of its own pin.
#[test]
fn ordering_clear_precedes_each_event() {
    for (name, sc, _) in scenarios() {
        let t = run_new(&sc);
        for (i, e) in t.iter().enumerate() {
            if let Ev::Trigger(v) = e {
                let pin = (*v as usize + 0x10000 - sc.virq_base % 0x10000) % 0x10000;
                assert!(i > 0 && t[i - 1] == Ev::Write(status(pin), 1), "{name}: event {v} not preceded by clear");
            }
        }
        let writes = t.iter().filter(|e| matches!(e, Ev::Write(..))).count();
        assert_eq!(writes, triggers(&t).len(), "{name}: one clear per event");
    }
}

/// Resume covers every later pending pin, including ones raised during an event.
#[test]
fn coverage_resumes_after_each_pin() {
    for (name, sc, pins) in scenarios() {
        assert_eq!(triggers(&run_new(&sc)), expected_virqs(&sc, &pins), "coverage {name}");
    }
}

