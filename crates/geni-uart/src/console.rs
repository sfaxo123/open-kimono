//! HW-23 banner bridge: emit the HW-16 console banner over any [`TxSink`].
//!
//! The string is byte-for-byte the `BANNER` const from the fb-write crate's
//! evidence module (`crates/fb-write/src/evidence.rs:16`, HW-11/HW-16 — the
//! same line U-Boot/UEFI stdout printed on the carrier). It is **copied** in
//! here deliberately: this crate must NOT add a cross-crate dependency on
//! fb-write (per HW-23 brief), and `tests.rs::emit_banner_matches_fb_write`
//! pins the copy against the original with an `include_str!` byte scan so it
//! cannot silently drift.

use crate::{TxSink, TxSinkError};

/// The HW-16 banner line — string value copied verbatim from
/// `crates/fb-write/src/evidence.rs` (`pub const BANNER`, line 16; the
/// HW-11 deterministic console evidence rendered by HW-16 over UEFI stdout).
/// 93 bytes, ASCII, no CR/LF inside. Do not edit: any change breaks both the
/// fb-write render digest and `emit_banner_matches_fb_write` here.
pub const BANNER: &str = "REDOX OP6 CONSOLE HW-11 0123456789 abcdefghijklmnopqrstuvwxyz !\"#$%&'()*+,-./:;<=>?@[]^_`{|}~";

/// Emit [`BANNER`] followed by a single `\n` (LF-only newline policy, see
/// [`TxSink::write_str`]) over `sink`. Returns the number of bytes written.
///
/// On error the byte count is not returned — but the sink has already
/// received every byte up to the failing one (writes are not buffered).
pub fn emit_banner<S: TxSink + ?Sized>(sink: &mut S) -> Result<usize, TxSinkError> {
    sink.write_str(BANNER)?;
    sink.write_str("\n")?;
    Ok(BANNER.len() + 1)
}
