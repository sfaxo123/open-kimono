//! `ufs-core` — read-only UFS host controller core for the OnePlus 6 (`qcom,sdm845-ufshc`).
//!
//! Spec: `specs/ufs.md`. The driver takes the register window through the `mmio-replay`
//! [`Mmio`] trait and its descriptor/data memory through [`Dma`], so every step is host-testable
//! against a register + UTRD/UPIU model or a replayed phone trace.
//!
//! Bring-up ([`Ufshc::bring_up`]): HCE cycle with the Qualcomm QUniPro/PHY checks, UIC
//! `DME_LINKSTARTUP`, UTRL setup, NOP OUT/IN, `fDeviceInit`, device/geometry/unit descriptors →
//! logical-unit table. Then [`Ufshc::read_capacity`], [`Ufshc::read`] and [`Ufshc::read_gpt`].
//!
//! **Read-only by construction:** no UTRD with host-to-device direction is ever built, the SCSI
//! layer only accepts an opcode allowlist ([`scsi::Cmd`], [`Ufshc::scsi_raw`]) and the only Query
//! write is the volatile `fDeviceInit` flag.

#![no_std]
#![forbid(unsafe_code)]
#![deny(missing_docs)]

extern crate alloc;

mod device;
pub mod dma;
pub mod gpt;
mod hc;
pub mod regs;
pub mod scsi;
pub mod upiu;
mod xfer;

pub use device::{DeviceInfo, LogicalUnit};
pub use dma::Dma;
pub use hc::{HostInfo, Ufshc};
pub use mmio_replay::Mmio;

/// Where a bounded wait ran out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// HCE did not drop to 0.
    HceDisable,
    /// HCE did not come back to 1.
    HceEnable,
    /// The PHY TX FSM never reached HIBERN8 after HCE.
    PhyHibern8,
    /// HCS.UCRDY never set.
    UicReady,
    /// IS.UCCS never set.
    UicComplete,
    /// The transfer doorbell never cleared.
    Doorbell,
    /// The slot did not clear after UTRLCLR.
    Clear,
    /// `fDeviceInit` never read back 0.
    DeviceInit,
}

/// Driver errors. Every error path leaves the controller without an outstanding slot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// The controller lacks a capability this driver needs.
    Unsupported(&'static str),
    /// The DMA region is too small or misaligned.
    DmaLayout,
    /// A bounded wait expired.
    Timeout(Stage),
    /// `MPHY_TX_FSM_STATE` after HCE (the firmware-initialised PHY did not survive).
    PhyNotReady(u32),
    /// A UIC command returned a non-zero config result code.
    Uic {
        /// UIC opcode.
        opcode: u8,
        /// `UCMDARG2[7:0]`.
        result: u8,
    },
    /// Link startup completed but HCS.DP is clear.
    NoDevice,
    /// HCS.UTRLRDY clear or a stale doorbell after list setup.
    ListNotReady,
    /// A fatal IS bit was raised during a transfer (raw IS value).
    Fatal(u32),
    /// Overall Command Status in the UTRD was not success.
    Ocs(u8),
    /// The response UPIU had the wrong transaction code or task tag.
    BadUpiu,
    /// The UPIU response field was not "target success".
    Response(u8),
    /// The SCSI status was CHECK CONDITION.
    CheckCondition(scsi::Sense),
    /// Another non-GOOD SCSI status.
    ScsiStatus(u8),
    /// A command outside the read-only allowlist; nothing was sent.
    Refused(u8),
    /// A descriptor was shorter than its fixed fields or had the wrong IDN.
    Descriptor(u8),
    /// Invalid caller argument (LUN, length, buffer); nothing was sent.
    InvalidArgument,
    /// The device returned fewer data-in bytes than requested (bytes received).
    ShortTransfer(usize),
    /// GPT parse failure.
    Gpt(gpt::GptError),
}

/// Result alias.
pub type Result<T> = core::result::Result<T, Error>;

/// Busy-wait hook between polls (production: a short sleep; tests: no-op).
pub trait Delay {
    /// Wait about `us` microseconds.
    fn delay_us(&mut self, us: u32);
}

/// A [`Delay`] that returns immediately.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoDelay;

impl Delay for NoDelay {
    fn delay_us(&mut self, _us: u32) {}
}

/// Qualcomm vendor settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Qcom {
    /// `core_clk_unipro` in MHz for `DME_VS_CORE_CLK_CTRL` (DT top OPP: 150).
    pub unipro_core_clk_mhz: u8,
}

/// Poll budgets (iterations, each followed by `poll_delay_us`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Polls {
    /// HCE enable/disable.
    pub hce: u32,
    /// PHY HIBERN8 check (DME_GET per poll).
    pub phy: u32,
    /// UIC ready / completion.
    pub uic: u32,
    /// Transfer doorbell.
    pub transfer: u32,
    /// Slot clear after a timeout.
    pub clear: u32,
    /// `fDeviceInit` read-back queries.
    pub device_init: u32,
}

/// Driver configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    /// `Some` on `qcom,*-ufshc`.
    pub qcom: Option<Qcom>,
    /// Poll budgets.
    pub polls: Polls,
    /// Delay between polls, µs.
    pub poll_delay_us: u32,
    /// Delay after writing HCE = 1 before the first read-back, µs (Linux/FreeBSD: 100).
    pub hce_settle_us: u32,
    /// NOP OUT attempts (Linux `NOP_OUT_RETRIES` = 10).
    pub nop_attempts: u8,
}

impl Config {
    /// SDM845 defaults: Qualcomm hooks on, timeouts in line with FreeBSD/Linux.
    pub const fn sdm845() -> Self {
        Config {
            qcom: Some(Qcom {
                unipro_core_clk_mhz: 150,
            }),
            polls: Polls {
                hce: 1_000,
                phy: 1_000,
                uic: 5_000,
                transfer: 100_000,
                clear: 1_000,
                device_init: 2_000,
            },
            poll_delay_us: 100,
            hce_settle_us: 100,
            nop_attempts: 10,
        }
    }
}
