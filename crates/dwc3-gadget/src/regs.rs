//! Register offsets and fields (spec §3, `specs/dwc3-gadget.md`).
//!
//! Core offsets are relative to the `snps,dwc3` window (SDM845 `0x0a600000`),
//! QSCRATCH offsets to the `qcom,dwc3` glue window (`0x0a6f8800`).

pub const GCTL: u64 = 0xc110;
pub const GCTL_PRTCAPDIR_MASK: u32 = 0b11 << 12;
pub const GCTL_PRTCAPDIR_DEVICE: u32 = 2 << 12;

pub const GUCTL1: u64 = 0xc11c;
pub const GUCTL1_DEV_FORCE_20_CLK_FOR_30_CLK: u32 = 1 << 26;

pub const GSNPSID: u64 = 0xc120;
pub const CORE_ID_USB3: u32 = 0x5533;
pub const CORE_ID_USB31: u32 = 0x3331;

pub const GHWPARAMS3: u64 = 0xc14c;

pub const GUSB2PHYCFG0: u64 = 0xc200;
pub const GUSB2PHYCFG_SUSPENDUSB20: u32 = 1 << 6;
pub const GUSB2PHYCFG_ENBLSLPM: u32 = 1 << 8;
pub const GUSB2PHYCFG_USBTRDTIM_MASK: u32 = 0xf << 10;
pub const GUSB2PHYCFG_USBTRDTIM_8BIT: u32 = 9 << 10;

pub const GEVNTADR_LO0: u64 = 0xc400;
pub const GEVNTADR_HI0: u64 = 0xc404;
pub const GEVNTSIZ0: u64 = 0xc408;
pub const GEVNTSIZ_INTMASK: u32 = 1 << 31;
pub const GEVNTCOUNT0: u64 = 0xc40c;
pub const GEVNTCOUNT_MASK: u32 = 0xffff;

pub const DCFG: u64 = 0xc700;
pub const DCFG_DEVSPD_MASK: u32 = 0b111;
pub const DCFG_DEVSPD_HIGH: u32 = 0;
pub const DCFG_DEVADDR_SHIFT: u32 = 3;
pub const DCFG_DEVADDR_MASK: u32 = 0x7f << DCFG_DEVADDR_SHIFT;

pub const DCTL: u64 = 0xc704;
pub const DCTL_RUN_STOP: u32 = 1 << 31;
pub const DCTL_CSFTRST: u32 = 1 << 30;

pub const DEVTEN: u64 = 0xc708;
pub const DEVTEN_DISCONN: u32 = 1 << 0;
pub const DEVTEN_USBRST: u32 = 1 << 1;
pub const DEVTEN_CONNECTDONE: u32 = 1 << 2;
pub const DEVTEN_WKUP: u32 = 1 << 4;
pub const DEVTEN_U3L2L1SUSP: u32 = 1 << 6;

pub const DSTS: u64 = 0xc70c;
pub const DSTS_DEVCTRLHLT: u32 = 1 << 22;
pub const DSTS_CONNECTSPD_MASK: u32 = 0b111;
pub const CONNECTSPD_HIGH: u32 = 0;
pub const CONNECTSPD_FULL: u32 = 1;

pub const DALEPENA: u64 = 0xc720;

/// Per physical endpoint `n`: `DEPCMDPAR2` at `+0x0`, `PAR1` `+0x4`, `PAR0` `+0x8`, `DEPCMD` `+0xc`.
pub const fn depcmd_base(phys: u8) -> u64 {
    0xc800 + 0x10 * phys as u64
}
pub const DEPCMDPAR2: u64 = 0x0;
pub const DEPCMDPAR1: u64 = 0x4;
pub const DEPCMDPAR0: u64 = 0x8;
pub const DEPCMD: u64 = 0xc;

pub const DEPCMD_CMDIOC: u32 = 1 << 8;
pub const DEPCMD_CMDACT: u32 = 1 << 10;
pub const DEPCMD_HIPRI_FORCERM: u32 = 1 << 11;
pub const DEPCMD_PARAM_SHIFT: u32 = 16;

/// `DEPCMD.CMDSTATUS` (bits 15:12).
pub const fn depcmd_status(v: u32) -> u32 {
    (v >> 12) & 0xf
}

/// Endpoint command types (`DEPCMD.CMDTYP`).
pub mod cmd {
    pub const SET_EP_CONFIG: u32 = 1;
    pub const SET_XFER_RESOURCE: u32 = 2;
    pub const SET_STALL: u32 = 4;
    pub const CLEAR_STALL: u32 = 5;
    pub const START_TRANSFER: u32 = 6;
    pub const END_TRANSFER: u32 = 8;
    pub const START_NEW_CONFIG: u32 = 9;
}

/// `DEPCFG` parameter 0: action, FIFO number, max packet size, endpoint type.
pub const fn depcfg_par0(ep_type: u32, mps: u16, fifo: u8, modify: bool) -> u32 {
    (if modify { 2 << 30 } else { 0 })
        | ((fifo as u32 & 0x1f) << 17)
        | ((mps as u32 & 0x7ff) << 3)
        | ((ep_type & 0b11) << 1)
}

/// `DEPCFG` parameter 1: physical endpoint number, interval, XferNotReady + XferComplete enabled.
pub const fn depcfg_par1(phys: u8, interval_m1: u8) -> u32 {
    ((phys as u32 & 0x1f) << 25) | ((interval_m1 as u32) << 16) | (1 << 10) | (1 << 8)
}

/// QSCRATCH glue registers.
pub mod qscratch {
    pub const GENERAL_CFG: u64 = 0x08;
    pub const PIPE_UTMI_CLK_SEL: u32 = 1 << 0;
    pub const PIPE3_PHYSTATUS_SW: u32 = 1 << 3;
    pub const PIPE_UTMI_CLK_DIS: u32 = 1 << 8;
    pub const HS_PHY_CTRL: u64 = 0x10;
    pub const UTMI_OTG_VBUS_VALID: u32 = 1 << 20;
    pub const SW_SESSVLD_SEL: u32 = 1 << 28;
    pub const SS_PHY_CTRL: u64 = 0x30;
    pub const LANE0_PWR_PRESENT: u32 = 1 << 24;
}
