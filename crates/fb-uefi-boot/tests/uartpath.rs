//! HW-24 host tests for the gated geni-uart path glue in
//! `crates/fb-uefi-boot/src/uartpath.rs`.
//!
//! These live in `tests/` (an external test crate) on purpose: the binary
//! itself is `#![no_std]` + `#![no_main]` with uefi's panic/entry lang items,
//! so `cargo test` cannot compile the *bin* test harness on the host. The
//! integration test instead depends on the crate's library target — which
//! `src/lib.rs` provides by re-exporting the `uartpath` module — and runs
//! the pure glue against a tiny local mock sink under std.

use fb_uefi_boot::uartpath::{self, GENI_UART_ENABLE};

/// Tiny local mock sink (10 lines) — deliberately NOT geni-uart's
/// test-only mock module: this crate must not depend on internals of a
/// read-only crate.
struct MockSink {
    bytes: Vec<u8>,
}

impl geni_uart::TxSink for MockSink {
    fn write_byte(&mut self, b: u8) -> Result<(), geni_uart::TxSinkError> {
        self.bytes.push(b);
        Ok(())
    }
}

#[test]
fn uartpath_disabled_returns_none() {
    // No sink constructed (the QEMU-virt default): pure None, no writes.
    assert_eq!(uartpath::emit_banner_if_enabled(None).unwrap(), None);
}

#[test]
fn uartpath_enabled_counts_bytes() {
    let mut mock = MockSink { bytes: Vec::new() };
    let n = uartpath::emit_banner_if_enabled(Some(&mut mock))
        .unwrap()
        .expect("sink present must yield Some(count)");
    assert_eq!(n, "HW24-UART-PATH=ENABLED\n".len());
    assert_eq!(mock.bytes, b"HW24-UART-PATH=ENABLED\n");
}

#[test]
fn geni_uart_guard_is_off_by_default() {
    // The pinned runtime guard: false on QEMU virt (no geni device).
    assert!(!GENI_UART_ENABLE);
}
