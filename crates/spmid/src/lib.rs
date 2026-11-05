//! Read-only SDM845 SPMI arbiter v5 and PM8998 button sampling.
use mmio_replay::Mmio;

const VALID_VERSION_MIN: u32 = 0x5000_0000;
const VALID_VERSION_MAX: u32 = 0x7000_0000;
const DONE: u32 = 1;
const MAX_CHANNELS: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    UnsupportedVersion(u32),
    BadChannel,
    Timeout,
    BadStatus(u32),
}

#[derive(Debug, Clone, Copy)]
pub struct Windows {
    pub core: u64,
    pub observer: u64,
    pub config: u64,
    pub owner: u8,
}

pub struct Arbiter<M: Mmio> {
    io: M,
    windows: Windows,
    channels: Vec<Option<u16>>,
}

impl<M: Mmio> Arbiter<M> {
    pub fn new(mut io: M, windows: Windows) -> Result<Self, Error> {
        let version = io.read_u32(windows.core);
        if !(VALID_VERSION_MIN..VALID_VERSION_MAX).contains(&version) {
            return Err(Error::UnsupportedVersion(version));
        }
        let count = ((io.read_u32(windows.core + 4) & 0x7ff) as usize).min(MAX_CHANNELS);
        // The 4096-entry table belongs on the heap, not the small Redox driver stack.
        let mut channels = vec![None; 16 * 256];
        for apid in 0..count {
            let entry = io.read_u32(windows.core + 0x900 + 4 * apid as u64);
            if entry == 0 {
                continue;
            }
            let sid = ((entry >> 16) & 0xf) as usize;
            let pid = ((entry >> 8) & 0xff) as usize;
            let owner = (io.read_u32(windows.config + 0x700 + 4 * apid as u64) & 7) as u8;
            // Prefer an APID owned by our EE, but observer reads work on either.
            let channel = &mut channels[sid * 256 + pid];
            if channel.is_none() || owner == windows.owner {
                *channel = Some(apid as u16);
            }
        }
        Ok(Self {
            io,
            windows,
            channels,
        })
    }

    pub fn read_byte(&mut self, sid: u8, addr: u16) -> Result<u8, Error> {
        let channel = self
            .channels
            .get(sid as usize * 256 + (addr >> 8) as usize)
            .copied()
            .flatten()
            .ok_or(Error::BadChannel)?;
        let base = self.windows.observer + u64::from(channel) * 0x80;
        self.io.write_u32(base + 4, 0);
        self.io
            .write_u32(base, (1 << 27) | (u32::from(addr & 0xff) << 4) | 1);
        for _ in 0..1024 {
            let status = self.io.read_u32(base + 8);
            if status & DONE != 0 {
                if status != DONE {
                    return Err(Error::BadStatus(status));
                }
                return Ok(self.io.read_u32(base + 0x18) as u8);
            }
        }
        Err(Error::Timeout)
    }

    pub fn into_io(self) -> M {
        self.io
    }

    pub fn pmic_type(&mut self) -> Result<u8, Error> {
        self.read_byte(0, 0x104)
    }
    pub fn sample(&mut self) -> Result<Levels, Error> {
        Ok(Levels {
            power: self.read_byte(0, 0x810)? & 0x80 != 0,
            volume_down: self.read_byte(0, 0xc410)? & 1 == 0,
            volume_up: self.read_byte(0, 0xc510)? & 1 == 0,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Levels {
    pub power: bool,
    pub volume_down: bool,
    pub volume_up: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Power,
    VolumeDown,
    VolumeUp,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Edge {
    pub button: Button,
    pub pressed: bool,
}

/// Initial sample establishes a baseline; no phantom press is emitted.
pub struct Buttons {
    previous: Option<Levels>,
    power_confirmed: bool,
}
impl Buttons {
    pub fn new(power_confirmed: bool) -> Self {
        Self {
            previous: None,
            power_confirmed,
        }
    }
    pub fn update(&mut self, levels: Levels) -> Vec<Edge> {
        let mut edges = Vec::new();
        if let Some(previous) = self.previous {
            for (button, old, new, enabled) in [
                (
                    Button::Power,
                    previous.power,
                    levels.power,
                    self.power_confirmed,
                ),
                (
                    Button::VolumeDown,
                    previous.volume_down,
                    levels.volume_down,
                    true,
                ),
                (Button::VolumeUp, previous.volume_up, levels.volume_up, true),
            ] {
                if enabled && old != new {
                    edges.push(Edge {
                        button,
                        pressed: new,
                    });
                }
            }
        }
        self.previous = Some(levels);
        edges
    }
}
