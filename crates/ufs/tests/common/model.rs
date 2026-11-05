//! UFSHCI register model with Qualcomm vendor registers. On a doorbell it hands the slot's UTRD
//! to [`UfsDevice`]. Unknown offsets and non-32-bit accesses panic: they are driver bugs.

use super::device::UfsDevice;
use super::{SharedDma, BASE};
use mmio_replay::{Mmio, Width};
use std::collections::BTreeMap;
use ufs_core::regs::{self, qcom};

/// Register-level faults.
#[derive(Clone, Default)]
pub struct HwFaults {
    pub phy_dead: bool,
    pub link_result: u8,
    pub no_device: bool,
    pub hce_stuck_on: bool,
    pub hce_stuck_off: bool,
    pub uic_silent: bool,
    pub hang_doorbell: bool,
    pub clear_ignored: bool,
    pub fatal_is: Option<u32>,
    pub list_not_ready: bool,
}

pub struct UfsModel {
    pub dma: SharedDma,
    pub dev: UfsDevice,
    pub hw: HwFaults,
    pub cap: u32,
    pub ver: u32,
    pub hw_version: u32,
    pub hce: bool,
    pub link_up: bool,
    pub is: u32,
    pub ie: u32,
    pub cfg0: u32,
    pub cfg1: u32,
    pub utrlba: u64,
    pub rsr: u32,
    pub dbr: u32,
    pub args: [u32; 3],
    pub dme: BTreeMap<(u16, u16), u32>,
    pub uic_log: Vec<(u8, u32, u32)>,
}

/// Reset value of `DME_VS_CORE_CLK_CTRL` in the model (DIV_EN set, wrong cycle count).
pub const CORE_CLK_CTRL_RESET: u32 = 0x0000_0113;

impl UfsModel {
    /// An SDM845-like controller: 32 slots, 64-bit addressing, UFSHCI 2.0, qcom HW 3.1.0.
    pub fn sdm845(dma: SharedDma, dev: UfsDevice) -> Self {
        UfsModel {
            dma,
            dev,
            hw: HwFaults::default(),
            cap: 0x0107_001f,
            ver: 0x0000_0200,
            hw_version: 0x3001_0000,
            hce: false,
            link_up: false,
            is: 0,
            ie: 0,
            cfg0: 0,
            cfg1: 0,
            utrlba: 0,
            rsr: 0,
            dbr: 0,
            args: [0; 3],
            dme: BTreeMap::new(),
            uic_log: Vec::new(),
        }
    }

    fn reset_unipro(&mut self) {
        self.link_up = false;
        self.rsr = 0;
        self.dbr = 0;
        self.dme.clear();
        self.dme
            .insert((qcom::DME_VS_CORE_CLK_CTRL, 0), CORE_CLK_CTRL_RESET);
    }

    fn hcs(&self) -> u32 {
        let mut v = 0;
        if self.hce {
            v |= regs::HCS_UCRDY;
        }
        if self.link_up && !self.hw.no_device {
            v |= regs::HCS_DP;
            if !self.hw.list_not_ready {
                v |= regs::HCS_UTRLRDY;
            }
        }
        v
    }

    fn uic(&mut self, op: u8) {
        self.uic_log.push((op, self.args[0], self.args[2]));
        if self.hw.uic_silent {
            return;
        }
        let key = ((self.args[0] >> 16) as u16, self.args[0] as u16);
        let mut result = 0;
        match op {
            regs::DME_GET => {
                let v = match key.0 {
                    qcom::MPHY_TX_FSM_STATE => Some(if self.hw.phy_dead {
                        0
                    } else {
                        qcom::TX_FSM_HIBERN8
                    }),
                    regs::PA_CONNECTED_TX_DATA_LANES => Some(2),
                    _ => self.dme.get(&key).copied(),
                };
                match v {
                    Some(v) => self.args[2] = v,
                    None => result = 0x01,
                }
            }
            regs::DME_SET => {
                self.dme.insert(key, self.args[2]);
            }
            regs::DME_LINKSTARTUP => {
                result = self.hw.link_result;
                if result == 0 && !self.hw.phy_dead {
                    self.link_up = true;
                }
            }
            _ => result = 0x01,
        }
        self.args[1] = result as u32;
        self.is |= regs::IS_UCCS;
    }

    fn doorbell(&mut self, v: u32) {
        if self.rsr == 0 {
            return;
        }
        if let Some(f) = self.hw.fatal_is {
            self.is |= f;
            self.dbr |= v;
            return;
        }
        if self.hw.hang_doorbell {
            self.dbr |= v;
            return;
        }
        for slot in 0..32 {
            if v & (1 << slot) != 0 {
                let utrd = self.utrlba + 32 * slot as u64;
                self.dev.process(&self.dma, utrd);
                self.is |= regs::IS_UTRCS;
            }
        }
    }
}

impl Mmio for UfsModel {
    fn read(&mut self, addr: u64, width: Width) -> u64 {
        assert_eq!(width, Width::W32, "UFSHCI registers are 32-bit");
        (match addr - BASE {
            regs::CAP => self.cap,
            regs::VER => self.ver,
            regs::IS => self.is,
            regs::IE => self.ie,
            regs::HCS => self.hcs(),
            regs::HCE => self.hce as u32,
            regs::UTRLBA => self.utrlba as u32,
            regs::UTRLBAU => (self.utrlba >> 32) as u32,
            regs::UTRLDBR => self.dbr,
            regs::UTRLRSR => self.rsr,
            regs::UCMDARG1 => self.args[0],
            regs::UCMDARG2 => self.args[1],
            regs::UCMDARG3 => self.args[2],
            qcom::CFG0 => self.cfg0,
            qcom::CFG1 => self.cfg1,
            qcom::HW_VERSION => self.hw_version,
            off => panic!("read of unmodelled register {off:#x}"),
        }) as u64
    }

    fn write(&mut self, addr: u64, width: Width, value: u64) {
        assert_eq!(width, Width::W32, "UFSHCI registers are 32-bit");
        let v = value as u32;
        match addr - BASE {
            regs::IS => self.is &= !v,
            regs::IE => self.ie = v,
            regs::HCE if v & regs::HCE_ENABLE != 0 => {
                if !self.hw.hce_stuck_off {
                    self.hce = true;
                    self.reset_unipro();
                }
            }
            regs::HCE => {
                if !self.hw.hce_stuck_on {
                    self.hce = false;
                    self.reset_unipro();
                }
            }
            regs::UTRLBA => self.utrlba = (self.utrlba & !0xffff_ffff) | v as u64,
            regs::UTRLBAU => self.utrlba = (self.utrlba & 0xffff_ffff) | (v as u64) << 32,
            regs::UTRLRSR => self.rsr = v & 1,
            regs::UTRLDBR => self.doorbell(v),
            regs::UTRLCLR => {
                if !self.hw.clear_ignored {
                    self.dbr &= v;
                }
            }
            regs::UCMDARG1 => self.args[0] = v,
            regs::UCMDARG2 => self.args[1] = v,
            regs::UCMDARG3 => self.args[2] = v,
            regs::UICCMD => self.uic(v as u8),
            qcom::CFG0 => self.cfg0 = v,
            qcom::CFG1 => self.cfg1 = v,
            off => panic!("write of unmodelled register {off:#x} = {v:#x}"),
        }
    }
}

/// `mmio-replay` has no blanket `&mut T` impl; this lets tests keep the model after the driver.
impl Mmio for &mut UfsModel {
    fn read(&mut self, addr: u64, width: Width) -> u64 {
        (**self).read(addr, width)
    }
    fn write(&mut self, addr: u64, width: Width, value: u64) {
        (**self).write(addr, width, value)
    }
}
