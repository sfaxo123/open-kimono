//! UFSHCI register offsets/bits and the Qualcomm vendor additions (`specs/ufs.md` §3).

/// Host Controller Capabilities.
pub const CAP: u64 = 0x00;
/// UFS version.
pub const VER: u64 = 0x08;
/// Interrupt Status (write-1-to-clear).
pub const IS: u64 = 0x20;
/// Interrupt Enable.
pub const IE: u64 = 0x24;
/// Host Controller Status.
pub const HCS: u64 = 0x30;
/// Host Controller Enable.
pub const HCE: u64 = 0x34;
/// UTP Transfer Request List base, low 32 bits.
pub const UTRLBA: u64 = 0x50;
/// UTP Transfer Request List base, high 32 bits.
pub const UTRLBAU: u64 = 0x54;
/// UTRL doorbell.
pub const UTRLDBR: u64 = 0x58;
/// UTRL clear (a 0 bit clears that slot).
pub const UTRLCLR: u64 = 0x5c;
/// UTRL run/stop.
pub const UTRLRSR: u64 = 0x60;
/// UIC command opcode.
pub const UICCMD: u64 = 0x90;
/// UIC command argument 1 (`attr << 16 | selector`).
pub const UCMDARG1: u64 = 0x94;
/// UIC command argument 2 (`[7:0]` config result code).
pub const UCMDARG2: u64 = 0x98;
/// UIC command argument 3 (value).
pub const UCMDARG3: u64 = 0x9c;

/// CAP: number of transfer slots minus one.
pub const CAP_NUTRS_MASK: u32 = 0xff;
/// CAP: 64-bit addressing supported.
pub const CAP_64AS: u32 = 1 << 24;
/// CAP: legacy single doorbell *not* supported.
pub const CAP_LSDBS: u32 = 1 << 29;

/// IS: UTP transfer request completion.
pub const IS_UTRCS: u32 = 1 << 0;
/// IS: UIC error.
pub const IS_UE: u32 = 1 << 2;
/// IS: UIC command completion.
pub const IS_UCCS: u32 = 1 << 10;
/// IS: device fatal error.
pub const IS_DFES: u32 = 1 << 11;
/// IS: UTP error.
pub const IS_UTPES: u32 = 1 << 12;
/// IS: host controller fatal error.
pub const IS_HCFES: u32 = 1 << 16;
/// IS: system bus fatal error.
pub const IS_SBFES: u32 = 1 << 17;
/// IS bits that abort a transfer.
pub const IS_FATAL: u32 = IS_UE | IS_DFES | IS_UTPES | IS_HCFES | IS_SBFES;

/// HCS: device present.
pub const HCS_DP: u32 = 1 << 0;
/// HCS: transfer request list ready.
pub const HCS_UTRLRDY: u32 = 1 << 1;
/// HCS: UIC command ready.
pub const HCS_UCRDY: u32 = 1 << 3;

/// HCE: host controller enable.
pub const HCE_ENABLE: u32 = 1 << 0;

/// UIC opcode DME_GET.
pub const DME_GET: u8 = 0x01;
/// UIC opcode DME_SET.
pub const DME_SET: u8 = 0x02;
/// UIC opcode DME_PEER_GET.
pub const DME_PEER_GET: u8 = 0x03;
/// UIC opcode DME_LINKSTARTUP.
pub const DME_LINKSTARTUP: u8 = 0x16;

/// UniPro `PA_ConnectedTxDataLanes`.
pub const PA_CONNECTED_TX_DATA_LANES: u16 = 0x1561;
/// M-PHY TX `TX_LCC_ENABLE` (selector = lane).
pub const TX_LCC_ENABLE: u16 = 0x002c;

/// Qualcomm vendor registers and DME attributes (Linux `ufs-qcom.h`, facts only).
pub mod qcom {
    /// `REG_UFS_CFG0`.
    pub const CFG0: u64 = 0xd8;
    /// `REG_UFS_CFG1`.
    pub const CFG1: u64 = 0xdc;
    /// `REG_UFS_HW_VERSION`.
    pub const HW_VERSION: u64 = 0xe4;
    /// CFG0: `QUNIPRO_G4_SEL` (cleared on HW major >= 5 only).
    pub const CFG0_QUNIPRO_G4_SEL: u32 = 1 << 5;
    /// CFG1: `QUNIPRO_SEL`.
    pub const CFG1_QUNIPRO_SEL: u32 = 1 << 0;
    /// DME `MPHY_TX_FSM_STATE` (selector = lane).
    pub const MPHY_TX_FSM_STATE: u16 = 0x0041;
    /// `MPHY_TX_FSM_STATE` value once the PHY reached HIBERN8 after HCE.
    pub const TX_FSM_HIBERN8: u32 = 1;
    /// DME `DME_VS_CORE_CLK_CTRL`.
    pub const DME_VS_CORE_CLK_CTRL: u16 = 0xd002;
    /// `DME_VS_CORE_CLK_CTRL` cycles-per-µs field (HW major < 4).
    pub const CLK_1US_CYCLES_MASK: u32 = 0xff;
    /// `DME_VS_CORE_CLK_CTRL` `CORE_CLK_DIV_EN`.
    pub const CORE_CLK_DIV_EN: u32 = 1 << 8;

    /// Major field of `HW_VERSION`.
    pub fn hw_major(ver: u32) -> u32 {
        ver >> 28
    }
}
