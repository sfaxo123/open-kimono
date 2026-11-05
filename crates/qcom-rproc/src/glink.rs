//! Host SMEM ring and intent/channel model. Hardware ring barriers are adapter-owned.
use crate::Error;
use alloc::{
    collections::{BTreeMap, VecDeque},
    string::String,
    vec::Vec,
};

/// Little-endian eight-byte GLINK command header.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Header {
    pub command: u16,
    pub param1: u16,
    pub param2: u32,
}
impl Header {
    pub fn encode(self) -> [u8; 8] {
        let mut b = [0; 8];
        b[..2].copy_from_slice(&self.command.to_le_bytes());
        b[2..4].copy_from_slice(&self.param1.to_le_bytes());
        b[4..].copy_from_slice(&self.param2.to_le_bytes());
        b
    }
    pub fn decode(b: &[u8]) -> Result<Self, Error> {
        if b.len() < 8 {
            return Err(Error::Malformed);
        }
        Ok(Self {
            command: u16::from_le_bytes([b[0], b[1]]),
            param1: u16::from_le_bytes([b[2], b[3]]),
            param2: u32::from_le_bytes(b[4..8].try_into().unwrap()),
        })
    }
}
/// Bounded byte FIFO standing in for a shared SMEM FIFO.
pub struct Ring {
    bytes: VecDeque<u8>,
    capacity: usize,
}
impl Ring {
    pub fn new(capacity: usize) -> Self {
        Self {
            bytes: VecDeque::new(),
            capacity,
        }
    }
    pub fn write(&mut self, b: &[u8]) -> Result<(), Error> {
        if b.len() > self.capacity.saturating_sub(self.bytes.len()) {
            return Err(Error::Full);
        }
        self.bytes.extend(b.iter().copied());
        Ok(())
    }
    pub fn read(&mut self, len: usize) -> Result<Vec<u8>, Error> {
        if len > self.bytes.len() {
            return Err(Error::Bounds);
        }
        Ok(self.bytes.drain(..len).collect())
    }
    pub fn len(&self) -> usize {
        self.bytes.len()
    }
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}
/// A complete TX_DATA frame: header, chunk length, remaining length, bytes.
/// Framing follows OpenBSD qcpas.c:947-970; the transport rounds ring writes
/// to eight bytes, so the encoded frame carries zero padding.
pub fn data_frame(channel: u16, intent: u32, payload: &[u8]) -> Result<Vec<u8>, Error> {
    let len = u32::try_from(payload.len()).map_err(|_| Error::Bounds)?;
    let mut frame = Vec::with_capacity(16 + payload.len() + 7);
    frame.extend_from_slice(
        &Header {
            command: 9,
            param1: channel,
            param2: intent,
        }
        .encode(),
    );
    frame.extend_from_slice(&len.to_le_bytes());
    frame.extend_from_slice(&0u32.to_le_bytes());
    frame.extend_from_slice(payload);
    frame.resize((frame.len() + 7) & !7, 0);
    Ok(frame)
}
/// Parse a single, non-fragmented TX_DATA frame.
pub fn parse_data_frame(frame: &[u8]) -> Result<(u16, u32, &[u8]), Error> {
    if frame.len() < 16 || !frame.len().is_multiple_of(8) {
        return Err(Error::Malformed);
    }
    let header = Header::decode(frame)?;
    if header.command != 9 || frame[12..16] != [0; 4] {
        return Err(Error::Malformed);
    }
    let len = u32::from_le_bytes(frame[8..12].try_into().unwrap()) as usize;
    let end = 16usize.checked_add(len).ok_or(Error::Bounds)?;
    if end > frame.len() || frame[end..].iter().any(|b| *b != 0) || ((end + 7) & !7) != frame.len()
    {
        return Err(Error::Malformed);
    }
    Ok((header.param1, header.param2, &frame[16..end]))
}
/// Version 1 is the only supported on-wire version.
pub fn negotiate_version(remote: Header) -> Result<Header, Error> {
    if remote.command != 0 || remote.param1 != 1 {
        return Err(Error::Malformed);
    }
    Ok(Header {
        command: 1,
        param1: 1,
        param2: remote.param2 & 1,
    })
}
/// Channel has explicit remote intents; sends consume a sufficient intent.
pub struct Link {
    names: BTreeMap<u16, String>,
    remote_intents: BTreeMap<u16, Vec<(u32, usize)>>,
}
impl Link {
    pub fn new() -> Self {
        Self {
            names: BTreeMap::new(),
            remote_intents: BTreeMap::new(),
        }
    }
    pub fn open(&mut self, channel: u16, name: &str) -> Result<Header, Error> {
        if name.is_empty()
            || self.names.values().any(|n| n == name)
            || self.names.contains_key(&channel)
        {
            return Err(Error::Duplicate);
        }
        self.names.insert(channel, String::from(name));
        Ok(Header {
            command: 2,
            param1: channel,
            param2: (name.len() + 1) as u32,
        })
    }
    pub fn advertise_remote_intent(
        &mut self,
        channel: u16,
        intent: u32,
        size: usize,
    ) -> Result<(), Error> {
        if !self.names.contains_key(&channel) {
            return Err(Error::InvalidId);
        }
        let intents = self.remote_intents.entry(channel).or_default();
        if size == 0 || intents.iter().any(|(id, _)| *id == intent) {
            return Err(Error::Duplicate);
        }
        intents.push((intent, size));
        Ok(())
    }
    pub fn send(&mut self, channel: u16, payload: &[u8]) -> Result<(Header, u32), Error> {
        if !self.names.contains_key(&channel) {
            return Err(Error::InvalidId);
        }
        let intents = self
            .remote_intents
            .get_mut(&channel)
            .ok_or(Error::NoIntent)?;
        let i = intents
            .iter()
            .position(|(_, size)| *size >= payload.len())
            .ok_or(Error::NoIntent)?;
        let (id, _) = intents.remove(i);
        Ok((
            Header {
                command: 9,
                param1: channel,
                param2: id,
            },
            id,
        ))
    }
    pub fn close(&mut self, channel: u16) -> Result<Header, Error> {
        self.names.remove(&channel).ok_or(Error::InvalidId)?;
        self.remote_intents.remove(&channel);
        Ok(Header {
            command: 3,
            param1: channel,
            param2: 0,
        })
    }
}
impl Default for Link {
    fn default() -> Self {
        Self::new()
    }
}
