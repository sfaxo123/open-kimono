//! Checked lookup in a firmware-provided global SMEM image, plus a host model.
use crate::Error;
use alloc::{collections::BTreeMap, vec::Vec};
const TOC: usize = 4 * 16 + 32 * 4 + 4 * 4;
const ENTRY: usize = 16;
fn word(bytes: &[u8], at: usize) -> Result<u32, Error> {
    let end = at.checked_add(4).ok_or(Error::Bounds)?;
    Ok(u32::from_le_bytes(
        bytes.get(at..end).ok_or(Error::Bounds)?.try_into().unwrap(),
    ))
}
/// Firmware image lookup; no raw pointer escapes a checked slice.
pub struct Global<'a> {
    image: &'a [u8],
    aux_base: u32,
}
impl<'a> Global<'a> {
    pub fn new(image: &'a [u8], aux_base: u32) -> Self {
        Self { image, aux_base }
    }
    pub fn get(&self, item: u16) -> Result<&'a [u8], Error> {
        if item >= 512 {
            return Err(Error::InvalidId);
        }
        let at = TOC + item as usize * ENTRY;
        if word(self.image, at)? == 0 {
            return Err(Error::MissingItem);
        }
        let offset = word(self.image, at + 4)? as usize;
        let len = word(self.image, at + 8)? as usize;
        let aux = word(self.image, at + 12)? & !3;
        if aux != 0 && aux != self.aux_base {
            return Err(Error::MissingItem);
        }
        if len == 0 {
            return Err(Error::Malformed);
        }
        let end = offset.checked_add(len).ok_or(Error::Bounds)?;
        self.image.get(offset..end).ok_or(Error::Bounds)
    }
}
/// Mutable shared-memory fixture for protocol tests; not a hardware allocator.
#[derive(Default)]
pub struct Model {
    items: BTreeMap<(u16, u16), Vec<u8>>,
}
impl Model {
    pub fn insert(&mut self, host: u16, item: u16, bytes: Vec<u8>) -> Result<(), Error> {
        if item >= 512 {
            return Err(Error::InvalidId);
        }
        if self.items.contains_key(&(host, item)) {
            return Err(Error::Duplicate);
        }
        self.items.insert((host, item), bytes);
        Ok(())
    }
    pub fn get(&self, host: u16, item: u16) -> Result<&[u8], Error> {
        self.items
            .get(&(host, item))
            .map(Vec::as_slice)
            .ok_or(Error::MissingItem)
    }
}
