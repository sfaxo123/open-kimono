//! Host harness: real model.rs + extracted gicv3.rs items (see gen.py).
//! Kernel `error!`/`info!`/... print `module_path:LEVEL -- msg`; here every
//! log macro records only the formatted message so tests can compare it.
#![allow(unused_macros)]

macro_rules! log_capture {
    ($($arg:tt)*) => { crate::shim::log(format!($($arg)*)) };
}
macro_rules! error { ($($arg:tt)*) => { log_capture!($($arg)*) }; }
macro_rules! warn { ($($arg:tt)*) => { log_capture!($($arg)*) }; }
macro_rules! info { ($($arg:tt)*) => { log_capture!($($arg)*) }; }
macro_rules! debug { ($($arg:tt)*) => { log_capture!($($arg)*) }; }
macro_rules! println { ($($arg:tt)*) => { log_capture!($($arg)*) }; }

#[allow(dead_code)]
#[path = "__MODEL_SOURCE__"]
pub mod model;

pub mod shim;

pub mod generated {
    pub mod baseline;
    pub mod current;
}

#[cfg(test)]
mod parity_tests;
