//! `embassy-usb-driver` implementation over a shared, polled [`Dwc3`].
//!
//! Every future drains the event ring when polled and re-wakes itself while
//! pending (polled mode: no interrupt yet, spec §5).

use crate::controller::{BusEvent, Dwc3};
use crate::ep::{phys, Xfer};
use crate::ep0::Begin;
use core::cell::RefCell;
use core::future::poll_fn;
use core::task::{Context, Poll};
use embassy_usb_driver as usb;
use embassy_usb_driver::{Direction, EndpointAddress, EndpointAllocError, EndpointError, EndpointInfo, EndpointType};
use mmio_replay::Mmio;

type Shared<'d, R, D> = &'d RefCell<Dwc3<R, D>>;

/// Run `f` against the drained core until it yields a value.
async fn wait<R: Mmio, D: Mmio, T>(core: Shared<'_, R, D>, mut f: impl FnMut(&mut Dwc3<R, D>) -> Option<T>) -> T {
    poll_fn(|cx: &mut Context<'_>| {
        let mut c = core.borrow_mut();
        c.poll();
        match f(&mut c) {
            Some(v) => Poll::Ready(v),
            None => {
                cx.waker().wake_by_ref();
                Poll::Pending
            }
        }
    })
    .await
}

/// The driver handed to `embassy_usb::Builder`. Call [`Dwc3::init`] first.
pub struct Driver<'d, R: Mmio, D: Mmio> {
    core: Shared<'d, R, D>,
}

impl<'d, R: Mmio, D: Mmio> Driver<'d, R, D> {
    pub fn new(core: Shared<'d, R, D>) -> Self {
        Self { core }
    }

    fn alloc(&mut self, dir: Direction, t: EndpointType, a: Option<EndpointAddress>, mps: u16, iv: u8) -> Result<Endpoint<'d, R, D>, EndpointAllocError> {
        if a.is_some_and(|a| a.direction() != dir) {
            return Err(EndpointAllocError);
        }
        let is_in = dir == Direction::In;
        let index = a.map(|a| a.index() as u8);
        let i = self.core.borrow_mut().alloc_ep(is_in, t as u8, index, mps, iv).ok_or(EndpointAllocError)?;
        let info = EndpointInfo { addr: EndpointAddress::from_parts(i as usize, dir), ep_type: t, max_packet_size: mps, interval_ms: iv };
        Ok(Endpoint { core: self.core, phys: phys(i, is_in), info })
    }
}

impl<'d, R: Mmio + 'd, D: Mmio + 'd> usb::Driver<'d> for Driver<'d, R, D> {
    type EndpointOut = Endpoint<'d, R, D>;
    type EndpointIn = Endpoint<'d, R, D>;
    type ControlPipe = ControlPipe<'d, R, D>;
    type Bus = Bus<'d, R, D>;

    fn alloc_endpoint_out(&mut self, t: EndpointType, a: Option<EndpointAddress>, mps: u16, iv: u8) -> Result<Self::EndpointOut, EndpointAllocError> {
        self.alloc(Direction::Out, t, a, mps, iv)
    }

    fn alloc_endpoint_in(&mut self, t: EndpointType, a: Option<EndpointAddress>, mps: u16, iv: u8) -> Result<Self::EndpointIn, EndpointAllocError> {
        self.alloc(Direction::In, t, a, mps, iv)
    }

    fn start(self, control_max_packet_size: u16) -> (Self::Bus, Self::ControlPipe) {
        let pipe = ControlPipe { core: self.core, mps: control_max_packet_size as usize, seq: 0 };
        (Bus { core: self.core }, pipe)
    }
}

pub struct Bus<'d, R: Mmio, D: Mmio> {
    core: Shared<'d, R, D>,
}

impl<'d, R: Mmio, D: Mmio> usb::Bus for Bus<'d, R, D> {
    async fn enable(&mut self) {
        let mut c = self.core.borrow_mut();
        let r = c.connect();
        c.note(r);
    }

    async fn disable(&mut self) {
        let mut c = self.core.borrow_mut();
        let r = c.disconnect();
        c.note(r);
    }

    async fn poll(&mut self) -> usb::Event {
        match wait(self.core, |c| c.take_bus_event()).await {
            BusEvent::PowerDetected => usb::Event::PowerDetected,
            BusEvent::Reset => usb::Event::Reset,
            BusEvent::Suspend => usb::Event::Suspend,
            BusEvent::Resume => usb::Event::Resume,
        }
    }

    fn endpoint_set_enabled(&mut self, a: EndpointAddress, enabled: bool) {
        if a.index() != 0 {
            let mut c = self.core.borrow_mut();
            let r = c.ep_set_enabled(phys(a.index() as u8, a.is_in()), enabled);
            c.note(r);
        }
    }

    fn endpoint_set_stalled(&mut self, a: EndpointAddress, stalled: bool) {
        if a.index() != 0 {
            let mut c = self.core.borrow_mut();
            let r = c.ep_set_stalled(phys(a.index() as u8, a.is_in()), stalled);
            c.note(r);
        }
    }

    fn endpoint_is_stalled(&mut self, a: EndpointAddress) -> bool {
        a.index() != 0 && self.core.borrow().eps[phys(a.index() as u8, a.is_in()) as usize].stalled
    }

    async fn remote_wakeup(&mut self) -> Result<(), usb::Unsupported> {
        Err(usb::Unsupported)
    }
}

pub struct Endpoint<'d, R: Mmio, D: Mmio> {
    core: Shared<'d, R, D>,
    phys: u8,
    info: EndpointInfo,
}

impl<'d, R: Mmio, D: Mmio> usb::Endpoint for Endpoint<'d, R, D> {
    fn info(&self) -> &EndpointInfo {
        &self.info
    }

    async fn wait_enabled(&mut self) {
        let p = self.phys as usize;
        wait(self.core, |c| c.eps[p].enabled.then_some(())).await
    }
}

impl<'d, R: Mmio, D: Mmio> usb::EndpointOut for Endpoint<'d, R, D> {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, EndpointError> {
        let p = self.phys;
        wait(self.core, |c| {
            let ep = c.eps[p as usize];
            if !ep.enabled {
                return Some(Err(EndpointError::Disabled));
            }
            match ep.xfer {
                Xfer::Idle => {
                    let r = c.ep_start(p, None);
                    let failed = r.is_err();
                    c.note(r);
                    failed.then_some(Err(EndpointError::Disabled))
                }
                Xfer::Busy { .. } => None,
                Xfer::Done { actual } => {
                    c.eps[p as usize].xfer = Xfer::Idle;
                    let n = actual as usize;
                    if n > buf.len() {
                        return Some(Err(EndpointError::BufferOverflow));
                    }
                    c.ep_read_buf(p, &mut buf[..n]);
                    Some(Ok(n))
                }
            }
        })
        .await
    }
}

impl<'d, R: Mmio, D: Mmio> usb::EndpointIn for Endpoint<'d, R, D> {
    async fn write(&mut self, buf: &[u8]) -> Result<(), EndpointError> {
        if buf.len() > self.info.max_packet_size as usize {
            return Err(EndpointError::BufferOverflow);
        }
        let p = self.phys;
        let mut started = false;
        wait(self.core, |c| {
            let ep = c.eps[p as usize];
            if !ep.enabled {
                return Some(Err(EndpointError::Disabled));
            }
            match (started, ep.xfer) {
                (_, Xfer::Busy { .. }) => None,
                (true, Xfer::Done { .. }) => {
                    c.eps[p as usize].xfer = Xfer::Idle;
                    Some(Ok(()))
                }
                (false, _) => {
                    let r = c.ep_start(p, Some(buf));
                    started = r.is_ok();
                    c.note(r);
                    (!started).then_some(Err(EndpointError::Disabled))
                }
                (true, Xfer::Idle) => Some(Err(EndpointError::Disabled)),
            }
        })
        .await
    }
}

pub struct ControlPipe<'d, R: Mmio, D: Mmio> {
    core: Shared<'d, R, D>,
    mps: usize,
    seq: u32,
}

impl<'d, R: Mmio, D: Mmio> ControlPipe<'d, R, D> {
    fn act(&self, f: impl FnOnce(&mut Dwc3<R, D>, u32) -> Result<(), crate::Error>) {
        let mut c = self.core.borrow_mut();
        let r = f(&mut c, self.seq);
        c.note(r);
    }
}

impl<'d, R: Mmio, D: Mmio> usb::ControlPipe for ControlPipe<'d, R, D> {
    fn max_packet_size(&self) -> usize {
        self.mps
    }

    async fn setup(&mut self) -> [u8; 8] {
        let (pkt, seq) = wait(self.core, |c| c.ep0_take_setup()).await;
        self.seq = seq;
        pkt
    }

    async fn data_out(&mut self, buf: &mut [u8], first: bool, _last: bool) -> Result<usize, EndpointError> {
        let seq = self.seq;
        if first {
            let mut c = self.core.borrow_mut();
            match c.ep0_out_begin(seq) {
                Ok(Begin::Armed) => {}
                Ok(Begin::Overflow) => return Err(EndpointError::BufferOverflow),
                Ok(_) => return Err(EndpointError::Disabled),
                Err(e) => {
                    c.note(Err(e));
                    return Err(EndpointError::Disabled);
                }
            }
        }
        wait(self.core, |c| c.ep0_out_read(seq, buf)).await.ok_or(EndpointError::Disabled)
    }

    async fn data_in(&mut self, data: &[u8], first: bool, last: bool) -> Result<(), EndpointError> {
        let seq = self.seq;
        let begun = self.core.borrow_mut().ep0_in_begin(seq, data, first, last);
        match begun {
            Ok(Begin::Queued) => return Ok(()),
            Ok(Begin::Armed) => {}
            Ok(Begin::Overflow) => return Err(EndpointError::BufferOverflow),
            Ok(Begin::Aborted) => return Err(EndpointError::Disabled),
            Err(e) => {
                self.core.borrow_mut().note(Err(e));
                return Err(EndpointError::Disabled);
            }
        }
        match wait(self.core, |c| c.ep0_in_status(seq)).await {
            true => Ok(()),
            false => Err(EndpointError::Disabled),
        }
    }

    async fn accept(&mut self) {
        self.act(|c, s| c.ep0_accept(s));
    }

    async fn reject(&mut self) {
        self.act(|c, s| c.ep0_reject(s));
    }

    async fn accept_set_address(&mut self, addr: u8) {
        self.act(|c, s| c.ep0_set_address(s, addr));
    }
}
