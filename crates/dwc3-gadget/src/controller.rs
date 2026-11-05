//! Polled DWC3 device-mode core: init, connect, endpoint commands, event ring.

use crate::ep::EpState;
use crate::ep0::Ep0;
use crate::layout;
use crate::regs::{self, cmd, qscratch};
use crate::trb::{Trb, TrbCtl};
use mmio_replay::Mmio;

/// Board/controller parameters.
#[derive(Clone, Copy, Debug)]
pub struct Config {
    /// `snps,dwc3` register window.
    pub core_base: u64,
    /// `qcom,dwc3` QSCRATCH window.
    pub qscratch_base: u64,
    /// Bus (device-visible) address of the [`layout::DMA_SIZE`] DMA region.
    pub dma_base: u64,
    /// DT `qcom,select-utmi-as-pipe-clk` (USB2-only boards).
    pub select_utmi_as_pipe_clk: bool,
    /// Upper bound for every register poll loop.
    pub poll_limit: u32,
    /// Busy-wait helper (microseconds).
    pub delay_us: fn(u32),
}

impl Config {
    /// OnePlus 6 (SDM845) `usb_1`, peripheral-only, high-speed (spec §1).
    pub fn sdm845_usb1(dma_base: u64, delay_us: fn(u32)) -> Self {
        Self {
            core_base: 0x0a60_0000,
            qscratch_base: 0x0a6f_8800,
            dma_base,
            select_utmi_as_pipe_clk: true,
            poll_limit: 10_000,
            delay_us,
        }
    }
}

/// Failures the core reports instead of hanging or guessing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// `GSNPSID` is not a DWC_usb3/usb31 core (raw value).
    UnsupportedCore(u32),
    /// `GHWPARAMS3` endpoint counts are unusable (raw value).
    BadHwParams(u32),
    /// DMA region not 4 KiB aligned.
    UnalignedDma(u64),
    /// `DCTL.CSFTRST` never cleared.
    ResetTimeout,
    /// `DSTS.DEVCTRLHLT` never reached the requested state.
    HaltTimeout { running: bool },
    /// `DEPCMD.CMDACT` never cleared.
    CommandTimeout { phys: u8, cmd: u32 },
    /// `DEPCMD.CMDSTATUS` non-zero.
    CommandFailed { phys: u8, cmd: u32, status: u32 },
    /// `GEVNTCOUNT` larger than the ring (raw count).
    EventCount(u32),
    /// Operation needs [`Dwc3::init`] first.
    NotInitialized,
    /// Host connected at a speed this board cannot run (`DSTS.CONNECTSPD`).
    UnsupportedSpeed(u32),
}

/// Bus-level events for the USB stack, in arrival order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BusEvent {
    PowerDetected,
    Reset,
    Suspend,
    Resume,
}

pub(crate) const BUS_Q: usize = 8;

/// The controller: register window `R`, DMA window `D` (bus addresses).
pub struct Dwc3<R: Mmio, D: Mmio> {
    pub(crate) regs: R,
    pub(crate) dma: D,
    pub(crate) cfg: Config,
    pub(crate) num_eps: u8,
    pub(crate) num_in: u8,
    initialized: bool,
    pub(crate) running: bool,
    pub(crate) ev_read: u32,
    pub(crate) bus_q: [Option<BusEvent>; BUS_Q],
    pub(crate) power_reported: bool,
    pub(crate) ep0: Ep0,
    pub(crate) eps: [EpState; 32],
    pub(crate) config_started: bool,
    fault: Option<Error>,
    /// Events handled since creation.
    pub events_handled: u32,
    /// Event words this driver ignored.
    pub events_ignored: u32,
}

impl<R: Mmio, D: Mmio> Dwc3<R, D> {
    pub fn new(regs: R, dma: D, cfg: Config) -> Self {
        Self {
            regs,
            dma,
            cfg,
            num_eps: 0,
            num_in: 0,
            initialized: false,
            running: false,
            ev_read: 0,
            bus_q: [None; BUS_Q],
            power_reported: false,
            ep0: Ep0::new(),
            eps: [EpState::new(); 32],
            config_started: false,
            fault: None,
            events_handled: 0,
            events_ignored: 0,
        }
    }

    pub fn into_parts(self) -> (R, D) {
        (self.regs, self.dma)
    }

    /// First error raised from a path that cannot return one (async callbacks).
    pub fn fault(&self) -> Option<Error> {
        self.fault
    }

    pub fn is_running(&self) -> bool {
        self.running
    }

    /// Physical endpoint counts `(total, in)` from `GHWPARAMS3`.
    pub fn endpoint_counts(&self) -> (u8, u8) {
        (self.num_eps, self.num_in)
    }

    pub(crate) fn note(&mut self, r: Result<(), Error>) {
        if let Err(e) = r {
            self.fault.get_or_insert(e);
        }
    }

    pub(crate) fn rd(&mut self, off: u64) -> u32 {
        self.regs.read_u32(self.cfg.core_base + off)
    }
    pub(crate) fn wr(&mut self, off: u64, v: u32) {
        self.regs.write_u32(self.cfg.core_base + off, v)
    }
    fn rmw(&mut self, off: u64, clear: u32, set: u32) {
        let v = self.rd(off);
        self.wr(off, (v & !clear) | set);
    }
    fn qrmw(&mut self, off: u64, clear: u32, set: u32) {
        let a = self.cfg.qscratch_base + off;
        let v = self.regs.read_u32(a);
        self.regs.write_u32(a, (v & !clear) | set);
    }

    /// Identify, soft-reset and put the core in device mode (spec §4 Init).
    pub fn init(&mut self) -> Result<(), Error> {
        if self.cfg.dma_base & 0xfff != 0 {
            return Err(Error::UnalignedDma(self.cfg.dma_base));
        }
        let id = self.rd(regs::GSNPSID);
        if !matches!(id >> 16, regs::CORE_ID_USB3 | regs::CORE_ID_USB31) {
            return Err(Error::UnsupportedCore(id));
        }
        let hw3 = self.rd(regs::GHWPARAMS3);
        let total = (hw3 >> 12) & 0x3f;
        let ins = (hw3 >> 18) & 0x1f;
        if !(2..=32).contains(&total) || ins == 0 || ins > 16 || ins >= total || total - ins > 16 {
            return Err(Error::BadHwParams(hw3));
        }
        self.rmw(regs::GUCTL1, 0, regs::GUCTL1_DEV_FORCE_20_CLK_FOR_30_CLK);
        self.rmw(regs::DCTL, regs::DCTL_RUN_STOP, regs::DCTL_CSFTRST);
        if !self.wait_reg(regs::DCTL, regs::DCTL_CSFTRST, 0) {
            return Err(Error::ResetTimeout);
        }
        self.rmw(regs::GCTL, regs::GCTL_PRTCAPDIR_MASK, regs::GCTL_PRTCAPDIR_DEVICE);
        self.wr(regs::GEVNTSIZ0, regs::GEVNTSIZ_INTMASK);
        self.rmw(regs::GUCTL1, 0, regs::GUCTL1_DEV_FORCE_20_CLK_FOR_30_CLK);
        self.num_eps = total as u8;
        self.num_in = ins as u8;
        self.initialized = true;
        Ok(())
    }

    /// Glue + PHY config, event ring, EP0, RUN/STOP (spec §4 Glue, Start peripheral).
    pub fn connect(&mut self) -> Result<(), Error> {
        if !self.initialized {
            return Err(Error::NotInitialized);
        }
        if self.cfg.select_utmi_as_pipe_clk {
            self.qrmw(qscratch::GENERAL_CFG, 0, qscratch::PIPE_UTMI_CLK_DIS);
            (self.cfg.delay_us)(100);
            let sel = qscratch::PIPE_UTMI_CLK_SEL | qscratch::PIPE3_PHYSTATUS_SW;
            self.qrmw(qscratch::GENERAL_CFG, 0, sel);
            (self.cfg.delay_us)(100);
            self.qrmw(qscratch::GENERAL_CFG, qscratch::PIPE_UTMI_CLK_DIS, 0);
        }
        self.vbus_override(true);
        self.rmw(
            regs::GUSB2PHYCFG0,
            regs::GUSB2PHYCFG_SUSPENDUSB20 | regs::GUSB2PHYCFG_ENBLSLPM | regs::GUSB2PHYCFG_USBTRDTIM_MASK,
            regs::GUSB2PHYCFG_USBTRDTIM_8BIT,
        );
        self.rmw(regs::DCFG, regs::DCFG_DEVSPD_MASK | regs::DCFG_DEVADDR_MASK, regs::DCFG_DEVSPD_HIGH);
        let ring = self.cfg.dma_base + layout::EVENT_RING;
        self.wr(regs::GEVNTADR_LO0, ring as u32);
        self.wr(regs::GEVNTADR_HI0, (ring >> 32) as u32);
        self.wr(regs::GEVNTSIZ0, layout::EVENT_RING_LEN);
        self.wr(regs::GEVNTCOUNT0, 0);
        self.ev_read = 0;
        self.wr(
            regs::DEVTEN,
            regs::DEVTEN_DISCONN | regs::DEVTEN_USBRST | regs::DEVTEN_CONNECTDONE | regs::DEVTEN_WKUP
                | regs::DEVTEN_U3L2L1SUSP,
        );
        self.ep0_start()?;
        self.rmw(regs::DCTL, 0, regs::DCTL_RUN_STOP);
        if !self.wait_reg(regs::DSTS, regs::DSTS_DEVCTRLHLT, 0) {
            return Err(Error::HaltTimeout { running: true });
        }
        self.running = true;
        Ok(())
    }

    /// Drop off the bus: VBUS override off, RUN/STOP clear, events masked.
    pub fn disconnect(&mut self) -> Result<(), Error> {
        if !self.initialized {
            return Err(Error::NotInitialized);
        }
        self.vbus_override(false);
        self.rmw(regs::DCTL, regs::DCTL_RUN_STOP, 0);
        let halted = self.wait_reg(regs::DSTS, regs::DSTS_DEVCTRLHLT, regs::DSTS_DEVCTRLHLT);
        self.wr(regs::GEVNTSIZ0, regs::GEVNTSIZ_INTMASK | layout::EVENT_RING_LEN);
        self.running = false;
        self.ep0 = Ep0::new();
        for ep in self.eps.iter_mut() {
            ep.reset_runtime();
        }
        self.config_started = false;
        if halted { Ok(()) } else { Err(Error::HaltTimeout { running: false }) }
    }

    fn vbus_override(&mut self, on: bool) {
        let hs = qscratch::UTMI_OTG_VBUS_VALID | qscratch::SW_SESSVLD_SEL;
        let ss = qscratch::LANE0_PWR_PRESENT;
        if on {
            self.qrmw(qscratch::SS_PHY_CTRL, 0, ss);
            self.qrmw(qscratch::HS_PHY_CTRL, 0, hs);
        } else {
            self.qrmw(qscratch::SS_PHY_CTRL, ss, 0);
            self.qrmw(qscratch::HS_PHY_CTRL, hs, 0);
        }
    }

    fn wait_reg(&mut self, off: u64, mask: u32, want: u32) -> bool {
        (0..self.cfg.poll_limit).any(|_| self.rd(off) & mask == want)
    }

    /// Issue an endpoint command and wait for `CMDACT` to clear; returns the final `DEPCMD`.
    pub(crate) fn command(&mut self, phys: u8, ty: u32, param: u32, par: [u32; 3], extra: u32) -> Result<u32, Error> {
        let b = regs::depcmd_base(phys);
        self.wr(b + regs::DEPCMDPAR0, par[0]);
        self.wr(b + regs::DEPCMDPAR1, par[1]);
        self.wr(b + regs::DEPCMDPAR2, par[2]);
        self.wr(b + regs::DEPCMD, (param << regs::DEPCMD_PARAM_SHIFT) | extra | regs::DEPCMD_CMDACT | ty);
        for _ in 0..self.cfg.poll_limit {
            let v = self.rd(b + regs::DEPCMD);
            if v & regs::DEPCMD_CMDACT == 0 {
                return match regs::depcmd_status(v) {
                    0 => Ok(v),
                    status => Err(Error::CommandFailed { phys, cmd: ty, status }),
                };
            }
        }
        Err(Error::CommandTimeout { phys, cmd: ty })
    }

    /// DEPCFG(init or modify) + DEPXFERCFG as needed.
    pub(crate) fn configure_ep(&mut self, phys: u8, ep_type: u32, mps: u16, interval_m1: u8, modify: bool) -> Result<(), Error> {
        let fifo = if phys & 1 == 1 { phys >> 1 } else { 0 };
        let par = [regs::depcfg_par0(ep_type, mps, fifo, modify), regs::depcfg_par1(phys, interval_m1), 0];
        self.command(phys, cmd::SET_EP_CONFIG, 0, par, 0)?;
        if !modify {
            self.command(phys, cmd::SET_XFER_RESOURCE, 0, [1, 0, 0], 0)?;
        }
        Ok(())
    }

    /// Write one TRB for `phys` and start it; returns the transfer resource index.
    pub(crate) fn start_trb(&mut self, phys: u8, ctl: TrbCtl, buf: u64, len: u32) -> Result<u8, Error> {
        let addr = self.cfg.dma_base + layout::trb(phys);
        Trb::single(ctl, self.cfg.dma_base + buf, len).store(&mut self.dma, addr);
        let v = self.command(phys, cmd::START_TRANSFER, 0, [(addr >> 32) as u32, addr as u32, 0], 0)?;
        Ok(((v >> regs::DEPCMD_PARAM_SHIFT) & 0x7f) as u8)
    }

    pub(crate) fn end_transfer(&mut self, phys: u8, rsrc: u8) -> Result<(), Error> {
        self.command(phys, cmd::END_TRANSFER, rsrc as u32, [0; 3], regs::DEPCMD_HIPRI_FORCERM)?;
        (self.cfg.delay_us)(1000);
        Ok(())
    }

    pub(crate) fn load_trb(&mut self, phys: u8) -> Trb {
        Trb::load(&mut self.dma, self.cfg.dma_base + layout::trb(phys))
    }
}
