//! Pure protocol core for Qualcomm remote processors. Hardware access is supplied by callers.
#![no_std]
#![forbid(unsafe_code)]
extern crate alloc;

pub mod glink;
pub mod pas;
pub mod qmi;
pub mod qrtr;
pub mod smem;

/// Protocol or model failure. Hardware transports may use their own error types.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    InvalidId,
    InvalidAddress,
    MissingItem,
    Bounds,
    Malformed,
    Duplicate,
    NoIntent,
    Full,
    Smc(i64),
}
