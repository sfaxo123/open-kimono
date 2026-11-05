//! Test bench: a DWC3 model, the driver on top of it, device tasks polled
//! cooperatively, and USB-host helpers (control/bulk transfers with retries).
#![allow(dead_code)]

pub mod device;
pub mod model;

use dwc3_gadget::{Config, Dwc3};
use model::{Dma, HostErr, Hw, Regs, Shared, DMA};
use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll, Waker};

pub type Core = Dwc3<Regs, Dma>;
pub const DATA: u8 = 1;
pub const STATUS: u8 = 2;
const RETRIES: usize = 64;

pub fn leak<T>(v: T) -> &'static mut T {
    Box::leak(Box::new(v))
}

type Task = Pin<Box<dyn Future<Output = ()>>>;

pub struct Bench {
    pub hw: Shared,
    pub core: &'static RefCell<Core>,
    tasks: Vec<Option<Task>>,
}

impl Bench {
    /// Model + initialised core (not yet connected).
    pub fn new() -> Self {
        Self::with_hw(Hw::new())
    }

    pub fn with_hw(hw: Shared) -> Self {
        let cfg = Config { poll_limit: 16, ..Config::sdm845_usb1(DMA, |_| {}) };
        let core = leak(RefCell::new(Dwc3::new(Regs(hw.clone()), Dma(hw.clone()), cfg)));
        core.borrow_mut().init().expect("init");
        Self { hw, core, tasks: Vec::new() }
    }

    pub fn spawn(&mut self, f: impl Future<Output = ()> + 'static) {
        self.tasks.push(Some(Box::pin(f)));
    }

    /// Poll every device task a few rounds.
    pub fn pump(&mut self) {
        let mut cx = Context::from_waker(Waker::noop());
        for _ in 0..4 {
            for t in self.tasks.iter_mut() {
                if let Some(f) = t {
                    if f.as_mut().poll(&mut cx) == Poll::Ready(()) {
                        *t = None;
                    }
                }
            }
        }
    }

    /// Pump, then try a host operation until it is not NAKed.
    pub fn retry<T>(&mut self, mut f: impl FnMut(&mut Hw) -> Result<Option<T>, HostErr>) -> Result<T, HostErr> {
        for _ in 0..RETRIES {
            self.pump();
            if let Some(v) = f(&mut self.hw.borrow_mut())? {
                return Ok(v);
            }
        }
        Err(HostErr::Timeout)
    }

    pub fn bus_reset(&mut self) {
        self.hw.borrow_mut().bus_reset();
        self.pump();
    }

    fn send_setup(&mut self, pkt: [u8; 8]) -> Result<(), HostErr> {
        self.retry(|h| Ok(h.setup(pkt).then_some(())))
    }

    /// Control read: SETUP, IN data until short/complete, OUT status.
    pub fn control_in(&mut self, pkt: [u8; 8]) -> Result<Vec<u8>, HostErr> {
        let wlen = u16::from_le_bytes([pkt[6], pkt[7]]) as usize;
        self.send_setup(pkt)?;
        let mut data = Vec::new();
        loop {
            let p = self.retry(|h| h.in_token(1, DATA))?;
            let short = p.len() < 64;
            data.extend(p);
            if short || data.len() >= wlen {
                break;
            }
        }
        self.retry(|h| Ok(h.out_packet(0, STATUS, &[])?.then_some(())))?;
        Ok(data)
    }

    /// Control write: SETUP, OUT data packets, IN status.
    pub fn control_out(&mut self, pkt: [u8; 8], data: &[u8]) -> Result<(), HostErr> {
        self.send_setup(pkt)?;
        for c in data.chunks(64) {
            self.retry(|h| Ok(h.out_packet(0, DATA, c)?.then_some(())))?;
        }
        let z = self.retry(|h| h.in_token(1, STATUS))?;
        assert!(z.is_empty(), "status stage carried data");
        Ok(())
    }

    /// Bulk OUT transfer in `mps` packets (+ZLP when `zlp`).
    pub fn bulk_out(&mut self, phys: u8, data: &[u8], mps: usize, zlp: bool) -> Result<(), HostErr> {
        for c in data.chunks(mps) {
            self.retry(|h| Ok(h.out_packet(phys, DATA, c)?.then_some(())))?;
        }
        if zlp && data.len().is_multiple_of(mps) {
            self.retry(|h| Ok(h.out_packet(phys, DATA, &[])?.then_some(())))?;
        }
        Ok(())
    }

    /// One IN packet (interrupt endpoints).
    pub fn in_packet(&mut self, phys: u8) -> Result<Vec<u8>, HostErr> {
        self.retry(|h| h.in_token(phys, DATA))
    }

    /// Bulk IN until a short packet.
    pub fn bulk_in(&mut self, phys: u8, mps: usize) -> Result<Vec<u8>, HostErr> {
        let mut data = Vec::new();
        loop {
            let p = self.retry(|h| h.in_token(phys, DATA))?;
            let short = p.len() < mps;
            data.extend(p);
            if short {
                return Ok(data);
            }
        }
    }

    /// No model violations and no driver fault.
    pub fn assert_clean(&self) {
        let v = &self.hw.borrow().violations;
        assert!(v.is_empty(), "model violations: {v:#?}");
        assert_eq!(self.core.borrow().fault(), None);
    }
}

/// Standard request helpers.
pub fn req(bm: u8, b: u8, value: u16, index: u16, len: u16) -> [u8; 8] {
    let (v, i, l) = (value.to_le_bytes(), index.to_le_bytes(), len.to_le_bytes());
    [bm, b, v[0], v[1], i[0], i[1], l[0], l[1]]
}

pub fn get_descriptor(ty: u8, idx: u8, len: u16) -> [u8; 8] {
    req(0x80, 6, ((ty as u16) << 8) | idx as u16, 0, len)
}
