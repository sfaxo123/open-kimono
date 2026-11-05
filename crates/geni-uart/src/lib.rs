//! Qualcomm GENI Serial Engine debug-UART driver (qcom,geni-debug-uart):
//! init/baud setup + TX + RX (HW-18 TX, HW-19 init/baud/RX).
//!
//! Lane H device-console path. This is a **Rust-native driver**, not a
//! transliteration of the C oracle. Register offsets, bit meanings and the TX
//! command flow are *derived* from the U-Boot oracle (read-only fixtures):
//!
//! - `drivers/serial/serial_msm_geni.c` (U-Boot fork, cited below as `c:<line>`)
//! - `include/soc/qcom/geni-se.h`         (cited below as `h:<line>`)
//!
//! The device console on the OnePlus 6 (sdm845) is the geni debug UART at MMIO
//! base `0x00a84000` — DT node `soc@0/geniqup@ac0000/serial@a84000`,
//! `compatible = "qcom,geni-debug-uart"` (workbench/HW-CONSOLE.md §1–2).
//!
//! HONESTY: this driver is **host-mock tested only** — it has never been run
//! against real hardware. Register semantics come from the U-Boot oracle, not
//! the vendor Linux driver; device bring-up is unproven.

#![no_std]
#![deny(missing_docs)]

extern crate alloc;

#[cfg(test)]
#[macro_use]
extern crate std;

use core::ptr;

// -------------------------------------------------------------------------
// Register-offset window bound — HW-21 (root advisory).
//
// Derivation of SE_WINDOW_SIZE from the oracle register map: every register
// this driver touches is named in `include/soc/qcom/geni-se.h` lines 30–80
// (common SE) or mirrored by the U-Boot fork `drivers/serial/serial_msm_geni.c`
// lines 26–55 (UART-mode SE). The HIGHEST-named register in that map is
// `SE_GENI_RX_WATERMARK_REG = 0x810` (h:64; mirrored c:64, written at c:623 —
// see HW-CONSOLE.md §HW-18/§HW-19 register tables). A u32 access there spans
// [0x810, 0x814), so the oracle register map needs a window of at least
// 0x814 bytes. We round up to 0x1000 (4 KiB): the smallest page that covers
// the whole named map with headroom for FIFO indexing (RX_FIFOn elements
// extend past 0x780). The DT mapping on the carrier is far larger — node
// `serial@a84000` has `reg = <base 0xa84000 size 0x4000>` (HW-CONSOLE.md §2,
// `/proc/device-tree/.../reg` dump) — so this bound is strictly conservative:
// it admits every register in the oracle map and rejects everything beyond.
// -------------------------------------------------------------------------

/// Size of the SE register window this driver may access (bytes). See the
/// derivation block above: >= highest oracle-named register (`SE_GENI_RX_
/// WATERMARK_REG = 0x810`, h:64) + 4, rounded up to a 4 KiB page. The actual
/// DT mapping is 0x4000 (HW-CONSOLE.md §2) — strictly larger than this bound.
pub const SE_WINDOW_SIZE: u32 = 0x1000;

// -------------------------------------------------------------------------
// SE register offsets — exact values from the oracle headers, typed as
// validated `RegOffset` consts (HW-21). The underlying raw byte values are
/// unchanged; construction goes through `RegOffset::new`, which is `const`
// and panics at COMPILE time on any unaligned or out-of-window value — so
// every one of these consts is proven in-window before the crate compiles.
// Common SE registers: geni-se.h lines 30–80; UART-mode SE registers are
// defined in serial_msm_geni.c lines 26–55 (the fork mirrors the Linux
// qcom-geni-se.h names). Each const cites its source line.
// -------------------------------------------------------------------------

/// SE common status: `M_GENI_CMD_ACTIVE` BIT(0), `S_GENI_CMD_ACTIVE` BIT(12) (`h:35`, bits `h:108–109`).
pub const SE_GENI_STATUS: RegOffset = RegOffset::new_unchecked(0x40);
/// Master sequencer command register: opcode in bits 31..27 (`h:49`, `M_OPCODE_MSK` `h:139`).
pub const SE_GENI_M_CMD0: RegOffset = RegOffset::new_unchecked(0x600);
/// Master sequencer command-control: CANCEL BIT(2), ABORT BIT(1), DISABLE BIT(0) (`h:50`, bits `h:144–146`).
pub const SE_GENI_M_CMD_CTRL_REG: RegOffset = RegOffset::new_unchecked(0x604);
/// Master sequencer IRQ status (W1C semantics via IRQ_CLEAR) (`h:51`).
pub const SE_GENI_M_IRQ_STATUS: RegOffset = RegOffset::new_unchecked(0x610);
/// Master sequencer IRQ enable (`h:52`).
pub const SE_GENI_M_IRQ_EN: RegOffset = RegOffset::new_unchecked(0x614);
/// Master sequencer IRQ clear (write bit to clear) (`h:53`).
pub const SE_GENI_M_IRQ_CLEAR: RegOffset = RegOffset::new_unchecked(0x618);
/// Secondary sequencer command register (`h:54`, mirrored `c:29`) — HW-19 RX
/// path: getc re-arms the read with opcode UART_START_READ (1) in bits 31..27
/// at `c:408` (`S_OPCODE_SHIFT` `c:66`).
pub const SE_GENI_S_CMD0: RegOffset = RegOffset::new_unchecked(0x630);
/// Secondary sequencer command-control (`h:55`, mirrored `c:30`).
pub const SE_GENI_S_CMD_CTRL_REG: RegOffset = RegOffset::new_unchecked(0x634);
/// Secondary sequencer IRQ status (`h:56`, mirrored `c:32`) — HW-19 RX path:
/// getc reads it at `c:414` and echoes it back to S_IRQ_CLEAR at `c:416`.
pub const SE_GENI_S_IRQ_STATUS: RegOffset = RegOffset::new_unchecked(0x640);
/// Secondary sequencer IRQ enable (`h:57`, mirrored `c:33`) — HW-19 RX path:
/// oracle enables `S_RX_FIFO_WATERMARK_EN|S_RX_FIFO_LAST_EN` here (`c:350`, `c:379`).
pub const SE_GENI_S_IRQ_EN: RegOffset = RegOffset::new_unchecked(0x644);
/// Secondary sequencer IRQ clear (`h:58`) — used by the oracle's abort_rx at init.
pub const SE_GENI_S_IRQ_CLEAR: RegOffset = RegOffset::new_unchecked(0x648);
#[allow(non_upper_case_globals)] // name mirrors the upstream macro SE_GENI_TX_FIFOn (h:59)
/// TX FIFO write port, first element (`h:59`). The oracle writes each byte here
/// as one word (`c:393`, `c:637`) — no manual 4-byte packing in putc.
pub const SE_GENI_TX_FIFOn: RegOffset = RegOffset::new_unchecked(0x700);
#[allow(non_upper_case_globals)] // name mirrors the upstream macro SE_GENI_RX_FIFOn (h:60)
/// RX FIFO read port (`h:60`) — HW-19: `read_byte` pulls one word from here
/// (oracle getc reads it at `c:423`, low byte is the character, `c:424`).
pub const SE_GENI_RX_FIFOn: RegOffset = RegOffset::new_unchecked(0x780);
/// TX FIFO status: `TX_FIFO_WC` bits 27..0 = word count outstanding (`h:61`, `h:218`).
/// Zero means the TX FIFO is empty (`qcom_geni_serial_tx_empty`, `c:296–299`).
pub const SE_GENI_TX_FIFO_STATUS: RegOffset = RegOffset::new_unchecked(0x800);
/// RX FIFO status: `RX_FIFO_WC_MSK` bits 24..0 = word count in the RX FIFO
/// (`h:62`, mask `h:224`). Non-zero means bytes are readable — the oracle's
/// getc polls it at `c:417–418` and gates the FIFO read on it at `c:420–421`.
pub const SE_GENI_RX_FIFO_STATUS: RegOffset = RegOffset::new_unchecked(0x804);
/// TX watermark: number of fifo words that triggers `M_TX_FIFO_WATERMARK_EN` (`c:63`).
pub const SE_GENI_TX_WATERMARK_REG: RegOffset = RegOffset::new_unchecked(0x80c);
/// RX watermark (`c:64`) — set by the oracle's debug-UART init (`c:623`).
pub const SE_GENI_RX_WATERMARK_REG: RegOffset = RegOffset::new_unchecked(0x810);
/// Force all SE config registers back to hardware defaults: `FORCE_DEFAULT` BIT(0) (`h:32`, `h:89`).
pub const GENI_FORCE_DEFAULT_REG: RegOffset = RegOffset::new_unchecked(0x20);
/// Master SER clock config: `SER_CLK_EN` BIT(0), divider bits 15..4 (`h:36`, `h:112–114`).
pub const GENI_SER_M_CLK_CFG: RegOffset = RegOffset::new_unchecked(0x48);
/// Secondary SER clock config, same fields (`h:37`).
pub const GENI_SER_S_CLK_CFG: RegOffset = RegOffset::new_unchecked(0x4c);
/// FW revision read-only: protocol id bits 15..8 (`h:39`, `h:120–121`) — probe checks UART proto.
pub const GENI_FW_REVISION_RO: RegOffset = RegOffset::new_unchecked(0x68);
/// TX packing cfg0; oracle pins `UART_PACKING_CFG0 = 0xf` = 8 bits/word, 4 words/fifo-word (`c:91`, written `c:502`).
pub const SE_GENI_TX_PACKING_CFG0: RegOffset = RegOffset::new_unchecked(0x260);
/// TX packing cfg1 = 0 (LSB-first) (`c:92`, written `c:503`).
pub const SE_GENI_TX_PACKING_CFG1: RegOffset = RegOffset::new_unchecked(0x264);
/// RX packing cfg0/1 — mirror of TX, set by oracle init for the RX path (`c:45–46`, written `c:504–505`).
pub const SE_GENI_RX_PACKING_CFG0: RegOffset = RegOffset::new_unchecked(0x284);
/// RX packing cfg1 (`c:46`, written `c:505`).
pub const SE_GENI_RX_PACKING_CFG1: RegOffset = RegOffset::new_unchecked(0x288);
/// UART TX transfer byte length (`c:48`); oracle writes the payload size here before START_TX (`c:274`).
pub const SE_UART_TX_TRANS_LEN: RegOffset = RegOffset::new_unchecked(0x270);
/// UART TX stop-bit length, 0 = default 1 bit (`c:49`, written `c:513`).
pub const SE_UART_TX_STOP_BIT_LEN: RegOffset = RegOffset::new_unchecked(0x26c);
/// UART TX word length in bits (8) (`c:50`, written `c:511`).
pub const SE_UART_TX_WORD_LEN: RegOffset = RegOffset::new_unchecked(0x268);
/// UART RX word length in bits (8) (`c:51`, written `c:512`).
pub const SE_UART_RX_WORD_LEN: RegOffset = RegOffset::new_unchecked(0x28c);
/// UART TX transfer config: oracle writes `UART_CTS_MASK` = BIT(1) = ignore flow control (`c:68`, `c:492`, written `c:507`).
pub const SE_UART_TX_TRANS_CFG: RegOffset = RegOffset::new_unchecked(0x25c);
/// UART TX parity config, 0 = disabled (`c:53`, written `c:508`).
pub const SE_UART_TX_PARITY_CFG: RegOffset = RegOffset::new_unchecked(0x2a4);
/// UART RX transfer config, 0 (`c:54`, written `c:509`).
pub const SE_UART_RX_TRANS_CFG: RegOffset = RegOffset::new_unchecked(0x280);
/// UART RX parity config, 0 = disabled (`c:55`, written `c:510`).
pub const SE_UART_RX_PARITY_CFG: RegOffset = RegOffset::new_unchecked(0x2a8);

// -------------------------------------------------------------------------
// Bit meanings (from geni-se.h field comments).
// -------------------------------------------------------------------------

/// `M_GENI_CMD_ACTIVE` — master sequencer is running a command (`h:108`).
pub const M_GENI_CMD_ACTIVE: u32 = 1 << 0;
/// `S_GENI_CMD_ACTIVE` — secondary sequencer is running (`h:109`).
pub const S_GENI_CMD_ACTIVE: u32 = 1 << 12;
/// `M_CMD_DONE_EN` / done-status bit 0 of the master IRQ status/enable/clear (`h:159`).
pub const M_CMD_DONE: u32 = 1 << 0;
/// `M_CMD_ABORT_EN` — abort-completion status bit 5 (`h:164`).
pub const M_CMD_ABORT: u32 = 1 << 5;
/// `M_TX_FIFO_WATERMARK_EN` — TX fifo at/below watermark, master IRQ bit 30 (`h:183`).
pub const M_TX_FIFO_WATERMARK: u32 = 1 << 30;
/// `M_SEC_IRQ_EN` — secondary-sequencer summary, master IRQ bit 31 (`h:184`).
/// The oracle's getc polls this after arming UART_START_READ (`c:410–411`).
pub const M_SEC_IRQ: u32 = 1 << 31;
/// `M_RX_FIFO_WATERMARK_EN` — master IRQ bit 26 (`h:179`); enabled for RX at
/// `c:351`/`c:380`.
pub const M_RX_FIFO_WATERMARK: u32 = 1 << 26;
/// `M_RX_FIFO_LAST_EN` — master IRQ bit 27 (`h:180`); enabled for RX at
/// `c:351`/`c:380`.
pub const M_RX_FIFO_LAST: u32 = 1 << 27;
/// `S_RX_FIFO_WATERMARK_EN` — secondary IRQ bit 26 (`h:209`); enabled for RX
/// at `c:350`/`c:379`.
pub const S_RX_FIFO_WATERMARK: u32 = 1 << 26;
/// `S_RX_FIFO_LAST_EN` — secondary IRQ bit 27 (`h:210`); enabled for RX at
/// `c:350`/`c:379`.
pub const S_RX_FIFO_LAST: u32 = 1 << 27;
/// `S_CMD_DONE_EN` — secondary done status bit 0 (`h:192`).
pub const S_CMD_DONE: u32 = 1 << 0;
/// `S_CMD_ABORT_EN` — secondary abort status bit 5 (`h:196`).
pub const S_CMD_ABORT: u32 = 1 << 5;
/// `S_GENI_CMD_ABORT` — request abort of the secondary sequencer (`h:201`).
pub const S_GENI_CMD_ABORT_BIT: u32 = 1 << 1;
/// `M_GENI_CMD_ABORT` — request abort of the master sequencer (`h:145`).
pub const M_GENI_CMD_ABORT_BIT: u32 = 1 << 1;
/// `FORCE_DEFAULT` — reset SE config to defaults (`h:89`).
pub const FORCE_DEFAULT: u32 = 1 << 0;
/// `SER_CLK_EN` — enable the SER clock (`h:112`).
pub const SER_CLK_EN: u32 = 1 << 0;
/// `CLK_DIV_SHFT` — divider field starts at bit 4 (`c:72`, `h:114`).
pub const CLK_DIV_SHFT: u32 = 4;
/// `TX_FIFO_WC` mask — outstanding TX word count, bits 27..0 (`h:218`).
pub const TX_FIFO_WC: u32 = 0x0fff_ffff;
/// `RX_FIFO_WC_MSK` — RX FIFO word count, bits 24..0 (`h:224`, mirrored `c:69`
/// as the poll mask at `c:417`). Non-zero = at least one fifo word readable.
pub const RX_FIFO_WC: u32 = 0x01ff_ffff;
/// `UART_START_TX` opcode = 1 for the UART protocol; placed in bits 31..27 (`c:67`, `c:65`, used `c:275`).
pub const UART_START_TX_OPCODE: u32 = 0x1;
/// `UART_START_READ` opcode = 1 on the secondary sequencer (`c:61`) — RX path only.
pub const UART_START_READ_OPCODE: u32 = 0x1;
/// Default TX watermark used by the oracle putc (`DEF_TX_WM`, `c:57`).
pub const DEF_TX_WM: u32 = 2;
/// Default RX watermark used by the oracle (`DEF_RX_WM`, `c:58`).
pub const DEF_RX_WM: u32 = 2;
/// Packed UART START_TX command word: opcode 1 << 27 (`qcom_geni_serial_setup_tx`, `c:270–277`).
pub const M_CMD_UART_START_TX: u32 = UART_START_TX_OPCODE << 27;
/// Oversampling factor for UART bit-clock generation (`UART_OVERSAMPLING`, `c:22`).
pub const UART_OVERSAMPLING: u32 = 32;
/// Oracle's SER-clock DFS table — the parent-clock candidates `get_clk_cfg`
/// scans for a multiple of `baud * oversampling` (`root_freq[]`, `c:103–104`;
/// mirrors the GCC `ftbl_gcc_qupv3_wrap0_s0_clk_src` entries, U-Boot
/// `drivers/clk/qcom/clock-sdm845.c:30–39`). The 48 MHz entry (`c:104`) is the
/// one that satisfies 115200; see `ser_clock_for_baud`.
pub const ROOT_FREQ_HZ: [u32; 10] = [
    7_372_800,
    14_745_600,
    19_200_000,
    29_491_200,
    32_000_000,
    48_000_000,
    64_000_000,
    80_000_000,
    96_000_000,
    100_000_000,
];

// -------------------------------------------------------------------------
// Device-tree facts pinned from workbench/HW-CONSOLE.md §1–2 (READ-ONLY).
// -------------------------------------------------------------------------

/// MMIO base of the geni debug UART (DT `serial@a84000` under `geniqup@ac0000`).
pub const DEBUG_UART_BASE: u64 = 0x00a8_4000;
/// DT compatible string of the console node.
pub const DEBUG_UART_COMPATIBLE: &str = "qcom,geni-debug-uart";
/// DT node path of the console under the SoC bus node.
pub const DEBUG_UART_DT_PATH: &str = "soc@0/geniqup@ac0000/serial@a84000";
/// Kernel `stdout-path` / `console=` baud observed on the carrier (HW-CONSOLE.md §1).
pub const CONSOLE_BAUD: u32 = 115200;

/// Register access interface the driver is written against. Implemented by a
/// real volatile MMIO window on target and by a plain register array in tests.
///
/// HW-21 (root advisory): every method takes a **validated** [`RegOffset`],
/// never a raw `u32`. Safe code cannot construct an unaligned or out-of-window
/// offset without going through the checked constructors, so the UB surface of
/// the old safe `read(u32)`/`write(u32)` API is closed at the type level.
pub trait SeRegs {
    /// Read the 32-bit SE register at validated offset `off`.
    fn read(&self, off: RegOffset) -> u32;
    /// Write a 32-bit value to the SE register at validated offset `off`.
    fn write(&mut self, off: RegOffset, val: u32);
    /// Read a FIFO read-port register at validated offset `off`. On hardware
    /// this is an ordinary volatile read; mocks override it so reading
    /// `SE_GENI_RX_FIFOn` pops a word instead of peeking forever (default:
    /// same as `read`). `&mut self` because popping mutates the mock's
    /// modelled FIFO.
    fn read_fifo(&mut self, off: RegOffset) -> u32 {
        self.read(off)
    }
}

/// Errors from constructing a register offset (HW-21 root advisory).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OffsetError {
    /// The byte offset is not 4-byte aligned — a u32 volatile access at this
    /// address would be UB (unsupported alignment on the SE bus).
    Unaligned,
    /// The byte offset lies outside `[0, SE_WINDOW_SIZE)` — past the register
    /// window this driver is allowed to touch (see `SE_WINDOW_SIZE` for the
    /// oracle-derived derivation).
    OutOfWindow,
}

/// A **validated** SE register byte offset: 4-byte aligned and strictly below
/// [`SE_WINDOW_SIZE`]. This is the only way offsets reach pointer arithmetic;
/// the type itself is the proof (invariant enforced by every constructor).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegOffset(u32);

impl RegOffset {
    /// Checked constructor — use this for any offset that comes from safe,
    /// non-const code. Returns an explicit error instead of UB.
    pub const fn new(off: u32) -> Result<Self, OffsetError> {
        if off % 4 != 0 {
            return Err(OffsetError::Unaligned);
        }
        if off >= SE_WINDOW_SIZE {
            return Err(OffsetError::OutOfWindow);
        }
        Ok(Self(off))
    }

    /// Const constructor for the crate's own register map: panics at COMPILE
    /// time (const-eval) on an unaligned or out-of-window value, so every
    /// `pub const …: RegOffset` in this file is proven valid before the crate
    /// can build. Not a soundness hole — it cannot produce an invalid value
    /// at runtime; it refuses to compile instead.
    pub const fn new_unchecked(off: u32) -> Self {
        if off % 4 != 0 || off >= SE_WINDOW_SIZE {
            panic!("static register offset is unaligned or out of window");
        }
        Self(off)
    }

    /// The raw byte offset (in-window and aligned by construction).
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Real hardware: volatile MMIO over a base address. Never exercised by host
/// tests (no real hardware access from tests).
pub struct Mmio {
    base: *mut u8,
}

impl Mmio {
    /// Wrap the SE register window at `base`.
    ///
    /// # Safety
    /// `base` must be a valid, 4-byte-aligned MMIO mapping of one GENI SE that
    /// covers at least [`SE_WINDOW_SIZE`] bytes (the DT node maps 0x4000 — see
    /// HW-CONSOLE.md §2 — so any real `serial@…` mapping satisfies this). The
    /// constructor guarantees the base itself is aligned; every *offset* added
    /// to it later is validated by [`RegOffset`] (aligned, in-window), so the
    /// final pointer always lands inside a mapped, correctly-aligned word.
    ///
    /// TARGET-WIDTH CONTRACT: the `u64` → pointer narrowing is deliberate and
    /// sound on the supported targets — aarch64 addresses in the TTBR0 range
    /// fit in 48 bits and the SE bases here are far below 2^32. It is an
    /// explicit contract of this API, NOT a defect to "fix": on any target
    /// whose pointers are narrower than the physical addresses in use,
    /// callers must map first and pass a pointer-derived base.
    ///
    /// `Mmio` is `Send`/`Sync`: it is a plain address with no interior Rust
    /// state; concurrent volatile word accesses to distinct (and even equal)
    /// offsets of one SE are well-defined device behavior, and the driver
    /// itself takes `&mut self`, so aliasing rules for ordinary memory do not
    /// apply to this mapping.
    pub const unsafe fn new(base: u64) -> Self {
        Self {
            base: base as *mut u8,
        }
    }
}

// SAFETY: see the Send/Sync reasoning on `Mmio::new` — the struct is a bare
// device address; all accesses go through validated `RegOffset`s.
unsafe impl Send for Mmio {}
// SAFETY: as above — no Rust-visible interior mutability, volatile word
// accesses only at aligned in-window offsets.
unsafe impl Sync for Mmio {}

impl SeRegs for Mmio {
    fn read(&self, off: RegOffset) -> u32 {
        // SAFETY: the constructor guarantees a valid, aligned mapping of at
        // least SE_WINDOW_SIZE bytes, and `RegOffset`'s invariant guarantees
        // `off` is 4-byte aligned and < SE_WINDOW_SIZE — so this pointer is
        // in-bounds and correctly aligned for a u32 volatile access. This is
        // the ONLY pointer arithmetic in the crate (HW-21 minimal unsafe).
        unsafe { ptr::read_volatile(self.base.add(off.get() as usize) as *const u32) }
    }
    fn write(&mut self, off: RegOffset, val: u32) {
        // SAFETY: identical justification to `read` above.
        unsafe { ptr::write_volatile(self.base.add(off.get() as usize) as *mut u32, val) }
    }
}

/// Errors the TX flow can report (the oracle silently times out; we surface it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxError {
    /// The master sequencer never went idle (M_GENI_CMD_ACTIVE stuck).
    SequencerBusy,
    /// The TX watermark bit never asserted — FIFO not accepting data.
    FifoNotReady,
    /// The TX command never completed (and abort did not settle either).
    TxNotDone,
}

/// One-shot result type for the TX path.
pub type TxResult = Result<(), TxError>;

/// Errors the RX flow can report (the oracle silently returns 0 on an empty
/// FIFO — `msm_serial_getc` `c:420–421`; we surface it instead).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RxError {
    /// No byte available: the RX FIFO word count (`RX_FIFO_WC`, `h:224`) is
    /// zero. WouldBlock-style — retry later; no FIFO read was performed.
    WouldReadNothing,
    /// The secondary-sequencer summary IRQ (`M_SEC_IRQ_EN`, `h:184`) never
    /// asserted after arming UART_START_READ — the oracle's poll timed out
    /// (`c:410–411`).
    SecondaryTimeout,
}

/// One-shot result type for the RX path.
pub type RxResult = Result<u8, RxError>;

/// Errors the baud-setup path can report (the oracle bails `-EINVAL` when no
/// DFS entry matches — `msm_serial_setbrg` `c:209–212`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BaudError {
    /// No `root_freq` table entry (`c:103–104`) is an exact multiple of
    /// `baud * UART_OVERSAMPLING` — the oracle prints "Can't find matching DFS
    /// entry" (`get_clk_div_rate` `c:141–145`) and returns `-EINVAL` (`c:211`).
    UnsupportedRate,
}

/// Bounded spin budget for polling loops. The oracle derives a baud-scaled
/// microsecond timeout (`qcom_geni_serial_poll_bit`, `c:236–268`); on host mocks
/// a bounded iteration count keeps tests deterministic and on target prevents
/// an infinite hang. 10_000 polls is far more than one FIFO-drain at 115200.
pub const DEFAULT_POLL_BUDGET: u32 = 10_000;

/// SER-clock + divider for a baud rate — pure derivation of the oracle's
/// `get_clk_div_rate` (`c:134–149`) over `get_clk_cfg` (`c:113–120`):
/// `desired = baud * oversampling` (`c:139`), pick the first `root_freq` entry
/// that is an exact multiple of it (`c:115–117`, table `c:103–104`), then
/// `clk_div = ser_clk / desired` (`c:147`). Oversampling 32 = `UART_OVERSAMPLING`
/// (`c:22`; the probe may halve it for QUP HW >= 2.5 via `geni_set_oversampling`
/// `c:448–475`, but SDM845 debug UART keeps 32).
///
/// Worked example — 115200: `desired = 115200 * 32 = 3_686_400`; the scan hits
/// the FIRST table multiple, 7_372_800 (`c:103`), so `ser_clk = 7.3728 MHz` and
/// `clk_div = 7_372_800 / 3_686_400 = 2`. (48 MHz is NOT an exact multiple —
/// remainder 76_800 — so it never qualifies for 115200. The early debug-UART
/// path instead pins a fixed parent: `CLK_DIV = CONFIG_DEBUG_UART_CLOCK /
/// (CONFIG_BAUDRATE * UART_OVERSAMPLING)` with an #error guard (`c:612–616`);
/// the HW-18 init transcript uses div 13 from U-Boot's actual
/// `CONFIG_BAUDRATE=921600 @ 48 MHz` debug-console config.)
///
/// HONESTY: `ser_clk` is the oracle's *assumed* GCC parent rate from its DFS
/// table — we do NOT program the SDM845 clock tree (GCC qup-s0 clk-src / GPLL0
/// parents); that remains unimplemented here.
pub fn ser_clock_for_baud(baud: u32, oversampling: u32) -> Option<(u32, u32)> {
    let desired = baud.checked_mul(oversampling)?;
    if desired == 0 {
        return None;
    }
    for &ser in &ROOT_FREQ_HZ {
        if ser % desired == 0 {
            // c:147 — *clk_div = ser_clk / desired_clk
            return Some((ser, ser / desired));
        }
    }
    None
}

/// The exact SER-clock config word the oracle writes for a divider:
/// `SER_CLK_EN | (clk_div << CLK_DIV_SHFT)` (`geni_serial_baud` `c:192–193`,
/// written to both M and S clk-cfg at `c:195–196`).
pub fn ser_clk_cfg_for_div(clk_div: u32) -> u32 {
    SER_CLK_EN | (clk_div << CLK_DIV_SHFT)
}

/// GENI debug-UART handle bound to a register interface `R`.
///
/// TX flow mirrors what the oracle's putc actually does per byte
/// (`msm_serial_putc` `c:383–399`, `_debug_uart_putc` `c:628–640`):
/// 1. set TX watermark, 2. program TX transfer length + issue UART_START_TX,
/// 3. wait for the TX-FIFO-watermark bit in M_IRQ_STATUS, 4. write the byte as
///    one word to `SE_GENI_TX_FIFOn`, 5. clear the watermark IRQ bit,
/// 6. wait for M_CMD_DONE (abort on timeout) and clear IRQs.
/// The oracle does NOT pack 4 bytes per FIFO word in putc — packing is handled
/// by the hardware via `SE_GENI_TX_PACKING_CFG0 = 0xf` (`c:91`, `c:502`) — so
/// neither do we.
pub struct GeniUart<R: SeRegs> {
    regs: R,
    poll_budget: u32,
}

impl<R: SeRegs> GeniUart<R> {
    /// Bind the driver to a register interface.
    pub fn new(regs: R) -> Self {
        Self {
            regs,
            poll_budget: DEFAULT_POLL_BUDGET,
        }
    }

    /// Override the bounded poll budget (tests use tiny values).
    pub fn with_poll_budget(mut self, budget: u32) -> Self {
        self.poll_budget = budget;
        self
    }

    /// Borrow the register interface (for diagnostics).
    pub fn regs(&self) -> &R {
        &self.regs
    }

    /// Borrow the register interface mutably (mock introspection in tests).
    pub fn regs_mut(&mut self) -> &mut R {
        &mut self.regs
    }

    /// Poll a masked bit field until it equals `want`, up to `poll_budget`
    /// reads. Rust-native replacement for the oracle's timed poll (`c:236–268`).
    fn poll_bit(&self, off: RegOffset, mask: u32, want: bool) -> bool {
        for _ in 0..self.poll_budget {
            if ((self.regs.read(off) & mask) != 0) == want {
                return true;
            }
        }
        false
    }

    /// Wait until the master sequencer is idle. The oracle checks
    /// `SE_GENI_STATUS & M_GENI_CMD_ACTIVE` in `qcom_geni_serial_start_tx`
    /// (`c:326–328`) before touching TX state.
    pub fn wait_sequencer_idle(&self) -> TxResult {
        if self.poll_bit(SE_GENI_STATUS, M_GENI_CMD_ACTIVE, false) {
            Ok(())
        } else {
            Err(TxError::SequencerBusy)
        }
    }

    /// Minimal init as far as the putc TX flow depends on it. Derived from the
    /// oracle's early debug-UART init `_debug_uart_init` (`c:618–626`) plus the
    /// parts of `geni_serial_init` (`c:477–514`) that the TX path reads back:
    ///
    /// - abort any in-flight master command and force config defaults — the
    ///   oracle does an unconditional poll_tx_done + abort_rx first (`c:499–500`,
    ///   `abort_rx` writes S_GENI_CMD_CTRL_REG=`S_GENI_CMD_ABORT` `c:360`,
    ///   clears S IRQs `c:363`, then GENI_FORCE_DEFAULT_REG=FORCE_DEFAULT `c:364`);
    /// - TX/RX packing cfg (`c:502–505`), TX transfer/parity/word-length config
    ///   (`c:507–513`) — 8N1, flow control ignored via CTS mask BIT(1) (`c:492`);
    /// - SER clock enable + divider (`geni_serial_baud` `c:187–197`: writes
    ///   `SER_CLK_EN | clk_div<<4` to GENI_SER_M_CLK_CFG and GENI_SER_S_CLK_CFG);
    /// - RX watermark (`c:623`) so the secondary sequencer is sane;
    /// - TX start prep (`qcom_geni_serial_start_tx` `c:321–338`: sets TX
    ///   watermark `DEF_TX_WM` and enables `M_TX_FIFO_WATERMARK_EN|M_CMD_DONE_EN`
    ///   in M_IRQ_EN when idle).
    ///
    /// What the oracle does at *probe* but we deliberately skip (documented,
    /// quote-only per brief): clock-framework rate setting (`msm_serial_probe`
    /// `clk_enable` `c:530`), FW-revision protocol check via GENI_FW_REVISION_RO
    /// bits 15..8 == UART (`c:533–548`), oversampling vs QUP HW version via the
    /// parent's QUP_HW_VER_REG (`geni_set_oversampling` `c:448–475`), and RX
    /// setup/start (`msm_geni_serial_setup_rx`, `qcom_geni_serial_start_rx`).
    pub fn init(&mut self, clk_div: u32) -> TxResult {
        // Master sequencer settle before reconfig (oracle poll_tx_done at c:499).
        self.wait_sequencer_idle()?;

        // Abort secondary sequencer + force config defaults (abort_rx, c:360–364).
        self.regs
            .write(SE_GENI_S_CMD_CTRL_REG, S_GENI_CMD_ABORT_BIT);
        self.regs
            .write(SE_GENI_S_IRQ_CLEAR, S_CMD_DONE | S_CMD_ABORT);
        self.regs.write(GENI_FORCE_DEFAULT_REG, FORCE_DEFAULT);

        // Packing: 8 bits/word, 4 words per fifo word (UART_PACKING_CFG0/1, c:91–92).
        self.regs.write(SE_GENI_TX_PACKING_CFG0, 0xf);
        self.regs.write(SE_GENI_TX_PACKING_CFG1, 0);
        self.regs.write(SE_GENI_RX_PACKING_CFG0, 0xf);
        self.regs.write(SE_GENI_RX_PACKING_CFG1, 0);

        // 8N1, no flow control (CTS mask BIT(1)), parity off (geni_serial_init c:507–513).
        self.regs.write(SE_UART_TX_TRANS_CFG, 1 << 1);
        self.regs.write(SE_UART_TX_PARITY_CFG, 0);
        self.regs.write(SE_UART_RX_TRANS_CFG, 0);
        self.regs.write(SE_UART_RX_PARITY_CFG, 0);
        self.regs.write(SE_UART_TX_WORD_LEN, 8);
        self.regs.write(SE_UART_RX_WORD_LEN, 8);
        self.regs.write(SE_UART_TX_STOP_BIT_LEN, 0);

        // SER clocks: enable + divider (geni_serial_baud, c:190–196).
        let clk_cfg = SER_CLK_EN | (clk_div << CLK_DIV_SHFT);
        self.regs.write(GENI_SER_M_CLK_CFG, clk_cfg);
        self.regs.write(GENI_SER_S_CLK_CFG, clk_cfg);

        // RX watermark so the secondary side is not starved (_debug_uart_init c:623).
        self.regs.write(SE_GENI_RX_WATERMARK_REG, DEF_RX_WM);

        // TX start prep (qcom_geni_serial_start_tx, c:321–338): only when idle.
        if self.regs.read(SE_GENI_STATUS) & M_GENI_CMD_ACTIVE == 0 {
            let irq_en = self.regs.read(SE_GENI_M_IRQ_EN);
            self.regs.write(SE_GENI_TX_WATERMARK_REG, DEF_TX_WM);
            self.regs
                .write(SE_GENI_M_IRQ_EN, irq_en | M_TX_FIFO_WATERMARK | M_CMD_DONE);
        }
        Ok(())
    }

    /// Emit one byte. Flow mirrors the oracle putc (`c:383–399` / `c:628–640`).
    pub fn write_byte(&mut self, b: u8) -> TxResult {
        // 1. TX watermark = DEF_TX_WM (c:387, c:632).
        self.regs.write(SE_GENI_TX_WATERMARK_REG, DEF_TX_WM);

        // 2. Program transfer length 1 and issue UART_START_TX (setup_tx c:270–277).
        self.regs.write(SE_UART_TX_TRANS_LEN, 1);
        self.regs.write(SE_GENI_M_CMD0, M_CMD_UART_START_TX);

        // 3. Wait until the TX FIFO has room — the watermark bit in
        //    SE_GENI_M_IRQ_STATUS (c:390–391). While this bit is clear the
        //    hardware will not accept a FIFO write, so we must not write data.
        if !self.poll_bit(SE_GENI_M_IRQ_STATUS, M_TX_FIFO_WATERMARK, true) {
            return Err(TxError::FifoNotReady);
        }

        // 4. Push the byte as one FIFO word (c:393, c:637).
        self.regs.write(SE_GENI_TX_FIFOn, b as u32);

        // 5. Clear the watermark IRQ bit we just consumed (c:394, c:638).
        self.regs.write(SE_GENI_M_IRQ_CLEAR, M_TX_FIFO_WATERMARK);

        // 6. Wait for command completion; on timeout abort and wait for the
        //    abort bit, then clear done (+abort) IRQs (poll_tx_done c:279–294).
        if !self.poll_bit(SE_GENI_M_IRQ_STATUS, M_CMD_DONE, true) {
            self.regs
                .write(SE_GENI_M_CMD_CTRL_REG, M_GENI_CMD_ABORT_BIT);
            if !self.poll_bit(SE_GENI_M_IRQ_STATUS, M_CMD_ABORT, true) {
                return Err(TxError::TxNotDone);
            }
            self.regs
                .write(SE_GENI_M_IRQ_CLEAR, M_CMD_DONE | M_CMD_ABORT);
            return Err(TxError::TxNotDone);
        }
        self.regs.write(SE_GENI_M_IRQ_CLEAR, M_CMD_DONE);
        Ok(())
    }

    /// Configure baud from a rate — the oracle's `msm_serial_setbrg` (`c:199–221`)
    /// minus the clock-framework call. Computes `(ser_clk, clk_div)` with
    /// `get_clk_div_rate(baud, oversampling=32)` (`c:134–149`, called `c:208`),
    /// then writes `SER_CLK_EN | div<<4` to GENI_SER_M_CLK_CFG and
    /// GENI_SER_S_CLK_CFG (`geni_serial_baud` `c:187–197`, called `c:218`).
    ///
    /// Deliberate difference from the oracle: `clk_set_rate(priv->se, ser_clk)`
    /// (`c:213`) is NOT performed — programming the SDM845 GCC QUP SE clock
    /// (CGC) is out of scope for HW-19; we rely on the parent already running
    /// at the oracle's table rate (see `ser_clock_for_baud` honesty note).
    pub fn set_baud(&mut self, baud: u32) -> Result<(), BaudError> {
        let (_ser_clk, clk_div) =
            ser_clock_for_baud(baud, UART_OVERSAMPLING).ok_or(BaudError::UnsupportedRate)?;
        let clk_cfg = ser_clk_cfg_for_div(clk_div);
        self.regs.write(GENI_SER_M_CLK_CFG, clk_cfg);
        self.regs.write(GENI_SER_S_CLK_CFG, clk_cfg);
        Ok(())
    }

    /// Arm the RX path — derived from `msm_geni_serial_setup_rx` (`c:367–381`)
    /// / `qcom_geni_serial_start_rx` (`c:340–352`). The oracle's order after
    /// abort_rx (already done in `init`): RX packing (`c:373–374`, also in
    /// `init`), issue UART_START_READ on the secondary sequencer
    /// (`geni_se_setup_s_cmd` `c:310–320`, called `c:376`), RX watermark
    /// `DEF_RX_WM` (`c:378`), and enable the RX fifo bits in S_IRQ_EN
    /// (`c:379`) and M_IRQ_EN (`c:380`).
    pub fn start_rx(&mut self) {
        // geni_se_setup_s_cmd(base, UART_START_READ, 0): opcode 1<<27, params 0 (c:376).
        self.regs
            .write(SE_GENI_S_CMD0, UART_START_READ_OPCODE << 27);
        self.regs.write(SE_GENI_RX_WATERMARK_REG, DEF_RX_WM); // c:378
        let s_en = self.regs.read(SE_GENI_S_IRQ_EN); // setbits_le32 c:379
        self.regs.write(
            SE_GENI_S_IRQ_EN,
            s_en | S_RX_FIFO_WATERMARK | S_RX_FIFO_LAST,
        );
        let m_en = self.regs.read(SE_GENI_M_IRQ_EN); // setbits_le32 c:380
        self.regs.write(
            SE_GENI_M_IRQ_EN,
            m_en | M_RX_FIFO_WATERMARK | M_RX_FIFO_LAST,
        );
    }

    /// Read one byte — derived from the oracle's `msm_serial_getc` (`c:401–425`).
    /// Faithful ordering: re-arm UART_START_READ (opcode 1<<27 into S_CMD0,
    /// `c:408`), poll for the secondary-sequencer summary bit `M_SEC_IRQ_EN`
    /// in M_IRQ_STATUS (`c:410–411`), snapshot both IRQ-status registers and
    /// echo them to their CLEAR registers — that is how the oracle clears RX
    /// IRQs (`c:413–416`) — then gate on `RX_FIFO_WC` in RX_FIFO_STATUS
    /// (`c:417–421`) and pull one word from RX_FIFOn, low byte = char
    /// (`c:423–424`).
    ///
    /// Rust-native differences: the FIFO gate is a single check, not the
    /// oracle's baud-scaled timed poll (`c:417–418`) — an empty FIFO returns
    /// `WouldReadNothing` immediately (no FIFO read performed) instead of
    /// silently returning 0 (`c:420–421`); and every poll is bounded.
    pub fn read_byte(&mut self) -> RxResult {
        // Re-arm the secondary sequencer read (c:408).
        self.regs
            .write(SE_GENI_S_CMD0, UART_START_READ_OPCODE << 27);

        // Wait for the secondary summary IRQ (c:410–411); bounded poll.
        if !self.poll_bit(SE_GENI_M_IRQ_STATUS, M_SEC_IRQ, true) {
            return Err(RxError::SecondaryTimeout);
        }

        // Snapshot + clear both sequencers' IRQs exactly as the oracle does (c:413–416).
        let m_irq_status = self.regs.read(SE_GENI_M_IRQ_STATUS);
        let s_irq_status = self.regs.read(SE_GENI_S_IRQ_STATUS);
        self.regs.write(SE_GENI_M_IRQ_CLEAR, m_irq_status);
        self.regs.write(SE_GENI_S_IRQ_CLEAR, s_irq_status);

        // Is there a fifo word to read? RX_FIFO_WC non-zero (c:417–421).
        if self.regs.read(SE_GENI_RX_FIFO_STATUS) & RX_FIFO_WC == 0 {
            return Err(RxError::WouldReadNothing);
        }

        // Pull one word; low byte is the character (c:423–424).
        let word = self.regs.read_fifo(SE_GENI_RX_FIFOn);
        Ok((word & 0xff) as u8)
    }

    /// Emit a byte slice, one oracle putc cycle per byte.
    pub fn write_bytes(&mut self, data: &[u8]) -> TxResult {
        for &b in data {
            self.write_byte(b)?;
        }
        Ok(())
    }

    /// HW-23 newline policy: on `\n`, emit **`\n` only** (the serial
    /// convention this driver already follows — the U-Boot oracle putc does
    /// no CR/LF translation either, and the carrier's `console=ttyMSM0`
    /// line discipline handles it). We deliberately do NOT expand `\n` to
    /// `\r\n`: raw serial consumers (null-modem captures, later Redox tty)
    /// expect LF-terminated lines on this port.
    pub fn write_str(&mut self, s: &str) -> TxResult {
        self.write_bytes(s.as_bytes())
    }
}

// -------------------------------------------------------------------------
// HW-23: text-output sink abstraction.
// -------------------------------------------------------------------------

/// The error type of the [`TxSink`] seam — a byte-oriented view of the TX
/// path for other crates, so consumers depend on the sink and not on the
/// concrete driver (and not on `TxError`'s register-level variants).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxSinkError {
    /// The underlying driver reported a TX failure for one byte. The raw
    /// [`TxError`] is preserved for diagnostics.
    Tx(TxError),
}

/// Object-safe byte-output trait implemented by [`GeniUart`] (HW-23). Other
/// crates take `&mut dyn TxSink` (or are generic over `S: TxSink`) and never
/// name the driver type.
pub trait TxSink {
    /// Emit one byte.
    fn write_byte(&mut self, b: u8) -> Result<(), TxSinkError>;

    /// HW-23 newline policy: on `\n`, emit **`\n` only** (serial convention;
    /// no CR insertion — see the note on [`GeniUart::write_str`]). Default
    /// impl loops [`TxSink::write_byte`], so object-safe `&mut dyn TxSink`
    /// users get the same policy.
    fn write_str(&mut self, s: &str) -> Result<(), TxSinkError> {
        for &b in s.as_bytes() {
            self.write_byte(b)?;
        }
        Ok(())
    }
}

impl<R: SeRegs> TxSink for GeniUart<R> {
    fn write_byte(&mut self, b: u8) -> Result<(), TxSinkError> {
        GeniUart::write_byte(self, b).map_err(TxSinkError::Tx)
    }
}

// -------------------------------------------------------------------------
// HW-26: byte-input sink abstraction — the RX mirror of `TxSink`.
// -------------------------------------------------------------------------

/// The error type of the [`ByteSource`] seam — a byte-oriented view of the
/// RX path for other layers, so consumers (line discipline, tty) depend on
/// the source and not on the concrete driver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ByteSourceError {
    /// The underlying driver reported an RX failure for this poll. The raw
    /// [`RxError`] is preserved for diagnostics.
    Rx(RxError),
}

/// Object-safe byte-input trait implemented by [`GeniUart`] (HW-26). Other
/// layers take `&mut dyn ByteSource` (or are generic over `S: ByteSource`)
/// and never name the driver type — RX counterpart of [`TxSink`].
pub trait ByteSource {
    /// Pull one byte from the RX path. An empty FIFO surfaces as
    /// [`RxError::WouldReadNothing`] inside [`ByteSourceError::Rx`] —
    /// callers must treat that as "retry later", not a fault (same
    /// contract as [`GeniUart::read_byte`]).
    fn read_byte(&mut self) -> Result<u8, ByteSourceError>;
}

impl<R: SeRegs> ByteSource for GeniUart<R> {
    fn read_byte(&mut self) -> Result<u8, ByteSourceError> {
        GeniUart::read_byte(self).map_err(ByteSourceError::Rx)
    }
}

/// Text-console bridge (HW-23): the HW-16 banner emitted over any [`TxSink`].
pub mod console;

/// Console input line discipline (HW-26, gap contract fixed HW-29): lines
/// over any [`ByteSource`].
pub mod rxline;

/// Device-independent console REPL glue (HW-29): prompt + echo over the
/// [`TxSink`] output seam and the [`rxline::LineReader`] input seam.
pub mod repl;

#[cfg(test)]
mod mock;
#[cfg(test)]
mod tests;
