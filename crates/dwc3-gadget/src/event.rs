//! Event-ring word decoding (spec §3 "Events").

/// One decoded 32-bit event-buffer entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// Endpoint event for physical endpoint `phys`.
    Ep { phys: u8, kind: EpEvent },
    /// Device event.
    Dev(DevEvent),
    /// Anything this driver does not handle (raw word kept for logs).
    Unknown(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EpEvent {
    XferComplete,
    XferInProgress,
    /// `stage`: 1 data, 2 status; `active`: a transfer was already active.
    XferNotReady { stage: u8, active: bool },
    /// Endpoint command finished: command type, transfer resource index, status.
    CmdComplete { cmd: u8, rsrc: u8, status: u8 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DevEvent {
    Disconnect,
    UsbReset,
    ConnectDone,
    LinkStateChange(u8),
    Wakeup,
    SuspendEntry,
    BufferOverflow,
    Other(u8),
}

pub const STAGE_DATA: u8 = 1;
pub const STAGE_STATUS: u8 = 2;

impl Event {
    pub fn decode(w: u32) -> Self {
        if w & 1 == 0 {
            let phys = ((w >> 1) & 0x1f) as u8;
            let status = ((w >> 12) & 0xf) as u8;
            let kind = match (w >> 6) & 0xf {
                1 => EpEvent::XferComplete,
                2 => EpEvent::XferInProgress,
                3 => EpEvent::XferNotReady {
                    stage: status & 0b11,
                    active: w & (1 << 15) != 0,
                },
                7 => EpEvent::CmdComplete {
                    cmd: ((w >> 24) & 0xf) as u8,
                    rsrc: ((w >> 16) & 0x7f) as u8,
                    status,
                },
                _ => return Event::Unknown(w),
            };
            return Event::Ep { phys, kind };
        }
        if (w >> 1) & 0x7f != 0 {
            return Event::Unknown(w);
        }
        let info = (w >> 16) as u8;
        Event::Dev(match (w >> 8) & 0x7f {
            0 => DevEvent::Disconnect,
            1 => DevEvent::UsbReset,
            2 => DevEvent::ConnectDone,
            3 => DevEvent::LinkStateChange(info & 0xf),
            4 => DevEvent::Wakeup,
            6 => DevEvent::SuspendEntry,
            11 => DevEvent::BufferOverflow,
            t => DevEvent::Other(t as u8),
        })
    }

    /// Encode (inverse of [`Event::decode`]); used by controller models.
    pub fn encode(self) -> u32 {
        match self {
            Event::Unknown(w) => w,
            Event::Dev(d) => {
                let (t, info) = match d {
                    DevEvent::Disconnect => (0, 0),
                    DevEvent::UsbReset => (1, 0),
                    DevEvent::ConnectDone => (2, 0),
                    DevEvent::LinkStateChange(s) => (3, s as u32),
                    DevEvent::Wakeup => (4, 0),
                    DevEvent::SuspendEntry => (6, 0),
                    DevEvent::BufferOverflow => (11, 0),
                    DevEvent::Other(t) => (t as u32, 0),
                };
                1 | (t << 8) | (info << 16)
            }
            Event::Ep { phys, kind } => {
                let base = (phys as u32 & 0x1f) << 1;
                base | match kind {
                    EpEvent::XferComplete => 1 << 6,
                    EpEvent::XferInProgress => 2 << 6,
                    EpEvent::XferNotReady { stage, active } => {
                        (3 << 6) | ((stage as u32 & 0b11) << 12) | ((active as u32) << 15)
                    }
                    EpEvent::CmdComplete { cmd, rsrc, status } => {
                        (7 << 6)
                            | ((status as u32 & 0xf) << 12)
                            | ((rsrc as u32 & 0x7f) << 16)
                            | ((cmd as u32 & 0xf) << 24)
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_events_decode() {
        assert_eq!(Event::decode(0x0000_0101), Event::Dev(DevEvent::UsbReset));
        assert_eq!(Event::decode(0x0000_0201), Event::Dev(DevEvent::ConnectDone));
        assert_eq!(Event::decode(0x0003_0301), Event::Dev(DevEvent::LinkStateChange(3)));
        assert_eq!(Event::decode(0x0000_0b01), Event::Dev(DevEvent::BufferOverflow));
    }

    #[test]
    fn non_device_class_events_are_unknown() {
        // Bits 7:1 != 0 select carkit/I2C event classes, not device events.
        assert_eq!(Event::decode(0x0000_0103), Event::Unknown(0x103));
    }

    #[test]
    fn endpoint_events_decode() {
        // XferComplete on phys 1 (EP0 IN).
        assert_eq!(
            Event::decode(0x42),
            Event::Ep { phys: 1, kind: EpEvent::XferComplete }
        );
        // XferNotReady(status) on phys 0.
        let w = (3 << 6) | (2 << 12);
        assert_eq!(
            Event::decode(w),
            Event::Ep { phys: 0, kind: EpEvent::XferNotReady { stage: 2, active: false } }
        );
        // StartTransfer complete, resource 3, on phys 4.
        let w = (4 << 1) | (7 << 6) | (3 << 16) | (6 << 24);
        assert_eq!(
            Event::decode(w),
            Event::Ep { phys: 4, kind: EpEvent::CmdComplete { cmd: 6, rsrc: 3, status: 0 } }
        );
        assert_eq!(Event::decode(5 << 6), Event::Unknown(5 << 6));
    }

    #[test]
    fn encode_decode_roundtrip() {
        let all = [
            Event::Dev(DevEvent::Disconnect),
            Event::Dev(DevEvent::SuspendEntry),
            Event::Dev(DevEvent::Wakeup),
            Event::Ep { phys: 9, kind: EpEvent::XferNotReady { stage: 1, active: true } },
            Event::Ep { phys: 31, kind: EpEvent::CmdComplete { cmd: 8, rsrc: 127, status: 1 } },
            Event::Ep { phys: 2, kind: EpEvent::XferInProgress },
        ];
        for e in all {
            assert_eq!(Event::decode(e.encode()), e);
        }
    }
}
