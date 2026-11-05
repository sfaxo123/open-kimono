//! Trace hooks that replace ONLY `ptr::read_volatile` / `ptr::write_volatile`.
//! They still perform the volatile access on the host buffer, then record
//! (kind, byte offset from the installed base, width in bits, value).

use std::cell::{Cell, RefCell};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    R { off: usize, bits: usize, val: u64 },
    W { off: usize, bits: usize, val: u64 },
}

thread_local! {
    static BASE: Cell<usize> = const { Cell::new(0) };
    static TRACE: RefCell<Vec<Access>> = const { RefCell::new(Vec::new()) };
}

pub fn reset(base: usize) {
    BASE.with(|b| b.set(base));
    TRACE.with(|t| t.borrow_mut().clear());
}

pub fn take() -> Vec<Access> {
    TRACE.with(|t| std::mem::take(&mut *t.borrow_mut()))
}

fn bits_of<T>(v: &T) -> u64 {
    let n = size_of::<T>();
    assert!(n <= 8, "trace hook: unsupported width {n}");
    let mut b = [0u8; 8];
    unsafe { core::ptr::copy_nonoverlapping(v as *const T as *const u8, b.as_mut_ptr(), n) };
    u64::from_le_bytes(b)
}

fn offset_of(p: usize) -> usize {
    p.wrapping_sub(BASE.with(|b| b.get()))
}

pub unsafe fn read_volatile<T>(src: *const T) -> T {
    let v = unsafe { core::ptr::read_volatile(src) };
    let a = Access::R { off: offset_of(src as usize), bits: 8 * size_of::<T>(), val: bits_of(&v) };
    TRACE.with(|t| t.borrow_mut().push(a));
    v
}

pub unsafe fn write_volatile<T>(dst: *mut T, src: T) {
    let a = Access::W { off: offset_of(dst as usize), bits: 8 * size_of::<T>(), val: bits_of(&src) };
    TRACE.with(|t| t.borrow_mut().push(a));
    unsafe { core::ptr::write_volatile(dst, src) };
}
