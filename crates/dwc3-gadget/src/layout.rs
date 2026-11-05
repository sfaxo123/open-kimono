//! Layout of the one physically contiguous, uncached DMA region the driver
//! needs (offsets from its bus address).
//!
//! | Offset | Size | Use |
//! |---|---|---|
//! | `0x0000` | 4 KiB | event ring |
//! | `0x1000` | 32 × 16 B | one TRB per physical endpoint |
//! | `0x1400` | 1 KiB | EP0 buffer (SETUP packet and control data stage) |
//! | `0x1800` | 32 × 512 B | packet bounce buffer per physical endpoint |

pub const EVENT_RING: u64 = 0x0000;
pub const EVENT_RING_LEN: u32 = 0x1000;
pub const TRBS: u64 = 0x1000;
pub const EP0_BUF: u64 = 0x1400;
pub const EP0_BUF_LEN: usize = 0x400;
pub const EP_BUFS: u64 = 0x1800;
pub const EP_BUF_LEN: usize = 0x200;
/// Total bytes of DMA memory (22 KiB); the region itself must start 4 KiB aligned.
pub const DMA_SIZE: usize = 0x1800 + 32 * EP_BUF_LEN;

pub const fn trb(phys: u8) -> u64 {
    TRBS + 16 * phys as u64
}

pub const fn ep_buf(phys: u8) -> u64 {
    EP_BUFS + (EP_BUF_LEN as u64) * phys as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regions_do_not_overlap() {
        assert!(EVENT_RING + EVENT_RING_LEN as u64 <= TRBS);
        assert!(trb(31) + 16 <= EP0_BUF);
        assert!(EP0_BUF + EP0_BUF_LEN as u64 <= EP_BUFS);
        assert_eq!(ep_buf(31) + EP_BUF_LEN as u64, DMA_SIZE as u64);
        assert_eq!(DMA_SIZE, 22 * 1024);
    }
}
