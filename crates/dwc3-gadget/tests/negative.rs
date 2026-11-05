//! Negative paths on the scripted model, mostly at the `embassy-usb-driver`
//! trait level so each failure is observed exactly where the stack sees it.

mod common;

use common::model::{Dma, HostErr, Regs};
use common::{get_descriptor, req, Bench, DATA, STATUS};
use dwc3_gadget::event::DevEvent;
use dwc3_gadget::{Bus, ControlPipe, Driver, Endpoint, Error};
use embassy_usb_driver::{
    Bus as _, ControlPipe as _, Driver as _, EndpointAddress, EndpointError, EndpointIn as _, EndpointOut as _,
    EndpointType, Event,
};
use std::cell::RefCell;
use std::future::Future;
use std::pin::pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

type Ep = Endpoint<'static, Regs, Dma>;
type Cell<T> = Rc<RefCell<Option<T>>>;

fn now<F: Future>(f: F) -> Option<F::Output> {
    match pin!(f).poll(&mut Context::from_waker(Waker::noop())) {
        Poll::Ready(v) => Some(v),
        Poll::Pending => None,
    }
}

struct Rig {
    b: Bench,
    bus: Bus<'static, Regs, Dma>,
    pipe: Option<ControlPipe<'static, Regs, Dma>>,
    out: Option<Ep>,
    inp: Option<Ep>,
}

/// Driver with one bulk OUT/IN pair, enabled, after reset + connect-done.
fn rig() -> Rig {
    let mut b = Bench::new();
    let mut d = Driver::new(b.core);
    let out = d.alloc_endpoint_out(EndpointType::Bulk, None, 512, 0).unwrap();
    let inp = d.alloc_endpoint_in(EndpointType::Bulk, None, 512, 0).unwrap();
    let (mut bus, pipe) = d.start(64);
    assert_eq!(now(bus.poll()), Some(Event::PowerDetected));
    now(bus.enable()).unwrap();
    b.bus_reset();
    assert_eq!(now(bus.poll()), Some(Event::Reset));
    bus.endpoint_set_enabled(EndpointAddress::from(0x01), true);
    bus.endpoint_set_enabled(EndpointAddress::from(0x81), true);
    Rig { b, bus, pipe: Some(pipe), out: Some(out), inp: Some(inp) }
}

fn cell<T>() -> Cell<T> {
    Rc::new(RefCell::new(None))
}

impl Rig {
    fn read_task(&mut self, len: usize) -> Cell<Result<usize, EndpointError>> {
        let (c, r) = (cell(), self.out.take().unwrap());
        let c2 = c.clone();
        self.b.spawn(async move {
            let mut ep = r;
            let mut buf = vec![0u8; len];
            *c2.borrow_mut() = Some(ep.read(&mut buf).await);
        });
        c
    }
}

#[test]
fn unknown_request_stalls_then_next_setup_recovers() {
    let mut b = Bench::new();
    common::device::spawn_device(&mut b);
    b.pump();
    b.bus_reset();
    assert_eq!(b.control_in(req(0xc0, 0x55, 0, 0, 8)), Err(HostErr::Stall));
    assert!(b.hw.borrow().cmds.contains(&(0, 4, 0)), "DEPSSTALL on EP0");
    assert_eq!(b.control_in(get_descriptor(1, 0, 18)).unwrap().len(), 18);
    b.assert_clean();
}

#[test]
fn control_out_larger_than_ep0_buffer_stalls() {
    let mut r = rig();
    let mut pipe = r.pipe.take().unwrap();
    let res = cell();
    let res2 = res.clone();
    r.b.spawn(async move {
        pipe.setup().await;
        let mut buf = [0u8; 64];
        *res2.borrow_mut() = Some(pipe.data_out(&mut buf, true, false).await);
    });
    r.b.retry(|h| Ok(h.setup(req(0x40, 1, 0, 0, 2000)).then_some(()))).unwrap();
    r.b.pump();
    assert_eq!(*res.borrow(), Some(Err(EndpointError::BufferOverflow)));
    assert_eq!(r.b.hw.borrow_mut().out_packet(0, DATA, &[0; 64]), Err(HostErr::Stall));
    assert!(r.b.hw.borrow_mut().setup(get_descriptor(1, 0, 18)), "SETUP re-armed after stall");
    r.b.assert_clean();
}

#[test]
fn control_data_in_aborted_by_bus_reset() {
    let mut r = rig();
    let mut pipe = r.pipe.take().unwrap();
    let res = cell();
    let res2 = res.clone();
    r.b.spawn(async move {
        pipe.setup().await;
        *res2.borrow_mut() = Some(pipe.data_in(&[1; 18], true, true).await);
    });
    r.b.retry(|h| Ok(h.setup(get_descriptor(1, 0, 18)).then_some(()))).unwrap();
    r.b.pump();
    assert!(res.borrow().is_none(), "data stage still pending");
    r.b.bus_reset();
    assert_eq!(*res.borrow(), Some(Err(EndpointError::Disabled)));
    assert!(r.b.hw.borrow_mut().setup(get_descriptor(1, 0, 18)), "SETUP re-armed after reset");
    r.b.assert_clean();
}

#[test]
fn bulk_read_into_short_buffer_overflows() {
    let mut r = rig();
    let res = r.read_task(16);
    r.b.bulk_out(2, &[7; 64], 512, false).unwrap();
    r.b.pump();
    assert_eq!(*res.borrow(), Some(Err(EndpointError::BufferOverflow)));
    r.b.assert_clean();
}

#[test]
fn write_larger_than_mps_is_refused_before_hardware() {
    let mut r = rig();
    let mut ep = r.inp.take().unwrap();
    let before = r.b.hw.borrow().cmds.len();
    assert_eq!(now(ep.write(&[0; 513])), Some(Err(EndpointError::BufferOverflow)));
    assert_eq!(r.b.hw.borrow().cmds.len(), before);
}

#[test]
fn disabling_endpoint_ends_pending_transfer() {
    let mut r = rig();
    let res = r.read_task(512);
    r.b.pump();
    assert!(r.b.hw.borrow().eps[2].armed.is_some());
    r.bus.endpoint_set_enabled(EndpointAddress::from(0x01), false);
    r.b.pump();
    assert_eq!(*res.borrow(), Some(Err(EndpointError::Disabled)));
    let hw = r.b.hw.borrow();
    assert!(hw.cmds.iter().any(|c| *c == (2, 8, hw.eps[2].rsrc as u32)), "END_TRANSFER with resource index");
    assert_eq!(hw.reg(0xc720) & (1 << 2), 0, "DALEPENA cleared");
    assert!(hw.violations.is_empty(), "{:?}", hw.violations);
}

#[test]
fn stalled_in_endpoint_halts_host_until_cleared() {
    let mut r = rig();
    let a = EndpointAddress::from(0x81);
    r.bus.endpoint_set_stalled(a, true);
    assert!(r.bus.endpoint_is_stalled(a));
    assert_eq!(r.b.hw.borrow_mut().in_token(3, DATA), Err(HostErr::Stall));
    r.bus.endpoint_set_stalled(a, false);
    assert!(!r.bus.endpoint_is_stalled(a));
    let mut ep = r.inp.take().unwrap();
    r.b.spawn(async move { ep.write(b"ok").await.unwrap() });
    assert_eq!(r.b.bulk_in(3, 512).unwrap(), b"ok");
    r.b.assert_clean();
}

#[test]
fn bus_reset_disables_user_endpoints_and_aborts_reads() {
    let mut r = rig();
    let res = r.read_task(512);
    r.b.pump();
    r.b.bus_reset();
    assert_eq!(*res.borrow(), Some(Err(EndpointError::Disabled)));
    assert_eq!(now(r.bus.poll()), Some(Event::Reset));
    assert_eq!(r.b.hw.borrow().reg(0xc720), 0b11, "only EP0 left active");
    r.b.assert_clean();
}

#[test]
fn disconnect_suspend_and_wakeup_become_bus_events() {
    let mut r = rig();
    r.b.hw.borrow_mut().dev_event(DevEvent::SuspendEntry);
    assert_eq!(now(r.bus.poll()), Some(Event::Suspend));
    r.b.hw.borrow_mut().dev_event(DevEvent::Wakeup);
    assert_eq!(now(r.bus.poll()), Some(Event::Resume));
    r.b.hw.borrow_mut().dev_event(DevEvent::Disconnect);
    assert_eq!(now(r.bus.poll()), Some(Event::Suspend));
    assert_eq!(r.b.hw.borrow().reg(0xc720), 0b11);
    assert_eq!(now(r.bus.poll()), None);
    r.b.assert_clean();
}

#[test]
fn superspeed_connect_done_is_refused() {
    let mut r = rig();
    r.b.hw.borrow_mut().speed = 4;
    r.b.bus_reset();
    now(r.bus.poll());
    assert_eq!(r.b.core.borrow().fault(), Some(Error::UnsupportedSpeed(4)));
}

#[test]
fn corrupt_event_count_is_reported_and_acked() {
    let mut r = rig();
    r.b.hw.borrow_mut().ev_count = 6;
    assert_eq!(now(r.bus.poll()), None);
    assert_eq!(r.b.core.borrow().fault(), Some(Error::EventCount(6)));
    assert_eq!(r.b.hw.borrow().ev_count, 0);
}

#[test]
fn endpoint_allocation_limits() {
    let b = Bench::new();
    let mut d = Driver::new(b.core);
    assert!(d.alloc_endpoint_in(EndpointType::Isochronous, None, 64, 1).is_err());
    assert!(d.alloc_endpoint_in(EndpointType::Control, None, 64, 0).is_err());
    assert!(d.alloc_endpoint_out(EndpointType::Bulk, None, 0, 0).is_err());
    assert!(d.alloc_endpoint_out(EndpointType::Bulk, None, 1024, 0).is_err());
    assert!(d.alloc_endpoint_out(EndpointType::Bulk, Some(EndpointAddress::from(0x82)), 512, 0).is_err(), "direction");
    let ep = d.alloc_endpoint_out(EndpointType::Bulk, Some(EndpointAddress::from(0x03)), 512, 0).unwrap();
    assert_eq!(u8::from(embassy_usb_driver::Endpoint::info(&ep).addr), 0x03);
    assert!(d.alloc_endpoint_out(EndpointType::Bulk, Some(EndpointAddress::from(0x03)), 512, 0).is_err(), "taken");
    // GHWPARAMS3: 8 IN physical endpoints, EP0-IN included -> 7 user IN endpoints.
    for _ in 0..7 {
        d.alloc_endpoint_in(EndpointType::Interrupt, None, 8, 10).unwrap();
    }
    assert!(d.alloc_endpoint_in(EndpointType::Bulk, None, 512, 0).is_err(), "exhausted");
}

#[test]
fn command_failure_while_enabling_is_recorded() {
    let b = Bench::new();
    let mut d = Driver::new(b.core);
    let _ep = d.alloc_endpoint_out(EndpointType::Bulk, None, 512, 0).unwrap();
    let (mut bus, _pipe) = d.start(64);
    now(bus.enable()).unwrap();
    b.hw.borrow_mut().fail_cmd = Some(0);
    bus.endpoint_set_enabled(EndpointAddress::from(0x01), true);
    assert_eq!(b.core.borrow().fault(), Some(Error::CommandFailed { phys: 0, cmd: 9, status: 1 }));
    assert_eq!(b.hw.borrow().reg(0xc720) & (1 << 2), 0, "not enabled");
}

#[test]
fn stuck_command_at_runtime_is_bounded() {
    let mut r = rig();
    r.b.hw.borrow_mut().stuck_cmd = Some(2);
    let res = r.read_task(512);
    r.b.pump();
    assert_eq!(r.b.core.borrow().fault(), Some(Error::CommandTimeout { phys: 2, cmd: 6 }));
    assert_eq!(*res.borrow(), Some(Err(EndpointError::Disabled)));
    let starts = r.b.hw.borrow().cmds.iter().filter(|c| c.0 == 2 && c.1 == 6).count();
    assert_eq!(starts, 1, "failed start is not retried");
}

#[test]
fn remote_wakeup_is_unsupported_and_status_before_decision_waits() {
    let mut r = rig();
    assert!(now(r.bus.remote_wakeup()).unwrap().is_err());
    // Host polls the status stage before the stack accepts: NAK, then ACK.
    let mut pipe = r.pipe.take().unwrap();
    r.b.retry(|h| Ok(h.setup(req(0x00, 9, 1, 0, 0)).then_some(()))).unwrap();
    assert_eq!(r.b.hw.borrow_mut().in_token(1, STATUS), Ok(None));
    now(pipe.setup()).unwrap();
    now(pipe.accept()).unwrap();
    r.b.pump();
    assert_eq!(r.b.hw.borrow_mut().in_token(1, STATUS), Ok(Some(vec![])));
    r.b.assert_clean();
}
