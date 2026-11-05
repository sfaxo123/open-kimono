//! QRTR V1 datagrams and a local nameserver/port table.
use crate::Error;
use alloc::{collections::BTreeMap, vec::Vec};
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Packet {
    pub kind: u32,
    pub src_node: u32,
    pub src_port: u32,
    pub confirm_rx: bool,
    pub dst_node: u32,
    pub dst_port: u32,
    pub payload: Vec<u8>,
}
impl Packet {
    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        let size = u32::try_from(self.payload.len()).map_err(|_| Error::Bounds)?;
        let mut b = Vec::with_capacity(32 + self.payload.len());
        for n in [
            1,
            self.kind,
            self.src_node,
            self.src_port,
            self.confirm_rx as u32,
            size,
            self.dst_node,
            self.dst_port,
        ] {
            b.extend_from_slice(&n.to_le_bytes());
        }
        b.extend_from_slice(&self.payload);
        Ok(b)
    }
    pub fn decode(b: &[u8]) -> Result<Self, Error> {
        if b.len() < 32 {
            return Err(Error::Malformed);
        }
        let n = |i: usize| u32::from_le_bytes(b[i..i + 4].try_into().unwrap());
        if n(0) != 1 || n(16) > 1 || n(20) as usize != b.len() - 32 {
            return Err(Error::Malformed);
        }
        Ok(Self {
            kind: n(4),
            src_node: n(8),
            src_port: n(12),
            confirm_rx: n(16) != 0,
            dst_node: n(24),
            dst_port: n(28),
            payload: b[32..].to_vec(),
        })
    }
}
/// Local service discovery table; wire-level nameserver control is a later transport adapter.
#[derive(Default)]
pub struct Nameserver {
    services: BTreeMap<(u32, u32), (u32, u32)>,
    ports: BTreeMap<u32, u32>,
}
impl Nameserver {
    pub fn register(
        &mut self,
        service: u32,
        instance: u32,
        node: u32,
        port: u32,
    ) -> Result<(), Error> {
        if service == 0 || port == 0 {
            return Err(Error::InvalidId);
        }
        if self.services.contains_key(&(service, instance)) {
            return Err(Error::Duplicate);
        }
        self.services.insert((service, instance), (node, port));
        Ok(())
    }
    pub fn lookup(&self, service: u32, instance: u32) -> Result<(u32, u32), Error> {
        self.services
            .get(&(service, instance))
            .copied()
            .ok_or(Error::MissingItem)
    }
    pub fn bind(&mut self, port: u32, owner: u32) -> Result<(), Error> {
        if port == 0 || self.ports.contains_key(&port) {
            return Err(Error::Duplicate);
        }
        self.ports.insert(port, owner);
        Ok(())
    }
    pub fn route(&self, packet: &Packet) -> Result<u32, Error> {
        self.ports
            .get(&packet.dst_port)
            .copied()
            .ok_or(Error::MissingItem)
    }
}
