//! Device management: NOP, Query, descriptors, logical units (`specs/ufs.md` §4 steps 9–12).

use crate::hc::{Completion, HostInfo, Ufshc};
use crate::upiu;
use crate::{Delay, Dma, Error, Result, Stage};
use alloc::vec::Vec;
use mmio_replay::Mmio;

/// Descriptor IDN: device.
pub const IDN_DEVICE: u8 = 0x00;
/// Descriptor IDN: unit.
pub const IDN_UNIT: u8 = 0x02;
/// Descriptor IDN: geometry.
pub const IDN_GEOMETRY: u8 = 0x07;
/// Flag IDN: `fDeviceInit`.
pub const FLAG_DEVICE_INIT: u8 = 0x01;
/// Largest descriptor read (the length field is one byte).
const DESC_MAX: u16 = 255;

/// An enabled logical unit, from its unit descriptor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogicalUnit {
    /// LUN (0..32).
    pub lun: u8,
    /// Logical block size in bytes (`2^bLogicalBlockSize`).
    pub block_size: u32,
    /// `qLogicalBlockCount`.
    pub block_count: u64,
    /// `bLUWriteProtect` (0 none, 1 power-on, 2 permanent).
    pub write_protect: u8,
    /// `bBootLunID` (0 = not a boot LU, 1 = A, 2 = B).
    pub boot_lun_id: u8,
}

/// What bring-up learned about the controller and device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceInfo {
    /// Controller identification.
    pub host: HostInfo,
    /// Device descriptor `wSpecVersion` (BCD, e.g. 0x0210).
    pub spec_version: u16,
    /// Device descriptor `wManufacturerID`.
    pub manufacturer_id: u16,
    /// Device descriptor `bNumberLU`.
    pub number_lu: u8,
    /// Geometry `qTotalRawDeviceCapacity` (512-byte units).
    pub raw_capacity: u64,
    /// LUNs scanned (8 or 32, from `bMaxNumberLU`).
    pub max_lu: u8,
    /// Enabled logical units in LUN order.
    pub units: Vec<LogicalUnit>,
}

fn be16(b: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([b[at], b[at + 1]])
}

fn be64(b: &[u8], at: usize) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[at..at + 8]);
    u64::from_be_bytes(a)
}

impl<M: Mmio, D: Dma, T: Delay> Ufshc<M, D, T> {
    /// Full read-only bring-up: controller, link, list, NOP, `fDeviceInit`, LU table.
    pub fn bring_up(&mut self) -> Result<DeviceInfo> {
        self.enable()?;
        self.link_startup()?;
        self.start_list()?;
        self.nop()?;
        self.device_init()?;
        self.enumerate()
    }

    /// NOP OUT → NOP IN; timeouts, OCS and response errors are retried `nop_attempts` times.
    pub fn nop(&mut self) -> Result<()> {
        let mut last = Error::Timeout(Stage::Doorbell);
        for _ in 0..self.cfg.nop_attempts.max(1) {
            let tag = self.next_tag();
            match self.transfer(&upiu::nop_out(tag), 0) {
                Ok(c) => match upiu::parse_header(&c.resp).response {
                    0 => return Ok(()),
                    r => last = Error::Response(r),
                },
                Err(e @ (Error::Ocs(_) | Error::Timeout(Stage::Doorbell))) => last = e,
                Err(e) => return Err(e),
            }
        }
        Err(last)
    }

    fn query(
        &mut self,
        function: u8,
        opcode: u8,
        idn: u8,
        index: u8,
        len: u16,
    ) -> Result<Completion> {
        let tag = self.next_tag();
        let c = self.transfer(&upiu::query(tag, function, opcode, idn, index, len), 0)?;
        match upiu::parse_header(&c.resp).response {
            0 => Ok(c),
            r => Err(Error::Response(r)),
        }
    }

    /// READ DESCRIPTOR; returns `bLength` bytes after checking the IDN.
    pub fn read_descriptor(&mut self, idn: u8, index: u8) -> Result<Vec<u8>> {
        let c = self.query(
            upiu::QUERY_FN_READ,
            upiu::QOP_READ_DESC,
            idn,
            index,
            DESC_MAX,
        )?;
        let mut d = c.resp[upiu::RESP_HDR_LEN..].to_vec();
        if d.len() < 2 || d[1] != idn || (d[0] as usize) < 2 {
            return Err(Error::Descriptor(idn));
        }
        d.truncate(d[0] as usize);
        Ok(d)
    }

    /// READ FLAG.
    pub fn read_flag(&mut self, idn: u8) -> Result<bool> {
        let c = self.query(upiu::QUERY_FN_READ, upiu::QOP_READ_FLAG, idn, 0, 0)?;
        Ok(upiu::query_value(&c.resp) & 1 != 0)
    }

    /// READ ATTRIBUTE (32-bit value).
    pub fn read_attribute(&mut self, idn: u8, index: u8) -> Result<u32> {
        let c = self.query(upiu::QUERY_FN_READ, upiu::QOP_READ_ATTR, idn, index, 0)?;
        Ok(upiu::query_value(&c.resp))
    }

    /// SET FLAG `fDeviceInit`, then poll until the device clears it. The only Query write.
    pub fn device_init(&mut self) -> Result<()> {
        self.query(
            upiu::QUERY_FN_WRITE,
            upiu::QOP_SET_FLAG,
            FLAG_DEVICE_INIT,
            0,
            0,
        )?;
        for _ in 0..self.cfg.polls.device_init {
            if !self.read_flag(FLAG_DEVICE_INIT)? {
                return Ok(());
            }
        }
        Err(Error::Timeout(Stage::DeviceInit))
    }

    /// Device + geometry descriptors, then one unit descriptor per possible LUN.
    pub fn enumerate(&mut self) -> Result<DeviceInfo> {
        let dev = self.read_descriptor(IDN_DEVICE, 0)?;
        if dev.len() < 0x1a {
            return Err(Error::Descriptor(IDN_DEVICE));
        }
        let geo = self.read_descriptor(IDN_GEOMETRY, 0)?;
        if geo.len() < 0x0d {
            return Err(Error::Descriptor(IDN_GEOMETRY));
        }
        let max_lu = if geo[0x0c] == 1 { 32 } else { 8 };
        let mut units = Vec::new();
        for lun in 0..max_lu {
            let u = self.read_descriptor(IDN_UNIT, lun)?;
            if u.len() < 0x13 {
                return Err(Error::Descriptor(IDN_UNIT));
            }
            if u[0x03] != 1 {
                continue;
            }
            if !(9..=16).contains(&u[0x0a]) {
                return Err(Error::Descriptor(IDN_UNIT));
            }
            units.push(LogicalUnit {
                lun,
                block_size: 1 << u[0x0a],
                block_count: be64(&u, 0x0b),
                write_protect: u[0x05],
                boot_lun_id: u[0x04],
            });
        }
        Ok(DeviceInfo {
            host: self.info,
            spec_version: be16(&dev, 0x10),
            manufacturer_id: be16(&dev, 0x18),
            number_lu: dev[0x06],
            raw_capacity: be64(&geo, 0x04),
            max_lu,
            units,
        })
    }
}
