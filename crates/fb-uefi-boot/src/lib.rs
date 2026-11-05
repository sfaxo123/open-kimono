//! HW-24: library surface of `fb-uefi-boot`, so host integration tests
//! (`tests/uartpath.rs`) can exercise the pure geni-uart path glue without
//! compiling the `#![no_main]` uefi binary test harness (whose panic/entry
//! lang items collide with std on the host). The binary (`src/main.rs`)
//! declares the same `uartpath` module for the EFI image.

#![no_std]

pub mod uartpath;
