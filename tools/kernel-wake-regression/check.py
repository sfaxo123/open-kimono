#!/usr/bin/env python3
"""Compile the actual scheduler timer-scan excerpt with host lock/context doubles.

This checks queue conservation when try_read fails. It does not execute a kernel,
IRQs, the wake-application loop, or prove which kernel an image contains.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess

RUST = r'''
use std::cell::{RefCell, Ref, RefMut};
use std::collections::BTreeSet;
use std::rc::{Rc, Weak};
use std::cmp::Ordering;

#[derive(Clone)]
struct WeakContextRef(Weak<ContextLock>);
impl WeakContextRef {
    fn upgrade(&self) -> Option<Rc<ContextLock>> { self.0.upgrade() }
    fn address(&self) -> usize { self.0.as_ptr() as usize }
}
impl PartialEq for WeakContextRef { fn eq(&self, r: &Self) -> bool { self.address() == r.address() } }
impl Eq for WeakContextRef {}
impl PartialOrd for WeakContextRef { fn partial_cmp(&self, r: &Self) -> Option<Ordering> { Some(self.cmp(r)) } }
impl Ord for WeakContextRef { fn cmp(&self, r: &Self) -> Ordering { self.address().cmp(&r.address()) } }
struct Status;
impl Status { fn is_soft_blocked(&self) -> bool { true } }
struct Context { status: Status, wake: Option<u128> }
struct ContextLock(RefCell<Context>);
impl ContextLock {
    fn try_read(&self, _: ()) -> Option<Ref<'_, Context>> { self.0.try_borrow().ok() }
}
struct Token;
impl Token { fn token(&self) {} }
struct QueueLock(RefCell<RunQueue>);
impl QueueLock { fn lock(&self) -> RefMut<'_, RunQueue> { self.0.borrow_mut() } }
struct RunQueue { timers: BTreeSet<(u128, WeakContextRef)> }
struct Internals {
    tmp_wakeups: RefCell<Vec<(Option<u128>, WeakContextRef)>>,
    tmp_timers: RefCell<Vec<(u128, WeakContextRef)>>,
    run_queue: QueueLock,
}
struct PerCpu { switch_internals: Internals }
fn cpu() -> PerCpu {
    PerCpu { switch_internals: Internals {
        tmp_wakeups: RefCell::new(Vec::new()), tmp_timers: RefCell::new(Vec::new()),
        run_queue: QueueLock(RefCell::new(RunQueue { timers: BTreeSet::new() })),
    }}
}
fn sleeper(wake: u128) -> Rc<ContextLock> {
    Rc::new(ContextLock(RefCell::new(Context { status: Status, wake: Some(wake) })))
}
fn add(cpu: &PerCpu, wake: u128, ctx: &Rc<ContextLock>) {
    cpu.switch_internals.run_queue.lock().timers.insert((wake, WeakContextRef(Rc::downgrade(ctx))));
}
fn scan(percpu: &PerCpu, schedule_time: u128, token: &Token) {
    include!("scan.inc");
}
#[test]
fn contended_due_timer_survives_and_wakes_once_after_unlock() {
    let cpu = cpu(); let ctx = sleeper(5); add(&cpu, 5, &ctx);
    let held = ctx.0.borrow_mut();
    scan(&cpu, 10, &Token);
    assert_eq!(cpu.switch_internals.tmp_wakeups.borrow().len(), 0);
    assert_eq!(cpu.switch_internals.run_queue.lock().timers.len(), 1,
               "due timer lost while context was write-locked");
    drop(held);
    scan(&cpu, 11, &Token);
    assert_eq!(cpu.switch_internals.run_queue.lock().timers.len(), 0);
    assert_eq!(cpu.switch_internals.tmp_wakeups.borrow().len(), 1);
    scan(&cpu, 12, &Token);
    assert_eq!(cpu.switch_internals.tmp_wakeups.borrow().len(), 1, "duplicate wake");
}
#[test]
fn normal_due_timer_wakes_and_future_timer_stays_queued() {
    let cpu = cpu(); let due = sleeper(5); let future = sleeper(50);
    add(&cpu, 5, &due); add(&cpu, 50, &future);
    scan(&cpu, 10, &Token);
    assert_eq!(cpu.switch_internals.tmp_wakeups.borrow().len(), 1);
    assert_eq!(cpu.switch_internals.run_queue.lock().timers.len(), 1);
    scan(&cpu, 11, &Token);
    assert_eq!(cpu.switch_internals.tmp_wakeups.borrow().len(), 1);
    scan(&cpu, 50, &Token);
    assert_eq!(cpu.switch_internals.tmp_wakeups.borrow().len(), 2);
}
#[test]
fn dead_context_and_obsolete_wake_are_not_requeued() {
    let cpu = cpu(); let dead = sleeper(5); let obsolete = sleeper(20);
    add(&cpu, 5, &dead); add(&cpu, 5, &obsolete); drop(dead);
    scan(&cpu, 10, &Token);
    assert_eq!(cpu.switch_internals.tmp_wakeups.borrow().len(), 0);
    assert_eq!(cpu.switch_internals.run_queue.lock().timers.len(), 0);
}
'''

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('source', type=Path)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    source = args.source.read_text()
    start_anchor = '        // Alarm (previously in update_runnable)'
    end_anchor = '        // Drain from percpu'
    assert source.count(start_anchor) == source.count(end_anchor) == 1, 'source anchors changed'
    start, end = source.index(start_anchor), source.index(end_anchor)
    assert start < end
    excerpt = source[start:end]
    args.output.mkdir(parents=True, exist_ok=False)
    (args.output / 'scan.inc').write_text('{\n' + excerpt + '\n}')
    (args.output / 'harness.rs').write_text(RUST)
    binary = (args.output / 'tests').resolve()
    with (args.output / 'compile.log').open('w') as log:
        compiled = subprocess.run(['rustc', '--edition=2024', '--test', str(args.output / 'harness.rs'),
                                   '-o', str(binary)], stdout=log, stderr=subprocess.STDOUT, timeout=30)
    receipt = {'source_sha256': hashlib.sha256(source.encode()).hexdigest(),
               'excerpt_sha256': hashlib.sha256(excerpt.encode()).hexdigest(),
               'compile_rc': compiled.returncode, 'test_rc': None,
               'scope': 'actual scan excerpt; host doubles; no kernel/IRQ/image proof'}
    if compiled.returncode == 0:
        with (args.output / 'tests.log').open('w') as log:
            tested = subprocess.run([str(binary), '--test-threads=1'], stdout=log,
                                    stderr=subprocess.STDOUT, timeout=10)
        receipt['test_rc'] = tested.returncode
    (args.output / 'receipt.json').write_text(json.dumps(receipt, indent=2) + '\n')
    print(json.dumps(receipt))
    return compiled.returncode or receipt['test_rc'] or 0

if __name__ == '__main__':
    raise SystemExit(main())
