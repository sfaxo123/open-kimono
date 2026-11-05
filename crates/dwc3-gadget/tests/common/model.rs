//! Scripted DWC3 controller model: register semantics, endpoint commands,
//! TRB processing and event-ring writes, driven by host-side calls.
//! Protocol violations by the driver are collected in `Hw::violations`.

use dwc3_gadget::event::{DevEvent, EpEvent, Event};
use dwc3_gadget::layout::DMA_SIZE;
use dwc3_gadget::trb::{Trb, TrbCtl, HWO, SPR};
use mmio_replay::{Mmio, Width};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

pub const CORE: u64 = 0x0a60_0000;
pub const QS: u64 = 0x0a6f_8800;
pub const DMA: u64 = 0x8000_0000;

#[derive(Clone, Copy, Default, Debug)]
pub struct HwEp {
    pub armed: Option<u64>,
    pub progress: u32,
    pub rsrc: u8,
    pub stalled: bool,
    pub configured: bool,
    pub mps: u16,
    pub nrdy_posted: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum HostErr {
    /// Endpoint is halted (STALL handshake).
    Stall,
    /// Device did not get ready within the retry budget.
    Timeout,
}

pub struct Hw {
    pub regs: BTreeMap<u64, u32>,
    pub dma: Vec<u8>,
    pub eps: [HwEp; 32],
    pub running: bool,
    pub ev_wr: u32,
    pub ev_count: u32,
    pub next_rsrc: u8,
    /// `(phys, cmd, param)` of every endpoint command.
    pub cmds: Vec<(u8, u32, u32)>,
    /// DCFG.DEVADDR seen when each status TRB was started.
    pub addr_at_status: Vec<u32>,
    pub violations: Vec<String>,
    /// Physical endpoint whose next command never clears CMDACT / fails.
    pub stuck_cmd: Option<u8>,
    pub fail_cmd: Option<u8>,
    pub speed: u32,
    /// Last SETUP had a data stage (status must be STATUS_3, else STATUS_2).
    pub three_stage: bool,
}

pub type Shared = Rc<RefCell<Hw>>;

impl Hw {
    pub fn new() -> Shared {
        let mut regs = BTreeMap::new();
        regs.insert(CORE + 0xc120, 0x5533_310a);
        regs.insert(CORE + 0xc14c, 0x0021_0000);
        Rc::new(RefCell::new(Hw {
            regs,
            dma: vec![0; DMA_SIZE],
            eps: [HwEp::default(); 32],
            running: false,
            ev_wr: 0,
            ev_count: 0,
            next_rsrc: 1,
            cmds: Vec::new(),
            addr_at_status: Vec::new(),
            violations: Vec::new(),
            stuck_cmd: None,
            fail_cmd: None,
            speed: 0,
            three_stage: false,
        }))
    }

    pub fn reg(&self, off: u64) -> u32 {
        *self.regs.get(&(CORE + off)).unwrap_or(&0)
    }

    pub fn qreg(&self, off: u64) -> u32 {
        *self.regs.get(&(QS + off)).unwrap_or(&0)
    }

    fn bad(&mut self, msg: String) {
        self.violations.push(msg);
    }

    fn d32(&self, a: u64) -> u32 {
        let i = (a - DMA) as usize;
        u32::from_le_bytes(self.dma[i..i + 4].try_into().unwrap())
    }

    fn put32(&mut self, a: u64, v: u32) {
        let i = (a - DMA) as usize;
        self.dma[i..i + 4].copy_from_slice(&v.to_le_bytes());
    }

    fn trb(&self, a: u64) -> Trb {
        Trb { ptr: self.d32(a) as u64 | ((self.d32(a + 4) as u64) << 32), status: self.d32(a + 8), control: self.d32(a + 12) }
    }

    /// Append one event word to the ring (only while running, as hardware).
    pub fn post(&mut self, e: Event) {
        if !self.running {
            return;
        }
        let size = self.reg(0xc408) & 0xffff;
        let base = self.reg(0xc400) as u64 | ((self.reg(0xc404) as u64) << 32);
        if size == 0 || self.ev_count + 4 > size {
            self.bad(format!("event ring overflow posting {e:?}"));
            return;
        }
        self.put32(base + self.ev_wr as u64, e.encode());
        self.ev_wr = (self.ev_wr + 4) % size;
        self.ev_count += 4;
    }

    fn command(&mut self, phys: u8, v: u32) {
        let b = CORE + 0xc800 + 0x10 * phys as u64;
        let par = [self.regs.get(&(b + 8)).copied().unwrap_or(0), self.regs.get(&(b + 4)).copied().unwrap_or(0)];
        let (ty, param) = (v & 0xf, v >> 16);
        self.cmds.push((phys, ty, param));
        let mut out = ty;
        if self.stuck_cmd == Some(phys) {
            self.regs.insert(b + 0xc, v);
            return;
        }
        if self.fail_cmd.take() == Some(phys) {
            self.regs.insert(b + 0xc, ty | (1 << 12));
            return;
        }
        let ep = &mut self.eps[phys as usize];
        match ty {
            1 => {
                ep.configured = true;
                ep.mps = ((par[0] >> 3) & 0x7ff) as u16;
                if (par[1] >> 25) & 0x1f != phys as u32 {
                    self.bad(format!("DEPCFG ep number mismatch on phys {phys}"));
                }
            }
            6 => {
                let a = ((par[0] as u64) << 32) | par[1] as u64;
                let t = self.trb(a);
                let ep = &mut self.eps[phys as usize];
                if !ep.configured || ep.armed.is_some() || !t.hw_owned() {
                    let msg = format!("bad START_TRANSFER on phys {phys}: {ep:?} {t:?}");
                    self.bad(msg);
                }
                let ep = &mut self.eps[phys as usize];
                ep.armed = Some(a);
                ep.progress = 0;
                ep.nrdy_posted = false;
                ep.rsrc = self.next_rsrc;
                out |= (self.next_rsrc as u32) << 16;
                self.next_rsrc = (self.next_rsrc % 127) + 1;
                if matches!(TrbCtl::from_control(t.control), Some(TrbCtl::Status2 | TrbCtl::Status3)) {
                    let addr = (self.reg(0xc700) >> 3) & 0x7f;
                    self.addr_at_status.push(addr);
                }
            }
            8 => {
                if ep.armed.is_none() || ep.rsrc as u32 != param {
                    let msg = format!("END_TRANSFER phys {phys} rsrc {param}: {ep:?}");
                    self.bad(msg);
                }
                self.eps[phys as usize].armed = None;
            }
            4 => ep.stalled = true,
            5 => ep.stalled = false,
            2 | 9 => {}
            _ => self.bad(format!("unknown command {ty} on phys {phys}")),
        }
        self.regs.insert(b + 0xc, out);
    }

    fn write_reg(&mut self, a: u64, v: u32) {
        match a.wrapping_sub(CORE) {
            0xc704 => {
                if v & (1 << 30) != 0 {
                    self.eps = [HwEp::default(); 32];
                }
                self.running = v & (1 << 31) != 0;
                self.regs.insert(a, v & !(1 << 30));
            }
            0xc40c => {
                let n = v & 0xffff;
                if n > self.ev_count {
                    self.bad(format!("GEVNTCOUNT over-ack {n} > {}", self.ev_count));
                }
                self.ev_count = self.ev_count.saturating_sub(n);
            }
            off @ 0xc800..=0xc9ff if off & 0xf == 0xc => {
                self.regs.insert(a, v);
                if v & (1 << 10) != 0 {
                    self.command(((off - 0xc800) / 0x10) as u8, v);
                }
            }
            _ => {
                self.regs.insert(a, v);
            }
        }
    }

    fn read_reg(&mut self, a: u64) -> u32 {
        match a.wrapping_sub(CORE) {
            0xc70c => (if self.running { 0 } else { 1 << 22 }) | self.speed,
            0xc40c => self.ev_count,
            _ => *self.regs.get(&a).unwrap_or(&0),
        }
    }

    fn finish_trb(&mut self, phys: u8, a: u64, remaining: u32, short: bool) {
        let t = self.trb(a);
        self.put32(a + 8, (t.status & !0x00ff_ffff) | remaining | if short { SPR } else { 0 });
        self.put32(a + 12, t.control & !HWO);
        self.eps[phys as usize].armed = None;
        self.post(Event::Ep { phys, kind: EpEvent::XferComplete });
    }

    fn not_ready(&mut self, phys: u8, stage: u8) {
        if !self.eps[phys as usize].nrdy_posted {
            self.eps[phys as usize].nrdy_posted = true;
            self.post(Event::Ep { phys, kind: EpEvent::XferNotReady { stage, active: false } });
        }
    }

    fn halted(&self, phys: u8) -> bool {
        self.eps[phys as usize].stalled || (phys < 2 && self.eps[0].stalled)
    }

    // ---- host side --------------------------------------------------------

    pub fn bus_reset(&mut self) {
        self.eps[0].stalled = false;
        self.eps[1].stalled = false;
        self.post(Event::Dev(DevEvent::UsbReset));
        self.post(Event::Dev(DevEvent::ConnectDone));
    }

    pub fn dev_event(&mut self, e: DevEvent) {
        self.post(Event::Dev(e));
    }

    /// Deliver a SETUP packet; `false` if no SETUP TRB is armed.
    pub fn setup(&mut self, pkt: [u8; 8]) -> bool {
        let Some(a) = self.eps[0].armed else { return false };
        let t = self.trb(a);
        if TrbCtl::from_control(t.control) != Some(TrbCtl::Setup) {
            return false;
        }
        for (i, b) in pkt.iter().enumerate() {
            self.dma[(t.ptr - DMA) as usize + i] = *b;
        }
        self.three_stage = pkt[6] != 0 || pkt[7] != 0;
        self.eps[0].stalled = false;
        self.eps[1].stalled = false;
        self.finish_trb(0, a, t.remaining() - 8, false);
        true
    }

    /// One IN token: `Ok(Some(packet))`, `Ok(None)` = NAK.
    pub fn in_token(&mut self, phys: u8, stage: u8) -> Result<Option<Vec<u8>>, HostErr> {
        if self.halted(phys) {
            return Err(HostErr::Stall);
        }
        let Some(a) = self.eps[phys as usize].armed else {
            self.not_ready(phys, stage);
            return Ok(None);
        };
        let t = self.trb(a);
        self.check_ctl(phys, stage, t);
        let mps = self.eps[phys as usize].mps as u32;
        let len = t.remaining();
        let p = self.eps[phys as usize].progress;
        let n = (len - p).min(mps);
        let start = (t.ptr - DMA) as usize + p as usize;
        let pkt = self.dma[start..start + n as usize].to_vec();
        self.eps[phys as usize].progress += n;
        if p + n >= len {
            self.finish_trb(phys, a, 0, false);
        }
        Ok(Some(pkt))
    }

    /// One OUT packet: `Ok(true)` accepted, `Ok(false)` = NAK.
    pub fn out_packet(&mut self, phys: u8, stage: u8, data: &[u8]) -> Result<bool, HostErr> {
        if self.halted(phys) {
            return Err(HostErr::Stall);
        }
        let Some(a) = self.eps[phys as usize].armed else {
            self.not_ready(phys, stage);
            return Ok(false);
        };
        let t = self.trb(a);
        self.check_ctl(phys, stage, t);
        let p = self.eps[phys as usize].progress;
        if p + data.len() as u32 > t.remaining() {
            self.bad(format!("babble: {} bytes into {}-byte TRB on phys {phys}", data.len(), t.remaining() - p));
            return Ok(false);
        }
        let start = (t.ptr - DMA) as usize + p as usize;
        self.dma[start..start + data.len()].copy_from_slice(data);
        self.eps[phys as usize].progress += data.len() as u32;
        let rem = t.remaining() - p - data.len() as u32;
        let short = data.len() < self.eps[phys as usize].mps as usize;
        if short || rem == 0 {
            self.finish_trb(phys, a, rem, short && rem != 0);
        }
        Ok(true)
    }

    fn check_ctl(&mut self, phys: u8, stage: u8, t: Trb) {
        let want = match (phys, stage) {
            (0..=1, 1) => Some(TrbCtl::ControlData),
            (0..=1, _) => None,
            _ => Some(TrbCtl::Normal),
        };
        let got = TrbCtl::from_control(t.control);
        let status = if self.three_stage { TrbCtl::Status3 } else { TrbCtl::Status2 };
        let ok = match want {
            Some(w) => got == Some(w),
            None => got == Some(status),
        };
        if !ok {
            self.bad(format!("phys {phys} stage {stage}: unexpected TRB {got:?}"));
        }
    }
}

/// Register window view of the model.
pub struct Regs(pub Shared);
/// DMA window view of the model.
pub struct Dma(pub Shared);

impl Mmio for Regs {
    fn read(&mut self, addr: u64, w: Width) -> u64 {
        assert_eq!(w, Width::W32, "register access width");
        self.0.borrow_mut().read_reg(addr) as u64
    }
    fn write(&mut self, addr: u64, w: Width, v: u64) {
        assert_eq!(w, Width::W32, "register access width");
        self.0.borrow_mut().write_reg(addr, v as u32);
    }
}

impl Mmio for Dma {
    fn read(&mut self, addr: u64, w: Width) -> u64 {
        let h = self.0.borrow();
        let i = (addr - DMA) as usize;
        (0..w.bytes()).fold(0, |v, k| v | (h.dma[i + k] as u64) << (8 * k))
    }
    fn write(&mut self, addr: u64, w: Width, v: u64) {
        let mut h = self.0.borrow_mut();
        let i = (addr - DMA) as usize;
        for k in 0..w.bytes() {
            h.dma[i + k] = (v >> (8 * k)) as u8;
        }
    }
}
