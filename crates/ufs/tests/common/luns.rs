//! Logical units and the device's descriptor + SCSI answers.

use super::device::{be, UfsDevice};

/// One logical unit.
#[derive(Clone)]
pub struct Lu {
    pub enabled: bool,
    pub block_shift: u8,
    pub boot_id: u8,
    pub write_protect: u8,
    pub data: Vec<u8>,
}

impl Lu {
    pub fn new(block_shift: u8, data: Vec<u8>) -> Self {
        Lu {
            enabled: true,
            block_shift,
            boot_id: 0,
            write_protect: 0,
            data,
        }
    }
    pub fn disabled() -> Self {
        Lu {
            enabled: false,
            block_shift: 12,
            boot_id: 0,
            write_protect: 0,
            data: Vec::new(),
        }
    }
    pub fn bs(&self) -> usize {
        1 << self.block_shift
    }
    pub fn blocks(&self) -> u64 {
        (self.data.len() / self.bs()) as u64
    }
}

impl UfsDevice {
    pub fn descriptor(&self, idn: u8, index: u8) -> Option<Vec<u8>> {
        match idn {
            0x00 => {
                let mut d = vec![0u8; 0x40];
                d[0] = 0x40;
                d[6] = self.luns.iter().filter(|l| l.enabled).count() as u8;
                d[0x10..0x12].copy_from_slice(&0x0210u16.to_be_bytes());
                d[0x18..0x1a].copy_from_slice(&0x01ceu16.to_be_bytes());
                Some(d)
            }
            0x07 => {
                let mut d = vec![0u8; 0x48];
                d[0] = 0x48;
                d[1] = 0x07;
                d[4..12].copy_from_slice(&0x0773_0000u64.to_be_bytes());
                d[0x0c] = self.max_lu_code;
                Some(d)
            }
            0x02 => {
                let max = if self.max_lu_code == 1 { 32 } else { 8 };
                if index >= max {
                    return None;
                }
                let lu = self
                    .luns
                    .get(index as usize)
                    .cloned()
                    .unwrap_or_else(Lu::disabled);
                let mut d = vec![0u8; 0x2d];
                d[0] = 0x2d;
                d[1] = 0x02;
                d[2] = index;
                d[3] = lu.enabled as u8;
                d[4] = lu.boot_id;
                d[5] = lu.write_protect;
                d[0x0a] = lu.block_shift;
                d[0x0b..0x13].copy_from_slice(&lu.blocks().to_be_bytes());
                Some(d)
            }
            _ => None,
        }
    }

    pub fn scsi(&mut self, lun: u8, cdb: &[u8], data: &mut Vec<u8>) -> Result<(), (u8, u8)> {
        let lu = match self.luns.get(lun as usize) {
            Some(lu) if lu.enabled => lu.clone(),
            _ => return Err((0x05, 0x25)),
        };
        if self.faults.unit_attention && !self.attention_done[lun as usize] {
            self.attention_done[lun as usize] = true;
            return Err((0x06, 0x29));
        }
        match cdb[0] {
            0x00 => Ok(()),
            0x03 => {
                data.extend_from_slice(&[0x70, 0, 0, 0, 0, 0, 0, 10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
                Ok(())
            }
            0x12 => {
                let mut inq = vec![0u8; 36];
                inq[8..16].copy_from_slice(b"SAMSUNG ");
                data.extend_from_slice(&inq);
                Ok(())
            }
            0x25 => {
                data.extend_from_slice(&((lu.blocks() - 1) as u32).to_be_bytes());
                data.extend_from_slice(&(lu.bs() as u32).to_be_bytes());
                Ok(())
            }
            0x28 => {
                if let Some(e) = self.faults.read_error {
                    return Err(e);
                }
                let (lba, n) = (be(&cdb[2..6]), be(&cdb[7..9]));
                if lba + n > lu.blocks() {
                    return Err((0x05, 0x21));
                }
                let (a, b) = (lba as usize * lu.bs(), (lba + n) as usize * lu.bs());
                data.extend_from_slice(&lu.data[a..b]);
                Ok(())
            }
            _ => Err((0x05, 0x20)),
        }
    }
}
