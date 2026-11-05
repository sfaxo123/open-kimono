//! EP0 control-transfer state machine (spec §4 "Control transfers").
//!
//! The controller owns the stage order (SETUP → optional DATA → STATUS); the
//! USB stack only supplies data and the accept/reject decision. Status TRBs
//! are armed when the controller reports XferNotReady(status) on the status
//! direction, whichever of that event and the decision comes first.

use crate::controller::{Dwc3, Error};
use crate::event::{EpEvent, STAGE_STATUS};
use crate::layout;
use crate::regs::{self, cmd};
use crate::trb::TrbCtl;
use mmio_replay::Mmio;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// Not started (before connect).
    Stopped,
    /// SETUP TRB armed on EP0-OUT.
    Setup,
    /// Setup packet received; the stack decides.
    Request,
    /// CONTROL_DATA armed on EP0-IN (`zlp`: the extra zero-length one).
    DataIn { zlp: bool },
    /// CONTROL_DATA armed on EP0-OUT.
    DataOut,
    /// OUT data stage finished; the stack reads it from the EP0 buffer.
    DataOutDone,
    /// Waiting for XferNotReady(status) on `status_phys`.
    AwaitStatus,
    /// STATUS TRB armed on `status_phys`.
    Status,
}

#[derive(Clone, Copy, Debug)]
pub struct Ep0 {
    pub stage: Stage,
    pub setup: [u8; 8],
    pub taken: bool,
    /// Increments with every setup packet and every abort.
    pub seq: u32,
    /// Sequence number whose IN data stage completed.
    pub in_done_seq: u32,
    pub mps: u16,
    pub in_len: usize,
    pub zlp_wanted: bool,
    pub out_len: usize,
    pub out_pos: usize,
    pub status_phys: u8,
    pub status_ctl: TrbCtl,
    pub nrdy_status: [bool; 2],
    pub rsrc: [Option<u8>; 2],
}

impl Ep0 {
    pub fn new() -> Self {
        Self {
            stage: Stage::Stopped,
            setup: [0; 8],
            taken: false,
            seq: 0,
            in_done_seq: u32::MAX,
            mps: 512,
            in_len: 0,
            zlp_wanted: false,
            out_len: 0,
            out_pos: 0,
            status_phys: 1,
            status_ctl: TrbCtl::Status2,
            nrdy_status: [false; 2],
            rsrc: [None; 2],
        }
    }

    pub fn w_length(&self) -> usize {
        u16::from_le_bytes([self.setup[6], self.setup[7]]) as usize
    }
}

/// Outcome of starting an EP0 data-stage step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Begin {
    /// Chunk buffered, nothing armed yet.
    Queued,
    /// Data TRB armed; wait for completion.
    Armed,
    /// The request was superseded (new SETUP, reset, stall).
    Aborted,
    /// Larger than the EP0 buffer; EP0 was stalled.
    Overflow,
}

impl Default for Ep0 {
    fn default() -> Self {
        Self::new()
    }
}

impl<R: Mmio, D: Mmio> Dwc3<R, D> {
    /// DEPSTARTCFG(0), configure both EP0 directions, enable them, arm SETUP.
    pub(crate) fn ep0_start(&mut self) -> Result<(), Error> {
        self.ep0 = Ep0::new();
        self.command(0, cmd::START_NEW_CONFIG, 0, [0; 3], 0)?;
        self.configure_ep(0, 0, 512, 0, false)?;
        self.configure_ep(1, 0, 512, 0, false)?;
        let v = self.rd(regs::DALEPENA);
        self.wr(regs::DALEPENA, v | 0b11);
        self.ep0_queue_setup()
    }

    fn ep0_trb(&mut self, phys: u8, ctl: TrbCtl, len: u32) -> Result<(), Error> {
        let rsrc = self.start_trb(phys, ctl, layout::EP0_BUF, len)?;
        self.ep0.rsrc[phys as usize] = Some(rsrc);
        Ok(())
    }

    fn ep0_queue_setup(&mut self) -> Result<(), Error> {
        self.ep0.stage = Stage::Setup;
        self.ep0.taken = false;
        self.ep0.nrdy_status = [false; 2];
        self.ep0_trb(0, TrbCtl::Setup, 8)
    }

    fn ep0_end_all(&mut self) -> Result<(), Error> {
        for phys in 0..2u8 {
            if let Some(r) = self.ep0.rsrc[phys as usize].take() {
                self.end_transfer(phys, r)?;
            }
        }
        Ok(())
    }

    /// Protocol stall: end transfers, stall EP0, re-arm SETUP.
    pub(crate) fn ep0_stall_restart(&mut self) -> Result<(), Error> {
        self.ep0.seq = self.ep0.seq.wrapping_add(1);
        self.ep0_end_all()?;
        self.command(0, cmd::SET_STALL, 0, [0; 3], 0)?;
        self.ep0_queue_setup()
    }

    pub(crate) fn ep0_bus_reset(&mut self) -> Result<(), Error> {
        self.ep0.seq = self.ep0.seq.wrapping_add(1);
        if self.ep0.stage == Stage::Setup || self.ep0.stage == Stage::Stopped {
            return Ok(());
        }
        self.ep0_end_all()?;
        self.ep0_queue_setup()
    }

    fn ep0_await_status(&mut self, phys: u8, ctl: TrbCtl) -> Result<(), Error> {
        self.ep0.status_phys = phys;
        self.ep0.status_ctl = ctl;
        self.ep0.stage = Stage::AwaitStatus;
        if self.ep0.nrdy_status[phys as usize] {
            self.ep0_arm_status()?;
        }
        Ok(())
    }

    fn ep0_arm_status(&mut self) -> Result<(), Error> {
        self.ep0.stage = Stage::Status;
        self.ep0_trb(self.ep0.status_phys, self.ep0.status_ctl, 0)
    }

    pub(crate) fn ep0_event(&mut self, phys: u8, kind: EpEvent) -> Result<(), Error> {
        match kind {
            EpEvent::XferComplete => self.ep0_complete(phys),
            EpEvent::XferNotReady { stage: STAGE_STATUS, .. } => {
                self.ep0.nrdy_status[phys as usize] = true;
                if self.ep0.stage == Stage::AwaitStatus && phys == self.ep0.status_phys {
                    self.ep0_arm_status()?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn ep0_complete(&mut self, phys: u8) -> Result<(), Error> {
        self.ep0.rsrc[phys as usize] = None;
        match (self.ep0.stage, phys) {
            (Stage::Setup, 0) => {
                let a = self.cfg.dma_base + layout::EP0_BUF;
                for i in 0..8 {
                    self.ep0.setup[i] = self.dma.read_u8(a + i as u64);
                }
                self.ep0.seq = self.ep0.seq.wrapping_add(1);
                self.ep0.taken = false;
                self.ep0.nrdy_status = [false; 2];
                self.ep0.stage = Stage::Request;
                Ok(())
            }
            (Stage::DataIn { zlp: false }, 1) if self.ep0.zlp_wanted => {
                self.ep0.stage = Stage::DataIn { zlp: true };
                self.ep0_trb(1, TrbCtl::ControlData, 0)
            }
            (Stage::DataIn { .. }, 1) => {
                self.ep0.in_done_seq = self.ep0.seq;
                self.ep0_await_status(0, TrbCtl::Status3)
            }
            (Stage::DataOut, 0) => {
                let trb = self.load_trb(0);
                let armed = round_up(self.ep0.w_length(), self.ep0.mps as usize);
                let got = armed.saturating_sub(trb.remaining() as usize);
                if got > self.ep0.w_length() {
                    return self.ep0_stall_restart();
                }
                self.ep0.out_len = got;
                self.ep0.out_pos = 0;
                self.ep0.stage = Stage::DataOutDone;
                Ok(())
            }
            (Stage::Status, p) if p == self.ep0.status_phys => self.ep0_queue_setup(),
            _ => self.ep0_stall_restart(),
        }
    }

    /// The pending setup packet, once per packet.
    pub fn ep0_take_setup(&mut self) -> Option<([u8; 8], u32)> {
        if self.ep0.stage == Stage::Request && !self.ep0.taken {
            self.ep0.taken = true;
            return Some((self.ep0.setup, self.ep0.seq));
        }
        None
    }

    /// Queue one IN chunk; arms the data TRB on `last`.
    pub(crate) fn ep0_in_begin(&mut self, seq: u32, data: &[u8], first: bool, last: bool) -> Result<Begin, Error> {
        if seq != self.ep0.seq || self.ep0.stage != Stage::Request {
            return Ok(Begin::Aborted);
        }
        if first {
            self.ep0.in_len = 0;
            self.ep0.zlp_wanted = false;
        }
        if data.is_empty() && !first {
            self.ep0.zlp_wanted = true;
        }
        if self.ep0.in_len + data.len() > layout::EP0_BUF_LEN {
            self.ep0_stall_restart()?;
            return Ok(Begin::Overflow);
        }
        let start = layout::EP0_BUF + self.ep0.in_len as u64;
        for (i, b) in data.iter().enumerate() {
            self.dma.write_u8(self.cfg.dma_base + start + i as u64, *b);
        }
        self.ep0.in_len += data.len();
        if !last {
            return Ok(Begin::Queued);
        }
        self.ep0.stage = Stage::DataIn { zlp: false };
        self.ep0_trb(1, TrbCtl::ControlData, self.ep0.in_len as u32)?;
        Ok(Begin::Armed)
    }

    /// `Some(true)` done, `Some(false)` aborted, `None` still running.
    pub(crate) fn ep0_in_status(&self, seq: u32) -> Option<bool> {
        if self.ep0.in_done_seq == seq {
            Some(true)
        } else if self.ep0.seq != seq {
            Some(false)
        } else {
            None
        }
    }

    /// Arm the whole OUT data stage (MPS-rounded, spec §4).
    pub(crate) fn ep0_out_begin(&mut self, seq: u32) -> Result<Begin, Error> {
        if seq != self.ep0.seq || self.ep0.stage != Stage::Request {
            return Ok(Begin::Aborted);
        }
        let len = round_up(self.ep0.w_length(), self.ep0.mps as usize);
        if len > layout::EP0_BUF_LEN {
            self.ep0_stall_restart()?;
            return Ok(Begin::Overflow);
        }
        self.ep0.stage = Stage::DataOut;
        self.ep0_trb(0, TrbCtl::ControlData, len as u32)?;
        Ok(Begin::Armed)
    }

    /// Copy the next received OUT bytes; `None` while the data stage runs, `Some(None)` if aborted.
    pub(crate) fn ep0_out_read(&mut self, seq: u32, buf: &mut [u8]) -> Option<Option<usize>> {
        if seq != self.ep0.seq {
            return Some(None);
        }
        match self.ep0.stage {
            Stage::DataOut => None,
            Stage::DataOutDone => {
                let n = buf.len().min(self.ep0.out_len - self.ep0.out_pos);
                let a = self.cfg.dma_base + layout::EP0_BUF + self.ep0.out_pos as u64;
                for (i, b) in buf[..n].iter_mut().enumerate() {
                    *b = self.dma.read_u8(a + i as u64);
                }
                self.ep0.out_pos += n;
                Some(Some(n))
            }
            _ => Some(None),
        }
    }

    pub(crate) fn ep0_accept(&mut self, seq: u32) -> Result<(), Error> {
        if seq != self.ep0.seq {
            return Ok(());
        }
        match self.ep0.stage {
            Stage::Request => self.ep0_await_status(1, TrbCtl::Status2),
            Stage::DataOutDone => self.ep0_await_status(1, TrbCtl::Status3),
            _ => Ok(()),
        }
    }

    pub(crate) fn ep0_reject(&mut self, seq: u32) -> Result<(), Error> {
        if seq != self.ep0.seq {
            return Ok(());
        }
        self.ep0_stall_restart()
    }

    /// SET_ADDRESS: DCFG.DEVADDR before the status stage (spec §4).
    pub(crate) fn ep0_set_address(&mut self, seq: u32, addr: u8) -> Result<(), Error> {
        if seq == self.ep0.seq {
            let v = self.rd(regs::DCFG) & !regs::DCFG_DEVADDR_MASK;
            self.wr(regs::DCFG, v | ((addr as u32 & 0x7f) << regs::DCFG_DEVADDR_SHIFT));
        }
        self.ep0_accept(seq)
    }
}

fn round_up(n: usize, m: usize) -> usize {
    n.div_ceil(m) * m
}
