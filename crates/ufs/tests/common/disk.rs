//! Builds logical-unit images with a valid primary GPT (UEFI layout, CRCs via `ufs_core::gpt`).

use ufs_core::gpt::crc32;

pub const TYPE_GUID: [u8; 16] = [
    0xa2, 0xa0, 0xd0, 0xeb, 0xe5, 0xb9, 0x33, 0x44, 0x87, 0xc0, 0x68, 0xb6, 0xb7, 0x26, 0x99, 0xc7,
];
const ENTRIES: u32 = 128;
const ENTRY_SIZE: u32 = 128;

/// A block-addressed image of `blocks` × `bs` bytes; block N's first 8 bytes hold `N` (LE) for
/// non-GPT blocks so reads can be checked, partitions are `(name, first, last)`.
pub fn gpt_image(bs: usize, blocks: u64, parts: &[(&str, u64, u64)]) -> Vec<u8> {
    let mut img = vec![0u8; bs * blocks as usize];
    for n in 0..blocks {
        let at = n as usize * bs;
        img[at..at + 8].copy_from_slice(&n.to_le_bytes());
        img[at + 8] = 0xa5;
    }
    let array_len = (ENTRIES * ENTRY_SIZE) as usize;
    let array_blocks = array_len.div_ceil(bs) as u64;
    let mut array = vec![0u8; array_len];
    for (i, (name, first, last)) in parts.iter().enumerate() {
        let e = &mut array[i * ENTRY_SIZE as usize..(i + 1) * ENTRY_SIZE as usize];
        e[0..16].copy_from_slice(&TYPE_GUID);
        e[16] = i as u8 + 1;
        e[32..40].copy_from_slice(&first.to_le_bytes());
        e[40..48].copy_from_slice(&last.to_le_bytes());
        for (j, u) in name.encode_utf16().enumerate() {
            e[56 + 2 * j..58 + 2 * j].copy_from_slice(&u.to_le_bytes());
        }
    }
    let mut h = vec![0u8; 92];
    h[0..8].copy_from_slice(b"EFI PART");
    h[8..12].copy_from_slice(&0x0001_0000u32.to_le_bytes());
    h[12..16].copy_from_slice(&92u32.to_le_bytes());
    h[24..32].copy_from_slice(&1u64.to_le_bytes());
    h[32..40].copy_from_slice(&(blocks - 1).to_le_bytes());
    h[40..48].copy_from_slice(&(2 + array_blocks).to_le_bytes());
    h[48..56].copy_from_slice(&(blocks - 2 - array_blocks).to_le_bytes());
    h[56] = 0x6f; // disk GUID byte 0
    h[72..80].copy_from_slice(&2u64.to_le_bytes());
    h[80..84].copy_from_slice(&ENTRIES.to_le_bytes());
    h[84..88].copy_from_slice(&ENTRY_SIZE.to_le_bytes());
    h[88..92].copy_from_slice(&crc32(&array).to_le_bytes());
    let crc = crc32(&h);
    h[16..20].copy_from_slice(&crc.to_le_bytes());
    img[bs..2 * bs].fill(0);
    img[bs..bs + 92].copy_from_slice(&h);
    let a = 2 * bs;
    img[a..a + array_blocks as usize * bs].fill(0);
    img[a..a + array_len].copy_from_slice(&array);
    img
}
