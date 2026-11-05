//! Host stand-ins for `current_mpidr()` and the kernel logger. Not CPU proof:
//! the MPIDR value is whatever the test sets for this thread.
use std::cell::{Cell, RefCell};

thread_local! {
    static MPIDR: Cell<u64> = Cell::new(0);
    static MPIDR_READS: Cell<usize> = Cell::new(0);
    static LOGS: RefCell<Vec<String>> = RefCell::new(Vec::new());
}

pub fn current_mpidr() -> u64 {
    MPIDR_READS.with(|r| r.set(r.get() + 1));
    MPIDR.with(|m| m.get())
}

pub fn log(msg: String) {
    LOGS.with(|l| l.borrow_mut().push(msg));
}

/// Set this thread's MPIDR and clear captured logs / read counter.
pub fn reset(mpidr: u64) {
    MPIDR.with(|m| m.set(mpidr));
    MPIDR_READS.with(|r| r.set(0));
    LOGS.with(|l| l.borrow_mut().clear());
}

/// Change MPIDR but keep captured logs (for multi-call ordering checks).
pub fn reset_mpidr_only(mpidr: u64) {
    MPIDR.with(|m| m.set(mpidr));
}

pub fn take_logs() -> Vec<String> {
    LOGS.with(|l| std::mem::take(&mut *l.borrow_mut()))
}

pub fn mpidr_reads() -> usize {
    MPIDR_READS.with(|r| r.get())
}
