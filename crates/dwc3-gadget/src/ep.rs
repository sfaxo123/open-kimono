//! Bulk/interrupt endpoints: one single-packet TRB per transfer (spec §4).

use crate::controller::{Dwc3, Error};
use crate::layout;
use crate::regs::{self, cmd};
use crate::trb::TrbCtl;
use mmio_replay::Mmio;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Xfer {
    Idle,
    /// TRB armed for `len` bytes.
    Busy { len: u32 },
    /// Hardware finished; `actual` bytes moved.
    Done { actual: u32 },
}

#[derive(Clone, Copy, Debug)]
pub struct EpState {
    pub allocated: bool,
    /// USB transfer type (2 bulk, 3 interrupt).
    pub ep_type: u8,
    pub mps: u16,
    pub interval_m1: u8,
    pub enabled: bool,
    pub stalled: bool,
    pub xfer: Xfer,
    pub rsrc: Option<u8>,
}

impl EpState {
    pub const fn new() -> Self {
        Self {
            allocated: false,
            ep_type: 0,
            mps: 0,
            interval_m1: 0,
            enabled: false,
            stalled: false,
            xfer: Xfer::Idle,
            rsrc: None,
        }
    }

    pub fn reset_runtime(&mut self) {
        self.enabled = false;
        self.stalled = false;
        self.xfer = Xfer::Idle;
        self.rsrc = None;
    }
}

impl Default for EpState {
    fn default() -> Self {
        Self::new()
    }
}

/// Physical endpoint number for USB endpoint `index` in direction `is_in`.
pub const fn phys(index: u8, is_in: bool) -> u8 {
    (index << 1) | is_in as u8
}

impl<R: Mmio, D: Mmio> Dwc3<R, D> {
    /// Reserve a hardware endpoint; `None` when no compatible one is free.
    pub fn alloc_ep(&mut self, is_in: bool, ep_type: u8, index: Option<u8>, mps: u16, interval_ms: u8) -> Option<u8> {
        if !matches!(ep_type, 2 | 3) || mps == 0 || mps as usize > layout::EP_BUF_LEN || self.num_eps == 0 {
            return None;
        }
        let count = if is_in { self.num_in } else { self.num_eps - self.num_in };
        let free = |s: &Self, i: u8| i >= 1 && i < count && phys(i, is_in) < 32 && !s.eps[phys(i, is_in) as usize].allocated;
        let i = match index {
            Some(i) => free(self, i).then_some(i)?,
            None => (1..count).find(|&i| free(self, i))?,
        };
        let ep = &mut self.eps[phys(i, is_in) as usize];
        ep.allocated = true;
        ep.ep_type = ep_type;
        ep.mps = mps;
        ep.interval_m1 = if ep_type == 3 { interval_ms.clamp(1, 16) - 1 } else { 0 };
        Some(i)
    }

    /// Enable: DEPSTARTCFG(2) once per configuration, DEPCFG, DEPXFERCFG, DALEPENA.
    pub fn ep_set_enabled(&mut self, phys: u8, enabled: bool) -> Result<(), Error> {
        let ep = self.eps[phys as usize];
        if !ep.allocated || ep.enabled == enabled {
            return Ok(());
        }
        let bit = 1u32 << phys;
        if enabled {
            if !self.config_started {
                self.command(0, cmd::START_NEW_CONFIG, 2, [0; 3], 0)?;
                self.config_started = true;
            }
            self.configure_ep(phys, ep.ep_type as u32, ep.mps, ep.interval_m1, false)?;
            let v = self.rd(regs::DALEPENA);
            self.wr(regs::DALEPENA, v | bit);
            let e = &mut self.eps[phys as usize];
            e.reset_runtime();
            e.enabled = true;
        } else {
            self.ep_end(phys)?;
            let v = self.rd(regs::DALEPENA);
            self.wr(regs::DALEPENA, v & !bit);
            self.eps[phys as usize].reset_runtime();
            if self.eps[2..].iter().all(|e| !e.enabled) {
                self.config_started = false;
            }
        }
        Ok(())
    }

    pub(crate) fn disable_user_eps(&mut self) {
        for p in 2..32u8 {
            let r = self.ep_set_enabled(p, false);
            self.note(r);
        }
        self.config_started = false;
    }

    fn ep_end(&mut self, phys: u8) -> Result<(), Error> {
        if let (Xfer::Busy { .. }, Some(r)) = (self.eps[phys as usize].xfer, self.eps[phys as usize].rsrc) {
            self.end_transfer(phys, r)?;
        }
        let e = &mut self.eps[phys as usize];
        e.xfer = Xfer::Idle;
        e.rsrc = None;
        Ok(())
    }

    pub fn ep_set_stalled(&mut self, phys: u8, stalled: bool) -> Result<(), Error> {
        let ep = self.eps[phys as usize];
        if !ep.enabled || ep.stalled == stalled {
            return Ok(());
        }
        if stalled {
            self.ep_end(phys)?;
            self.command(phys, cmd::SET_STALL, 0, [0; 3], 0)?;
        } else {
            self.command(phys, cmd::CLEAR_STALL, 0, [0; 3], 0)?;
        }
        self.eps[phys as usize].stalled = stalled;
        Ok(())
    }

    /// Copy `data` (IN) into the bounce buffer, or size an MPS read (OUT), and start it.
    pub(crate) fn ep_start(&mut self, phys: u8, data: Option<&[u8]>) -> Result<(), Error> {
        let len = match data {
            Some(d) => {
                let a = self.cfg.dma_base + layout::ep_buf(phys);
                for (i, b) in d.iter().enumerate() {
                    self.dma.write_u8(a + i as u64, *b);
                }
                d.len() as u32
            }
            None => self.eps[phys as usize].mps as u32,
        };
        let rsrc = self.start_trb(phys, TrbCtl::Normal, layout::ep_buf(phys), len)?;
        let e = &mut self.eps[phys as usize];
        e.xfer = Xfer::Busy { len };
        e.rsrc = Some(rsrc);
        Ok(())
    }

    pub(crate) fn ep_read_buf(&mut self, phys: u8, out: &mut [u8]) {
        let a = self.cfg.dma_base + layout::ep_buf(phys);
        for (i, b) in out.iter_mut().enumerate() {
            *b = self.dma.read_u8(a + i as u64);
        }
    }

    pub(crate) fn ep_complete(&mut self, phys: u8) {
        if let Xfer::Busy { len } = self.eps[phys as usize].xfer {
            let actual = len.saturating_sub(self.load_trb(phys).remaining());
            let e = &mut self.eps[phys as usize];
            e.xfer = Xfer::Done { actual };
            e.rsrc = None;
        } else {
            self.events_ignored += 1;
        }
    }
}
