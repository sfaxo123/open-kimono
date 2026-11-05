//! P5 stage 2 — `critical-section` implementation for the
//! Redox target (`cfg(target_os = "redox")`, feature
//! `dwc3-bringup`).
//!
//! The embassy stack (`embassy-usb`, the dwc3-gadget embassy
//! driver) requires a critical-section implementation; the
//! Redox base provides none for this standalone daemon.
//!
//! What this is: a single global spin guard. What makes it
//! sound HERE, and only here:
//!
//! - The RAM-boot kernel configuration this daemon ships in
//!   runs one CPU (the port's SMP work is a later phase), so
//!   no cross-core exclusion is needed.
//! - The daemon is fully polled: no interrupt handler in this
//!   stage takes a critical section or touches the USB state,
//!   so the guard can never be re-entered from IRQ context
//!   while held.
//! - Critical sections in the stack are short and never span
//!   an await that could switch to code taking another one on
//!   the same thread in a conflicting order; a re-entrant
//!   acquire on the same CPU would deadlock — if a future
//!   stage adds IRQ-driven USB or SMP, this impl must be
//!   replaced (interrupt-masking on one core, a real mutex
//!   story on several), not extended.
//!
//! The restore state is unused (nothing to mask/restore), so
//! `Default::default()` is returned, which stays correct if a
//! dependency unifies a `restore-state-*` feature on.

use core::sync::atomic::{AtomicBool, Ordering};

static TAKEN: AtomicBool = AtomicBool::new(false);

/// The registered implementation (unit type; the state lives
/// in the static above).
pub struct SpinImpl;

critical_section::set_impl!(SpinImpl);

unsafe impl critical_section::Impl for SpinImpl {
    unsafe fn acquire() -> critical_section::RawRestoreState {
        while TAKEN.swap(true, Ordering::Acquire) {
            core::hint::spin_loop();
        }
        Default::default()
    }

    unsafe fn release(_state: critical_section::RawRestoreState) {
        TAKEN.store(false, Ordering::Release);
    }
}
