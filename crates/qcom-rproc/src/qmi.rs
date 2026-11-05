//! QMI service message and bounded TLV codec.
use crate::Error;
use alloc::{vec, vec::Vec};
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Tlv {
    pub kind: u8,
    pub value: Vec<u8>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Message {
    pub kind: u8,
    pub transaction: u16,
    pub message: u16,
    pub tlvs: Vec<Tlv>,
}
impl Message {
    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        if !matches!(self.kind, 0 | 2 | 4) {
            return Err(Error::Malformed);
        }
        let mut payload = Vec::new();
        for tlv in &self.tlvs {
            let len = u16::try_from(tlv.value.len()).map_err(|_| Error::Bounds)?;
            payload.push(tlv.kind);
            payload.extend_from_slice(&len.to_le_bytes());
            payload.extend_from_slice(&tlv.value);
        }
        let len = u16::try_from(payload.len()).map_err(|_| Error::Bounds)?;
        let mut out = vec![self.kind];
        out.extend_from_slice(&self.transaction.to_le_bytes());
        out.extend_from_slice(&self.message.to_le_bytes());
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&payload);
        Ok(out)
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() < 7 || !matches!(bytes[0], 0 | 2 | 4) {
            return Err(Error::Malformed);
        }
        let u16at = |i| u16::from_le_bytes([bytes[i], bytes[i + 1]]);
        if u16at(5) as usize != bytes.len() - 7 {
            return Err(Error::Malformed);
        }
        let mut tlvs = Vec::new();
        let mut at = 7;
        while at < bytes.len() {
            if bytes.len() - at < 3 {
                return Err(Error::Malformed);
            }
            let size = u16at(at + 1) as usize;
            let end = at
                .checked_add(3)
                .and_then(|v| v.checked_add(size))
                .ok_or(Error::Bounds)?;
            let value = bytes.get(at + 3..end).ok_or(Error::Malformed)?.to_vec();
            tlvs.push(Tlv {
                kind: bytes[at],
                value,
            });
            at = end;
        }
        Ok(Self {
            kind: bytes[0],
            transaction: u16at(1),
            message: u16at(3),
            tlvs,
        })
    }
    pub fn required(&self, kind: u8) -> Result<&[u8], Error> {
        let mut matches = self.tlvs.iter().filter(|t| t.kind == kind);
        let first = matches.next().ok_or(Error::MissingItem)?;
        if matches.next().is_some() {
            return Err(Error::Duplicate);
        }
        Ok(&first.value)
    }
}
