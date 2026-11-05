//! Host controller: HCE cycle, Qualcomm hooks, UIC commands, UTRL setup, one-slot transfers.

use crate::dma::{self, Dma};
use crate::regs::{self, qcom};
use crate::{Config, Delay, Error, Result, Stage};
use alloc::vec::Vec;
use mmio_replay::Mmio;

/// Identification read during bring-up.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HostInfo {
    /// CAP register.
    pub cap: u32,
    /// VER register.
    pub ver: u32,
    /// Qualcomm `REG_UFS_HW_VERSION` (0 when not Qualcomm).
    pub qcom_hw_version: u32,
}

/// A completed transfer: the response UPIU (header + data segment).
pub(crate) struct Completion {
    pub resp: Vec<u8>,
}

/// UFS host controller driver.
pub struct Ufshc<M: Mmio, D: Dma, T: Delay> {
    io: M,
    base: u64,
    pub(crate) dma: D,
    pub(crate) delay: T,
    pub(crate) cfg: Config,
    pub(crate) info: HostInfo,
    pub(crate) tag: u8,
}

impl<M: Mmio, D: Dma, T: Delay> Ufshc<M, D, T> {
    /// Wrap a register window at `base` and a DMA region. No MMIO access happens here.
    pub fn new(io: M, base: u64, dma: D, delay: T, cfg: Config) -> Result<Self> {
        if dma.size() < dma::MIN_LEN || dma.phys() & 0xfff != 0 {
            return Err(Error::DmaLayout);
        }
        Ok(Ufshc {
            io,
            base,
            dma,
            delay,
            cfg,
            info: HostInfo::default(),
            tag: 0,
        })
    }

    /// Identification registers read by [`Ufshc::enable`].
    pub fn info(&self) -> HostInfo {
        self.info
    }

    /// Give back the register window and DMA region.
    pub fn into_parts(self) -> (M, D) {
        (self.io, self.dma)
    }

    pub(crate) fn rd(&mut self, off: u64) -> u32 {
        self.io.read_u32(self.base + off)
    }

    pub(crate) fn wr(&mut self, off: u64, v: u32) {
        self.io.write_u32(self.base + off, v)
    }

    pub(crate) fn wait(
        &mut self,
        stage: Stage,
        off: u64,
        polls: u32,
        ok: impl Fn(u32) -> bool,
    ) -> Result<u32> {
        for _ in 0..polls {
            let v = self.rd(off);
            if ok(v) {
                return Ok(v);
            }
            self.delay.delay_us(self.cfg.poll_delay_us);
        }
        Err(Error::Timeout(stage))
    }

    /// Steps 1–5 of `specs/ufs.md` §4: identify, IE=0, HCE 1→0→1, Qualcomm QUniPro + PHY check.
    pub fn enable(&mut self) -> Result<()> {
        let cap = self.rd(regs::CAP);
        let ver = self.rd(regs::VER);
        let hw = if self.cfg.qcom.is_some() {
            self.rd(qcom::HW_VERSION)
        } else {
            0
        };
        self.info = HostInfo {
            cap,
            ver,
            qcom_hw_version: hw,
        };
        if cap & regs::CAP_LSDBS != 0 {
            return Err(Error::Unsupported("single doorbell"));
        }
        let end = self.dma.phys() + self.dma.size() as u64 - 1;
        if end > u32::MAX as u64 && cap & regs::CAP_64AS == 0 {
            return Err(Error::Unsupported("64-bit addressing"));
        }
        self.wr(regs::IE, 0);
        let hce = self.rd(regs::HCE);
        if hce & regs::HCE_ENABLE != 0 {
            self.wr(regs::HCE, hce & !regs::HCE_ENABLE);
            let polls = self.cfg.polls.hce;
            self.wait(Stage::HceDisable, regs::HCE, polls, |v| {
                v & regs::HCE_ENABLE == 0
            })?;
        }
        if self.cfg.qcom.is_some() {
            let cfg1 = self.rd(qcom::CFG1);
            self.wr(qcom::CFG1, cfg1 | qcom::CFG1_QUNIPRO_SEL);
            if qcom::hw_major(hw) >= 5 {
                let cfg0 = self.rd(qcom::CFG0);
                self.wr(qcom::CFG0, cfg0 & !qcom::CFG0_QUNIPRO_G4_SEL);
            }
        }
        self.wr(regs::HCE, regs::HCE_ENABLE);
        self.delay.delay_us(self.cfg.hce_settle_us);
        let polls = self.cfg.polls.hce;
        self.wait(Stage::HceEnable, regs::HCE, polls, |v| {
            v & regs::HCE_ENABLE != 0
        })?;
        if self.cfg.qcom.is_some() {
            self.check_phy_hibern8()?;
        }
        Ok(())
    }

    fn check_phy_hibern8(&mut self) -> Result<()> {
        let mut state = 0;
        for _ in 0..self.cfg.polls.phy {
            state = self.dme_get(qcom::MPHY_TX_FSM_STATE, 0)?;
            if state == qcom::TX_FSM_HIBERN8 {
                return Ok(());
            }
            self.delay.delay_us(self.cfg.poll_delay_us);
        }
        Err(Error::PhyNotReady(state))
    }

    /// Issue one UIC command; returns `UCMDARG3` (the DME_GET value).
    pub fn uic(&mut self, opcode: u8, arg1: u32, arg2: u32, arg3: u32) -> Result<u32> {
        let polls = self.cfg.polls.uic;
        self.wait(Stage::UicReady, regs::HCS, polls, |v| {
            v & regs::HCS_UCRDY != 0
        })?;
        self.wr(regs::UCMDARG1, arg1);
        self.wr(regs::UCMDARG2, arg2);
        self.wr(regs::UCMDARG3, arg3);
        self.wr(regs::UICCMD, opcode as u32);
        self.wait(Stage::UicComplete, regs::IS, polls, |v| {
            v & regs::IS_UCCS != 0
        })?;
        self.wr(regs::IS, regs::IS_UCCS);
        let result = (self.rd(regs::UCMDARG2) & 0xff) as u8;
        if result != 0 {
            return Err(Error::Uic { opcode, result });
        }
        Ok(self.rd(regs::UCMDARG3))
    }

    /// DME_GET of a local attribute.
    pub fn dme_get(&mut self, attr: u16, sel: u16) -> Result<u32> {
        self.uic(regs::DME_GET, ((attr as u32) << 16) | sel as u32, 0, 0)
    }

    /// DME_SET of a local, volatile ("normal" set type) attribute.
    pub fn dme_set(&mut self, attr: u16, sel: u16, value: u32) -> Result<()> {
        self.uic(regs::DME_SET, ((attr as u32) << 16) | sel as u32, 0, value)
            .map(|_| ())
    }

    /// Steps 6–7: Qualcomm pre-link-startup DME setup, `DME_LINKSTARTUP`, HCS.DP check.
    pub fn link_startup(&mut self) -> Result<()> {
        if let Some(q) = self.cfg.qcom {
            let mut v = self.dme_get(qcom::DME_VS_CORE_CLK_CTRL, 0)?;
            v &= !(qcom::CLK_1US_CYCLES_MASK | qcom::CORE_CLK_DIV_EN);
            v |= q.unipro_core_clk_mhz as u32;
            self.dme_set(qcom::DME_VS_CORE_CLK_CTRL, 0, v)?;
            let lanes = self.dme_get(regs::PA_CONNECTED_TX_DATA_LANES, 0)?;
            for lane in 0..lanes.min(4) {
                self.dme_set(regs::TX_LCC_ENABLE, lane as u16, 0)?;
            }
        }
        self.uic(regs::DME_LINKSTARTUP, 0, 0, 0)?;
        if self.rd(regs::HCS) & regs::HCS_DP == 0 {
            return Err(Error::NoDevice);
        }
        Ok(())
    }

    /// Step 8: clear residual IS, program the UTRL base, check ready, run.
    pub fn start_list(&mut self) -> Result<()> {
        let is = self.rd(regs::IS);
        self.wr(regs::IS, is);
        let base = self.dma.phys() + dma::UTRL_OFF as u64;
        self.wr(regs::UTRLBA, base as u32);
        self.wr(regs::UTRLBAU, (base >> 32) as u32);
        if self.rd(regs::HCS) & regs::HCS_UTRLRDY == 0 || self.rd(regs::UTRLDBR) != 0 {
            return Err(Error::ListNotReady);
        }
        self.wr(regs::UTRLRSR, 1);
        Ok(())
    }
}
