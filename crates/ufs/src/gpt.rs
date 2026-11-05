//! GUID Partition Table parsing (UEFI 2.x §5.3), CRC-checked, read-only.

use crate::hc::Ufshc;
use crate::{Delay, Dma, Error, Result};
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use mmio_replay::Mmio;

/// Why a GPT was rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GptError {
    /// Block shorter than the header.
    Truncated,
    /// No `"EFI PART"` signature.
    Signature,
    /// Header size outside 92..=block size.
    HeaderSize,
    /// Header CRC32 mismatch.
    HeaderCrc,
    /// Entry size not 128 × 2^n (≤ 512) or more than 1024 entries.
    EntryLayout,
    /// Partition entry array CRC32 mismatch.
    EntriesCrc,
}

/// GPT header fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GptHeader {
    /// LBA of this header.
    pub current_lba: u64,
    /// LBA of the backup header.
    pub backup_lba: u64,
    /// First usable LBA.
    pub first_usable: u64,
    /// Last usable LBA.
    pub last_usable: u64,
    /// Disk GUID (on-disk byte order).
    pub disk_guid: [u8; 16],
    /// Start of the entry array.
    pub entries_lba: u64,
    /// Number of entries.
    pub num_entries: u32,
    /// Size of one entry.
    pub entry_size: u32,
    /// CRC32 of the entry array.
    pub entries_crc: u32,
}

/// One used partition entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GptEntry {
    /// Index in the entry array.
    pub index: u32,
    /// Partition type GUID (on-disk byte order).
    pub type_guid: [u8; 16],
    /// Unique GUID (on-disk byte order).
    pub unique_guid: [u8; 16],
    /// First LBA.
    pub first_lba: u64,
    /// Last LBA (inclusive).
    pub last_lba: u64,
    /// Attribute bits.
    pub attributes: u64,
    /// Name (UTF-16LE decoded, NUL-terminated).
    pub name: String,
}

/// A parsed table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Gpt {
    /// Header.
    pub header: GptHeader,
    /// Used entries (non-zero type GUID).
    pub entries: Vec<GptEntry>,
}

/// CRC-32/ISO-HDLC (the GPT/zlib CRC), bitwise.
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn le32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn le64(b: &[u8], at: usize) -> u64 {
    le32(b, at) as u64 | (le32(b, at + 4) as u64) << 32
}

fn guid(b: &[u8], at: usize) -> [u8; 16] {
    let mut g = [0u8; 16];
    g.copy_from_slice(&b[at..at + 16]);
    g
}

/// Parse and CRC-check the header in `block` (the LBA-1 block).
pub fn parse_header(block: &[u8]) -> core::result::Result<GptHeader, GptError> {
    if block.len() < 92 {
        return Err(GptError::Truncated);
    }
    if &block[0..8] != b"EFI PART" {
        return Err(GptError::Signature);
    }
    let size = le32(block, 12) as usize;
    if size < 92 || size > block.len() {
        return Err(GptError::HeaderSize);
    }
    let mut h = block[..size].to_vec();
    h[16..20].fill(0);
    if crc32(&h) != le32(block, 16) {
        return Err(GptError::HeaderCrc);
    }
    let header = GptHeader {
        current_lba: le64(block, 24),
        backup_lba: le64(block, 32),
        first_usable: le64(block, 40),
        last_usable: le64(block, 48),
        disk_guid: guid(block, 56),
        entries_lba: le64(block, 72),
        num_entries: le32(block, 80),
        entry_size: le32(block, 84),
        entries_crc: le32(block, 88),
    };
    let es = header.entry_size;
    if !(128..=512).contains(&es) || !es.is_power_of_two() || header.num_entries > 1024 {
        return Err(GptError::EntryLayout);
    }
    Ok(header)
}

/// CRC-check the entry array and decode the used entries.
pub fn parse_entries(h: &GptHeader, bytes: &[u8]) -> core::result::Result<Vec<GptEntry>, GptError> {
    let total = h.num_entries as usize * h.entry_size as usize;
    if bytes.len() < total {
        return Err(GptError::Truncated);
    }
    if crc32(&bytes[..total]) != h.entries_crc {
        return Err(GptError::EntriesCrc);
    }
    let mut out = Vec::new();
    for (i, e) in bytes[..total].chunks(h.entry_size as usize).enumerate() {
        if e[0..16].iter().all(|&b| b == 0) {
            continue;
        }
        let units = e[56..128]
            .chunks(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]));
        let name = char::decode_utf16(units.take_while(|&u| u != 0))
            .map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER))
            .collect();
        out.push(GptEntry {
            index: i as u32,
            type_guid: guid(e, 0),
            unique_guid: guid(e, 16),
            first_lba: le64(e, 32),
            last_lba: le64(e, 40),
            attributes: le64(e, 48),
            name,
        });
    }
    Ok(out)
}

impl<M: Mmio, D: Dma, T: Delay> Ufshc<M, D, T> {
    /// Read and verify the primary GPT of a LU (header at LBA 1).
    pub fn read_gpt(&mut self, lun: u8, block_size: u32) -> Result<Gpt> {
        let bs = block_size as usize;
        let mut block = vec![0u8; bs];
        self.read(lun, 1, block_size, &mut block)?;
        let header = parse_header(&block).map_err(Error::Gpt)?;
        let bytes = header.num_entries as usize * header.entry_size as usize;
        let mut array = vec![0u8; bytes.div_ceil(bs) * bs];
        self.read(lun, header.entries_lba, block_size, &mut array)?;
        let entries = parse_entries(&header, &array).map_err(Error::Gpt)?;
        Ok(Gpt { header, entries })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_check_value() {
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn header_rejects_garbage() {
        assert_eq!(parse_header(&[0u8; 10]), Err(GptError::Truncated));
        assert_eq!(parse_header(&[0u8; 512]), Err(GptError::Signature));
    }
}
