//! The DMA region the controller reads requests from and writes responses into.
//!
//! One physically contiguous, uncached region holds everything (`specs/ufs.md` §4–5):
//!
//! ```text
//! 0x0000  UTRL         32 slots × 32 B UTRD   (1 KiB aligned)
//! 0x0400  UCD slot 0   request UPIU @+0, response UPIU @+512, PRDT @+1024 (128 B aligned)
//! 0x1000  data buffer  data-in target for SCSI reads (rest of the region, ≤ 256 KiB used)
//! ```

/// A physically contiguous DMA buffer. Production: an uncached `/scheme/memory` allocation;
/// tests: a byte vector shared with the device model.
pub trait Dma {
    /// Bus address of byte 0 (physical — U-Boot leaves the UFS SMMU stream in S1 bypass).
    fn phys(&self) -> u64;
    /// Region size in bytes.
    fn size(&self) -> usize;
    /// Copy `buf.len()` bytes starting at `off` into `buf`.
    fn read(&mut self, off: usize, buf: &mut [u8]);
    /// Copy `data` into the region at `off`.
    fn write(&mut self, off: usize, data: &[u8]);
}

/// UTRL offset within the region.
pub const UTRL_OFF: usize = 0x0;
/// UTRD size.
pub const UTRD_LEN: usize = 32;
/// UCD (slot 0) offset within the region.
pub const UCD_OFF: usize = 0x400;
/// Request UPIU offset inside the UCD.
pub const UCD_REQ: usize = 0;
/// Response UPIU offset inside the UCD.
pub const UCD_RESP: usize = 512;
/// Response UPIU area size.
pub const UCD_RESP_LEN: usize = 512;
/// PRDT offset inside the UCD.
pub const UCD_PRDT: usize = 1024;
/// Data buffer offset within the region.
pub const DATA_OFF: usize = 0x1000;
/// Smallest usable region: list + UCD + one 4 KiB block.
pub const MIN_LEN: usize = DATA_OFF + 4096;
/// One PRDT entry carries at most 256 KiB.
pub const MAX_PRDT_BYTES: usize = 256 * 1024;

/// Bytes usable for one data-in transfer with this region.
pub fn data_capacity(len: usize) -> usize {
    len.saturating_sub(DATA_OFF).min(MAX_PRDT_BYTES)
}

/// Read a little-endian u32 from the region.
pub fn read_u32(dma: &mut impl Dma, off: usize) -> u32 {
    let mut b = [0u8; 4];
    dma.read(off, &mut b);
    u32::from_le_bytes(b)
}
