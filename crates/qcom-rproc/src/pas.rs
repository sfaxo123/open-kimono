//! PAS request surface; no SMC executes in this crate.
use crate::Error;

/// Qualcomm SIP/PIL call, with the SCM arg-info word and up to three arguments.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Request {
    pub function: u32,
    pub arginfo: u32,
    pub args: [u64; 3],
}

/// Executes a prepared secure monitor call. The adapter owns conduit, barriers and DMA.
pub trait Smc {
    fn call(&mut self, request: Request) -> Result<u64, i64>;
}

const fn function(cmd: u32) -> u32 {
    (1 << 30) | (2 << 24) | (2 << 8) | cmd
}
const fn arginfo(types: &[u32]) -> u32 {
    let mut result = types.len() as u32;
    let mut i = 0;
    while i < types.len() {
        result |= types[i] << (4 + 2 * i);
        i += 1;
    }
    result
}

/// Explicitly admitted PAS identities. IDs are platform policy, never accepted blindly.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PasId(u32);
impl PasId {
    pub fn new(id: u32, allowed: &[u32]) -> Result<Self, Error> {
        if id == 0 || !allowed.contains(&id) {
            return Err(Error::InvalidId);
        }
        Ok(Self(id))
    }
    pub fn get(self) -> u32 {
        self.0
    }
}

pub struct Pas<S: Smc> {
    smc: S,
}
impl<S: Smc> Pas<S> {
    pub fn new(smc: S) -> Self {
        Self { smc }
    }
    pub fn into_inner(self) -> S {
        self.smc
    }
    fn invoke(&mut self, command: u32, types: &[u32], args: [u64; 3]) -> Result<(), Error> {
        let status = self
            .smc
            .call(Request {
                function: function(command),
                arginfo: arginfo(types),
                args,
            })
            .map_err(Error::Smc)?;
        if status == 0 {
            Ok(())
        } else {
            Err(Error::Smc(status as i64))
        }
    }
    /// Metadata address must refer to a caller-owned DMA-safe buffer.
    pub fn init_image(&mut self, id: PasId, metadata_pa: u64) -> Result<(), Error> {
        if metadata_pa == 0 || !metadata_pa.is_multiple_of(4) {
            return Err(Error::InvalidAddress);
        }
        self.invoke(1, &[0, 2], [id.0 as u64, metadata_pa, 0])
    }
    pub fn mem_setup(&mut self, id: PasId, pa: u64, size: u64) -> Result<(), Error> {
        if pa == 0
            || !pa.is_multiple_of(4096)
            || size == 0
            || !size.is_multiple_of(4096)
            || pa.checked_add(size).is_none()
        {
            return Err(Error::InvalidAddress);
        }
        self.invoke(2, &[0, 0, 0], [id.0 as u64, pa, size])
    }
    pub fn auth_and_reset(&mut self, id: PasId) -> Result<(), Error> {
        self.invoke(5, &[0], [id.0 as u64, 0, 0])
    }
    pub fn shutdown(&mut self, id: PasId) -> Result<(), Error> {
        self.invoke(6, &[0], [id.0 as u64, 0, 0])
    }
}
