//! HW-24: the geni-uart text path COMPILED INTO the EFI image, gated OFF
//! at runtime by default.
//!
//! This module is pure glue between the `geni-uart` sink abstraction
//! ([`geni_uart::TxSink`]) and the EFI app: it never touches MMIO itself.
//! The caller decides whether a sink exists at all; this function only
//! forwards bytes when one is provided.
//!
//! ## Runtime guard — why a compile-time const, not a UEFI variable
//!
//! `GENI_UART_ENABLE` is a `const bool = false` on purpose:
//! - QEMU `-M virt` has **no geni device** — constructing an `Mmio` sink
//!   there would hand the driver a window over unmapped guest memory, so
//!   the sink must not be constructed at all on virt.
//! - A UEFI-variable guard would still require the code path to *decide*
//!   to build the sink at runtime; a const makes the disabled path
//!   provably inert (the sink branch is dead code eliminated when false)
//!   and keeps the acceptance deterministic: every QEMU boot prints
//!   `HW24-UART-PATH=DISABLED`, byte-identical, forever.
//! - Enabling the geni path on real hardware (sdm845, MMIO base
//!   `geni_uart::DEBUG_UART_BASE`) is a SEPARATE gated decision — it
//!   requires the SDM845 clock tree and operator sign-off (HW-23 honesty
//!   note), so flipping a runtime variable is the wrong affordance.

/// Compile-time runtime guard. FALSE on QEMU virt (no geni device);
/// enabling on real hardware is a separate gated decision — see the
/// module docs.
pub const GENI_UART_ENABLE: bool = false;

/// Forward the HW-24 marker text through `sink_opt` if a sink is present.
///
/// Pure glue, unit-testable: `None -> None` (path disabled — no sink was
/// ever constructed); `Some(sink) -> Some(count)` where `count` is the
/// number of bytes handed to the sink. Errors from the sink propagate.
pub fn emit_banner_if_enabled(
    sink_opt: Option<&mut dyn geni_uart::TxSink>,
) -> Result<Option<usize>, geni_uart::TxSinkError> {
    match sink_opt {
        None => Ok(None),
        Some(sink) => {
            const TEXT: &str = "HW24-UART-PATH=ENABLED\n";
            let n = TEXT.len();
            sink.write_str(TEXT)?;
            Ok(Some(n))
        }
    }
}
