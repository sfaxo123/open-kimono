//! Event-ring draining and dispatch, bus-event queue (spec §4 Events, USB reset, Connect done).

use crate::controller::{BusEvent, Dwc3, Error, BUS_Q};
use crate::event::{DevEvent, EpEvent, Event};
use crate::layout;
use crate::regs;
use mmio_replay::Mmio;

impl<R: Mmio, D: Mmio> Dwc3<R, D> {
    pub(crate) fn push_bus(&mut self, e: BusEvent) {
        if let Some(slot) = self.bus_q.iter_mut().find(|s| s.is_none()) {
            *slot = Some(e);
        } else {
            self.bus_q.rotate_left(1);
            self.bus_q[BUS_Q - 1] = Some(e);
        }
    }

    /// Next bus event; the first call reports power (VBUS is overridden, not sensed).
    pub fn take_bus_event(&mut self) -> Option<BusEvent> {
        if !self.power_reported {
            self.power_reported = true;
            return Some(BusEvent::PowerDetected);
        }
        let e = self.bus_q[0].take();
        self.bus_q.rotate_left(1);
        e
    }

    /// Drain the event ring. Call from any context that waits.
    pub fn poll(&mut self) {
        for _ in 0..8 {
            if !self.running {
                return;
            }
            let count = self.rd(regs::GEVNTCOUNT0) & regs::GEVNTCOUNT_MASK;
            if count == 0 {
                return;
            }
            if count > layout::EVENT_RING_LEN || !count.is_multiple_of(4) {
                self.note(Err(Error::EventCount(count)));
                self.wr(regs::GEVNTCOUNT0, count);
                return;
            }
            for _ in 0..count / 4 {
                let a = self.cfg.dma_base + layout::EVENT_RING + self.ev_read as u64;
                let w = self.dma.read_u32(a);
                self.ev_read = (self.ev_read + 4) % layout::EVENT_RING_LEN;
                self.dispatch(Event::decode(w));
            }
            self.wr(regs::GEVNTCOUNT0, count);
        }
    }

    fn dispatch(&mut self, e: Event) {
        self.events_handled += 1;
        match e {
            Event::Dev(DevEvent::UsbReset) => {
                let r = self.on_usb_reset();
                self.note(r);
            }
            Event::Dev(DevEvent::ConnectDone) => {
                let r = self.on_connect_done();
                self.note(r);
            }
            Event::Dev(DevEvent::Disconnect) => {
                self.disable_user_eps();
                self.push_bus(BusEvent::Suspend);
            }
            Event::Dev(DevEvent::SuspendEntry) => self.push_bus(BusEvent::Suspend),
            Event::Dev(DevEvent::Wakeup) => self.push_bus(BusEvent::Resume),
            Event::Ep { phys: phys @ 0..=1, kind } => {
                let r = self.ep0_event(phys, kind);
                self.note(r);
            }
            Event::Ep { phys, kind: EpEvent::XferComplete } => self.ep_complete(phys),
            Event::Ep { .. } => {}
            Event::Dev(_) | Event::Unknown(_) => self.events_ignored += 1,
        }
    }

    fn on_usb_reset(&mut self) -> Result<(), Error> {
        let v = self.rd(regs::DCFG);
        self.wr(regs::DCFG, v & !regs::DCFG_DEVADDR_MASK);
        self.disable_user_eps();
        self.ep0_bus_reset()?;
        self.push_bus(BusEvent::Reset);
        Ok(())
    }

    fn on_connect_done(&mut self) -> Result<(), Error> {
        let spd = self.rd(regs::DSTS) & regs::DSTS_CONNECTSPD_MASK;
        if spd != regs::CONNECTSPD_HIGH && spd != regs::CONNECTSPD_FULL {
            return Err(Error::UnsupportedSpeed(spd));
        }
        self.ep0.mps = 64;
        self.configure_ep(0, 0, 64, 0, true)?;
        self.configure_ep(1, 0, 64, 0, true)
    }
}
