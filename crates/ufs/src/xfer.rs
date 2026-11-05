//! One-slot UTP transfers: build UCD + UTRD in DMA, ring, poll, clear on timeout (`specs/ufs.md` §4).

use crate::dma::{self, Dma};
use crate::hc::{Completion, Ufshc};
use crate::regs;
use crate::upiu::{self, Direction, Utrd};
use crate::{Delay, Error, Result, Stage};
use alloc::vec;
use mmio_replay::Mmio;

/// The only slot used (single outstanding request, polled).
const SLOT: u32 = 0;

impl<M: Mmio, D: Dma, T: Delay> Ufshc<M, D, T> {
    pub(crate) fn next_tag(&mut self) -> u8 {
        let t = self.tag;
        self.tag = self.tag.wrapping_add(1);
        t
    }

    /// Run one request UPIU in slot 0, receiving up to `data_len` bytes into the data buffer.
    pub(crate) fn transfer(
        &mut self,
        req: &[u8; upiu::REQ_LEN],
        data_len: usize,
    ) -> Result<Completion> {
        // PRDT byte counts are dword granular; the UPIU carries the exact expected length.
        let prdt_bytes = (data_len + 3) & !3;
        if prdt_bytes > dma::data_capacity(self.dma.size()) {
            return Err(Error::InvalidArgument);
        }
        let phys = self.dma.phys();
        let ucd = dma::UCD_OFF;
        self.dma.write(ucd, &[0u8; dma::UCD_PRDT]);
        self.dma.write(ucd + dma::UCD_REQ, req);
        let (dir, prdt_len) = if data_len > 0 {
            let e = upiu::prdt_entry(phys + dma::DATA_OFF as u64, prdt_bytes);
            self.dma.write(ucd + dma::UCD_PRDT, &e);
            (Direction::DeviceToHost, 1)
        } else {
            (Direction::None, 0)
        };
        let utrd = Utrd {
            dir,
            ucd: phys + ucd as u64,
            resp_off_dw: (dma::UCD_RESP / 4) as u16,
            resp_len_dw: (dma::UCD_RESP_LEN / 4) as u16,
            prdt_off_dw: (dma::UCD_PRDT / 4) as u16,
            prdt_len,
        };
        let slot_off = dma::UTRL_OFF + SLOT as usize * dma::UTRD_LEN;
        self.dma.write(slot_off, &utrd.encode());
        self.wr(regs::UTRLDBR, 1 << SLOT);
        self.wait_slot()?;
        self.wr(regs::IS, regs::IS_UTRCS);
        let ocs = (dma::read_u32(&mut self.dma, slot_off + 8) & 0xff) as u8;
        if ocs != 0 {
            return Err(Error::Ocs(ocs));
        }
        let mut resp = vec![0u8; dma::UCD_RESP_LEN];
        self.dma.read(ucd + dma::UCD_RESP, &mut resp);
        let hdr = upiu::parse_header(&resp);
        let total = (upiu::RESP_HDR_LEN + hdr.data_len as usize).min(resp.len());
        resp.truncate(total);
        if hdr.tag != req[3] || hdr.trans != (req[0] | 0x20) {
            return Err(Error::BadUpiu);
        }
        Ok(Completion { resp })
    }

    fn wait_slot(&mut self) -> Result<()> {
        for _ in 0..self.cfg.polls.transfer {
            let is = self.rd(regs::IS);
            if is & regs::IS_FATAL != 0 {
                self.clear_slot()?;
                return Err(Error::Fatal(is));
            }
            if self.rd(regs::UTRLDBR) & (1 << SLOT) == 0 {
                return Ok(());
            }
            self.delay.delay_us(self.cfg.poll_delay_us);
        }
        self.clear_slot()?;
        Err(Error::Timeout(Stage::Doorbell))
    }

    fn clear_slot(&mut self) -> Result<()> {
        self.wr(regs::UTRLCLR, !(1u32 << SLOT));
        let polls = self.cfg.polls.clear;
        self.wait(Stage::Clear, regs::UTRLDBR, polls, |v| v & (1 << SLOT) == 0)
            .map(|_| ())
    }

    /// Copy `out.len()` bytes of the last data-in transfer out of the DMA buffer.
    pub(crate) fn copy_data(&mut self, out: &mut [u8]) {
        self.dma.read(dma::DATA_OFF, out);
    }
}
