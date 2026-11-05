//! The UFS *device* side: decodes one UTRD + UCD from DMA, answers NOP/QUERY/COMMAND UPIUs,
//! writes the response UPIU, data-in through the PRDT and the OCS. Also records every request so
//! tests can prove no write ever left the host.

use super::luns::Lu;
use super::SharedDma;

/// Injected device faults (each consumed when it fires, unless noted).
#[derive(Clone, Default)]
pub struct Faults {
    /// OCS for the next N NOP OUTs.
    pub nop_ocs: Vec<u8>,
    /// OCS for the next COMMAND UPIU.
    pub command_ocs: Option<u8>,
    /// Report UNIT ATTENTION on the first command to each LU.
    pub unit_attention: bool,
    /// CHECK CONDITION (key, asc) on every READ(10).
    pub read_error: Option<(u8, u8)>,
    /// Residual count reported on the next data-in command.
    pub residual: Option<u32>,
    /// Number of READ FLAG fDeviceInit polls that still return 1 (`u32::MAX` = forever).
    pub device_init_busy: u32,
    /// Echo a wrong task tag in the next response.
    pub bad_tag: bool,
    /// Return this IDN in descriptor responses instead of the requested one.
    pub wrong_idn: Option<u8>,
}

/// Every request as seen by the device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Seen {
    Nop,
    Query {
        function: u8,
        opcode: u8,
        idn: u8,
        index: u8,
    },
    Command {
        lun: u8,
        opcode: u8,
        flags: u8,
        dir: u8,
    },
}

pub struct UfsDevice {
    pub luns: Vec<Lu>,
    pub max_lu_code: u8,
    pub faults: Faults,
    pub seen: Vec<Seen>,
    pub device_init: bool,
    pub attention_done: Vec<bool>,
}

pub fn be(b: &[u8]) -> u64 {
    b.iter().fold(0, |a, &x| a << 8 | x as u64)
}

fn le(b: &[u8]) -> u64 {
    b.iter().rev().fold(0, |a, &x| a << 8 | x as u64)
}

impl UfsDevice {
    pub fn new(luns: Vec<Lu>) -> Self {
        let n = luns.len();
        UfsDevice {
            luns,
            max_lu_code: 0,
            faults: Faults::default(),
            seen: Vec::new(),
            device_init: false,
            attention_done: vec![false; n],
        }
    }

    /// Process the UTRD at bus address `utrd`. Mirrors what the controller + device do on a doorbell.
    pub fn process(&mut self, mem: &SharedDma, utrd: u64) {
        let d = mem.get(utrd, 32);
        let dw0 = le(&d[0..4]) as u32;
        let ucd = le(&d[16..24]);
        let resp_len = le(&d[24..26]) as usize * 4;
        let resp_off = le(&d[26..28]) as usize * 4;
        let prdt_n = le(&d[28..30]) as usize;
        let prdt_off = le(&d[30..32]) as usize * 4;
        let dir = ((dw0 >> 25) & 3) as u8;
        let ocs = if dw0 >> 28 != 1 || !ucd.is_multiple_of(128) || resp_len < 32 {
            0x01
        } else {
            let req = mem.get(ucd, 32);
            let mut resp = vec![0u8; 512];
            let mut data = Vec::new();
            let ocs = self.respond(&req, dir, &mut resp, &mut data);
            let ocs = if ocs == 0 {
                self.data_in(mem, ucd, prdt_off, prdt_n, &data)
            } else {
                ocs
            };
            mem.put(ucd + resp_off as u64, &resp[..resp_len.min(512)]);
            ocs
        };
        mem.put(utrd + 8, &[ocs]);
    }

    fn data_in(&self, mem: &SharedDma, ucd: u64, prdt_off: usize, n: usize, data: &[u8]) -> u8 {
        let mut done = 0;
        for i in 0..n {
            let e = mem.get(ucd + (prdt_off + 16 * i) as u64, 16);
            let (addr, dbc) = (le(&e[0..8]), le(&e[12..16]) as usize + 1);
            if dbc % 4 != 0 {
                return 0x02;
            }
            let take = dbc.min(data.len() - done);
            mem.put(addr, &data[done..done + take]);
            done += take;
        }
        if done < data.len() {
            0x03
        } else {
            0
        }
    }

    fn respond(&mut self, req: &[u8], dir: u8, resp: &mut [u8], data: &mut Vec<u8>) -> u8 {
        let tag = if std::mem::take(&mut self.faults.bad_tag) {
            req[3] ^ 0xff
        } else {
            req[3]
        };
        resp[3] = tag;
        match req[0] & 0x3f {
            0x00 => {
                self.seen.push(Seen::Nop);
                if !self.faults.nop_ocs.is_empty() {
                    return self.faults.nop_ocs.remove(0);
                }
                resp[0] = 0x20;
                0
            }
            0x16 => {
                self.query(req, resp);
                0
            }
            0x01 => self.command(req, dir, resp, data),
            _ => 0x01,
        }
    }

    fn query(&mut self, req: &[u8], resp: &mut [u8]) {
        let (function, opcode, idn, index) = (req[5], req[12], req[13], req[14]);
        self.seen.push(Seen::Query {
            function,
            opcode,
            idn,
            index,
        });
        resp[0] = 0x36;
        resp[5] = function;
        resp[12..16].copy_from_slice(&req[12..16]);
        let code = match (function, opcode, idn) {
            (0x01, 0x01, _) => match self.descriptor(idn, index) {
                Some(mut d) => {
                    if let Some(w) = self.faults.wrong_idn {
                        d[1] = w;
                    }
                    let n = d.len().min(be(&req[18..20]) as usize);
                    resp[10..12].copy_from_slice(&(n as u16).to_be_bytes());
                    resp[18..20].copy_from_slice(&(n as u16).to_be_bytes());
                    resp[32..32 + n].copy_from_slice(&d[..n]);
                    0
                }
                None if idn == 0x02 => 0xfc,
                None => 0xfd,
            },
            (0x01, 0x05, 0x01) => {
                let busy = self.device_init && self.faults.device_init_busy > 0;
                if busy && self.faults.device_init_busy != u32::MAX {
                    self.faults.device_init_busy -= 1;
                }
                resp[23] = busy as u8;
                0
            }
            (0x01, 0x03, 0x00) => {
                resp[20..24].copy_from_slice(&1u32.to_be_bytes());
                0
            }
            (0x81, 0x06, 0x01) => {
                self.device_init = true;
                0
            }
            (0x81, _, _) => 0xf7,
            _ => 0xfe,
        };
        resp[6] = code;
    }

    fn command(&mut self, req: &[u8], dir: u8, resp: &mut [u8], data: &mut Vec<u8>) -> u8 {
        let (lun, flags, exp) = (req[2], req[1], be(&req[12..16]) as usize);
        let cdb = &req[16..32];
        self.seen.push(Seen::Command {
            lun,
            opcode: cdb[0],
            flags,
            dir,
        });
        if let Some(o) = self.faults.command_ocs.take() {
            return o;
        }
        resp[0] = 0x21;
        let status = match self.scsi(lun, cdb, data) {
            Ok(()) => 0,
            Err((key, asc)) => {
                resp[32..34].copy_from_slice(&18u16.to_be_bytes());
                resp[34] = 0x70;
                resp[36] = key;
                resp[46] = asc;
                resp[10..12].copy_from_slice(&20u16.to_be_bytes());
                data.clear();
                2
            }
        };
        resp[7] = status;
        if data.len() > exp {
            data.truncate(exp);
        }
        let residual = self.faults.residual.take().unwrap_or(0) as usize + (exp - data.len());
        data.truncate(exp - residual.min(exp));
        if status == 0 {
            resp[12..16].copy_from_slice(&(residual as u32).to_be_bytes());
        }
        if !data.is_empty() && dir != 2 {
            0x02
        } else {
            0
        }
    }

    /// True when a host→device transfer, a W flag or a write-class Query was ever seen.
    pub fn saw_write(&self) -> bool {
        self.seen.iter().any(|s| match *s {
            Seen::Command { flags, dir, .. } => flags & 0x20 != 0 || dir == 1,
            Seen::Query {
                function: 0x81,
                opcode,
                idn,
                ..
            } => !(opcode == 0x06 && idn == 0x01),
            _ => false,
        })
    }
}
