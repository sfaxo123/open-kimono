//! Read-only SCSI over COMMAND UPIUs (`specs/ufs.md` §4 "SCSI").

use crate::hc::Ufshc;
use crate::{dma, upiu};
use crate::{Delay, Dma, Error, Result};
use mmio_replay::Mmio;

/// The only opcodes this driver will ever put in a CDB.
pub const ALLOWED_OPCODES: [u8; 5] = [0x00, 0x03, 0x12, 0x25, 0x28];

/// SCSI status CHECK CONDITION.
const CHECK_CONDITION: u8 = 0x02;
/// Sense key UNIT ATTENTION.
pub const UNIT_ATTENTION: u8 = 0x06;

/// Read-only command set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cmd {
    /// TEST UNIT READY.
    TestUnitReady,
    /// REQUEST SENSE (18 bytes, fixed format).
    RequestSense,
    /// INQUIRY (36 bytes standard data).
    Inquiry,
    /// READ CAPACITY(10) (8 bytes).
    ReadCapacity10,
    /// READ(10).
    Read10 {
        /// First logical block.
        lba: u32,
        /// Block count.
        blocks: u16,
    },
}

impl Cmd {
    /// The 16-byte CDB field of the COMMAND UPIU.
    pub fn cdb(&self) -> [u8; 16] {
        let mut c = [0u8; 16];
        match *self {
            Cmd::TestUnitReady => {}
            Cmd::RequestSense => {
                c[0] = 0x03;
                c[4] = 18;
            }
            Cmd::Inquiry => {
                c[0] = 0x12;
                c[4] = 36;
            }
            Cmd::ReadCapacity10 => c[0] = 0x25,
            Cmd::Read10 { lba, blocks } => {
                c[0] = 0x28;
                c[2..6].copy_from_slice(&lba.to_be_bytes());
                c[7..9].copy_from_slice(&blocks.to_be_bytes());
            }
        }
        c
    }

    /// Expected data-in length.
    pub fn data_len(&self, block_size: u32) -> usize {
        match *self {
            Cmd::TestUnitReady => 0,
            Cmd::RequestSense => 18,
            Cmd::Inquiry => 36,
            Cmd::ReadCapacity10 => 8,
            Cmd::Read10 { blocks, .. } => blocks as usize * block_size as usize,
        }
    }
}

/// Decoded sense data.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Sense {
    /// Sense key.
    pub key: u8,
    /// Additional sense code.
    pub asc: u8,
    /// Additional sense code qualifier.
    pub ascq: u8,
}

/// Parse fixed (0x70/0x71) or descriptor (0x72/0x73) format sense data.
pub fn parse_sense(b: &[u8]) -> Sense {
    let at = |i: usize| b.get(i).copied().unwrap_or(0);
    match at(0) & 0x7f {
        0x72 | 0x73 => Sense {
            key: at(1) & 0xf,
            asc: at(2),
            ascq: at(3),
        },
        _ => Sense {
            key: at(2) & 0xf,
            asc: at(12),
            ascq: at(13),
        },
    }
}

/// READ CAPACITY(10) result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capacity {
    /// Last addressable LBA.
    pub last_lba: u32,
    /// Block length in bytes.
    pub block_size: u32,
}

impl<M: Mmio, D: Dma, T: Delay> Ufshc<M, D, T> {
    /// Run an allowlisted command on a regular LU; data-in lands in `out`. Returns bytes received.
    pub fn scsi(&mut self, lun: u8, cmd: Cmd, block_size: u32, out: &mut [u8]) -> Result<usize> {
        self.scsi_raw(lun, &cmd.cdb(), cmd.data_len(block_size), out)
    }

    /// Run a raw CDB. Anything whose opcode is not in [`ALLOWED_OPCODES`] is refused before
    /// any register or DMA access.
    pub fn scsi_raw(
        &mut self,
        lun: u8,
        cdb: &[u8],
        data_len: usize,
        out: &mut [u8],
    ) -> Result<usize> {
        let op = *cdb.first().ok_or(Error::InvalidArgument)?;
        if !ALLOWED_OPCODES.contains(&op) {
            return Err(Error::Refused(op));
        }
        if cdb.len() > 16 || lun > 0x7f || data_len > out.len() {
            return Err(Error::InvalidArgument);
        }
        let mut c = [0u8; 16];
        c[..cdb.len()].copy_from_slice(cdb);
        for attempt in 0..2 {
            let tag = self.next_tag();
            let done = self.transfer(&upiu::command(tag, lun, data_len as u32, &c), data_len)?;
            let hdr = upiu::parse_header(&done.resp);
            if hdr.response != 0 {
                return Err(Error::Response(hdr.response));
            }
            match hdr.status {
                0 => {
                    let r = &done.resp;
                    let residual = u32::from_be_bytes([r[12], r[13], r[14], r[15]]) as usize;
                    let got = data_len.saturating_sub(residual);
                    self.copy_data(&mut out[..got]);
                    return Ok(got);
                }
                CHECK_CONDITION => {
                    let sense = parse_sense(upiu::sense(&done.resp));
                    if sense.key == UNIT_ATTENTION && attempt == 0 {
                        continue;
                    }
                    return Err(Error::CheckCondition(sense));
                }
                s => return Err(Error::ScsiStatus(s)),
            }
        }
        unreachable!("loop returns on its second attempt")
    }

    /// READ CAPACITY(10).
    pub fn read_capacity(&mut self, lun: u8) -> Result<Capacity> {
        let mut b = [0u8; 8];
        let got = self.scsi(lun, Cmd::ReadCapacity10, 0, &mut b)?;
        if got != 8 {
            return Err(Error::ShortTransfer(got));
        }
        Ok(Capacity {
            last_lba: u32::from_be_bytes([b[0], b[1], b[2], b[3]]),
            block_size: u32::from_be_bytes([b[4], b[5], b[6], b[7]]),
        })
    }

    /// READ(10) whole blocks starting at `lba` into `buf`, chunked to the DMA buffer.
    pub fn read(&mut self, lun: u8, lba: u64, block_size: u32, buf: &mut [u8]) -> Result<()> {
        let bs = block_size as usize;
        if bs == 0 || !buf.len().is_multiple_of(bs) {
            return Err(Error::InvalidArgument);
        }
        let per = (dma::data_capacity(self.dma.size()) / bs).min(u16::MAX as usize);
        if per == 0 || lba + (buf.len() / bs) as u64 > u32::MAX as u64 + 1 {
            return Err(Error::InvalidArgument);
        }
        let mut lba = lba as u32;
        for chunk in buf.chunks_mut(per * bs) {
            let blocks = (chunk.len() / bs) as u16;
            let got = self.scsi(lun, Cmd::Read10 { lba, blocks }, block_size, chunk)?;
            if got != chunk.len() {
                return Err(Error::ShortTransfer(got));
            }
            lba = lba.wrapping_add(blocks as u32);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cdbs_are_read_only_and_well_formed() {
        let r = Cmd::Read10 {
            lba: 0x0102_0304,
            blocks: 2,
        }
        .cdb();
        assert_eq!(&r[..10], &[0x28, 0, 1, 2, 3, 4, 0, 0, 2, 0]);
        assert_eq!(Cmd::Read10 { lba: 0, blocks: 3 }.data_len(4096), 12288);
        for c in [
            Cmd::TestUnitReady,
            Cmd::RequestSense,
            Cmd::Inquiry,
            Cmd::ReadCapacity10,
            Cmd::Read10 { lba: 0, blocks: 1 },
        ] {
            assert!(ALLOWED_OPCODES.contains(&c.cdb()[0]));
        }
        // No write-class opcode can sneak into the allowlist.
        for op in [
            0x0a, 0x2a, 0x2e, 0x35, 0x41, 0x42, 0x8a, 0x91, 0x93, 0xaa, 0x55, 0x15, 0x1b, 0x3b,
        ] {
            assert!(!ALLOWED_OPCODES.contains(&op), "opcode {op:#x}");
        }
    }

    #[test]
    fn sense_fixed_and_descriptor_formats() {
        let mut f = [0u8; 18];
        f[0] = 0x70;
        f[2] = 0x06;
        f[12] = 0x29;
        assert_eq!(
            parse_sense(&f),
            Sense {
                key: 6,
                asc: 0x29,
                ascq: 0
            }
        );
        assert_eq!(
            parse_sense(&[0x72, 0x05, 0x24, 0x01]),
            Sense {
                key: 5,
                asc: 0x24,
                ascq: 1
            }
        );
        assert_eq!(parse_sense(&[]), Sense::default());
    }
}
