//! Conservative RMI4 PDT discovery and F12 Data1 contact decoding.
use geni_i2c_core::{Bus, Error as BusError};

pub const ADDRESS: u8 = 0x20;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Bus(BusError),
    MissingFunction(u8),
    DuplicateFunction(u8),
    InvalidDescriptor,
    UnconfiguredReport,
    InvalidAttention,
}
impl From<BusError> for Error {
    fn from(e: BusError) -> Self {
        Self::Bus(e)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Function {
    pub query: u8,
    pub command: u8,
    pub control: u8,
    pub data: u8,
    pub irqs: u8,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Functions {
    pub f01: Function,
    pub f12: Function,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Contact {
    pub slot: u8,
    pub x: u16,
    pub y: u16,
    pub pressure: u8,
}
/// Data1 layout is learned from a device query/oracle; never assume a fixed offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Data1 {
    pub offset: u8,
    pub slots: u8,
}
impl Data1 {
    pub fn checked(offset: u8, slots: u8) -> Result<Self, Error> {
        if slots == 0 || slots > 10 || usize::from(offset) + usize::from(slots) * 8 > 256 {
            Err(Error::InvalidDescriptor)
        } else {
            Ok(Self { offset, slots })
        }
    }
}

pub struct Rmi4<B: Bus> {
    bus: B,
    functions: Functions,
    data1: Option<Data1>,
}
impl<B: Bus> Rmi4<B> {
    pub fn discover(mut bus: B) -> Result<Self, Error> {
        let mut f01 = None;
        let mut f12 = None;
        for addr in (0x05..=0xe9).rev().step_by(6) {
            let mut p = [0; 6];
            bus.write_read(ADDRESS, &[addr], &mut p)?;
            if p[5] == 0 {
                break;
            }
            let fun = Function {
                query: p[0],
                command: p[1],
                control: p[2],
                data: p[3],
                irqs: p[4] & 7,
            };
            let target = match p[5] {
                1 => &mut f01,
                0x12 => &mut f12,
                _ => continue,
            };
            if target.replace(fun).is_some() {
                return Err(Error::DuplicateFunction(p[5]));
            }
        }
        Ok(Self {
            bus,
            functions: Functions {
                f01: f01.ok_or(Error::MissingFunction(1))?,
                f12: f12.ok_or(Error::MissingFunction(0x12))?,
            },
            data1: None,
        })
    }
    pub fn functions(&self) -> Functions {
        self.functions
    }
    /// Only call with a descriptor validated against live F12 queries.
    pub fn set_data1(&mut self, descriptor: Data1) {
        self.data1 = Some(descriptor);
    }
    pub fn poll(&mut self) -> Result<Vec<Contact>, Error> {
        let desc = self.data1.ok_or(Error::UnconfiguredReport)?;
        let mut irq = [0; 2];
        self.bus
            .write_read(ADDRESS, &[self.functions.f01.data], &mut irq)?;
        if irq[0] & 0xc0 != 0 {
            return Ok(Vec::new());
        } // unconfigured/reset/flash state
        let mut raw = vec![0; usize::from(desc.slots) * 8];
        let address = self
            .functions
            .f12
            .data
            .checked_add(desc.offset)
            .ok_or(Error::InvalidDescriptor)?;
        self.bus.write_read(ADDRESS, &[address], &mut raw)?;
        Ok(decode(&raw))
    }
    pub fn into_bus(self) -> B {
        self.bus
    }
}
/// Attention interrupt from `interrupts-extended = <&controller pin flags>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Attention {
    pub controller: u32,
    pub pin: u32,
    pub flags: u32,
}
impl Attention {
    /// One specifier for a two-cell controller such as TLMM; DTB cells are big-endian.
    pub fn parse(raw: &[u8]) -> Result<Self, Error> {
        let cell = |i: usize| u32::from_be_bytes([raw[i], raw[i + 1], raw[i + 2], raw[i + 3]]);
        if raw.len() != 12 {
            return Err(Error::InvalidAttention);
        }
        let attention = Self {
            controller: cell(0),
            pin: cell(4),
            flags: cell(8),
        };
        if attention.controller == 0 || !matches!(attention.flags, 1 | 2 | 3 | 4 | 8) {
            return Err(Error::InvalidAttention);
        }
        Ok(attention)
    }
    /// Kernel IRQ scheme path that claims this specifier.
    pub fn irq_path(&self) -> String {
        format!(
            "/scheme/irq/phandle-{}/{},{}",
            self.controller, self.pin, self.flags
        )
    }
}

/// Single-pointer event derived from F12 contacts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pointer {
    Move { x: u16, y: u16 },
    Button { pressed: bool },
}

/// The first finger moves the pointer and holds the left button.
#[derive(Debug, Default)]
pub struct Tracker {
    pressed: bool,
    last: Option<(u16, u16)>,
}
impl Tracker {
    pub fn update(&mut self, contacts: &[Contact]) -> Vec<Pointer> {
        let mut events = Vec::new();
        match contacts.first() {
            Some(contact) => {
                let position = (contact.x, contact.y);
                if !self.pressed || self.last != Some(position) {
                    events.push(Pointer::Move {
                        x: contact.x,
                        y: contact.y,
                    });
                    self.last = Some(position);
                }
                if !self.pressed {
                    events.push(Pointer::Button { pressed: true });
                    self.pressed = true;
                }
            }
            None => events.extend(self.release()),
        }
        events
    }
    /// Releases a held button, for example before exiting on a bus error.
    pub fn release(&mut self) -> Option<Pointer> {
        let held = std::mem::take(&mut self.pressed);
        held.then_some(Pointer::Button { pressed: false })
    }
}

/// F12 Data1: one eight-byte object record per slot; only fingers are exposed.
pub fn decode(raw: &[u8]) -> Vec<Contact> {
    raw.chunks_exact(8)
        .enumerate()
        .filter_map(|(slot, o)| {
            if o[0] & 0x0f != 1 {
                return None;
            }
            let x = u16::from_le_bytes([o[1], o[2]]);
            if x > 1079 {
                return None;
            }
            Some(Contact {
                slot: slot as u8,
                x,
                y: u16::from_le_bytes([o[3], o[4]]),
                pressure: o[5],
            })
        })
        .collect()
}
