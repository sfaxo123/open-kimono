//! HW-11 deterministic console evidence: a fixed banner rendered through
//! [`Console`] onto a fixed [`Frame`], hashed with a hand-written SHA-256.
//!
//! The digest of the 320x128 render is pinned in `tests/fb_write.rs` as the
//! oracle for later device-side comparison (the fastboot boot attempt, gated
//! separately). Everything here is `no_std + alloc` safe and deterministic:
//! no time, no randomness, no external crates.

use crate::{Console, Frame};
use alloc::vec;
use alloc::vec::Vec;

/// The fixed banner text — prefix, digits, lowercase a-z and the 32
/// punctuation/symbol glyphs, on one line that wraps naturally through the
/// console grid. Byte-for-byte pinned: any edit changes the digest.
pub const BANNER: &str = "REDOX OP6 CONSOLE HW-11 0123456789 abcdefghijklmnopqrstuvwxyz !\"#$%&'()*+,-./:;<=>?@[]^_`{|}~";

/// Default console colours (the existing tests use the same): white on
/// black, X888 LE.
const FG_WHITE: u32 = 0x00FFFFFF;
const BG_BLACK: u32 = 0x00000000;

/// Render [`BANNER`] through a default-colour [`Console`] onto a zeroed
/// `width x height` frame (stride = width) and return the pixel buffer.
/// Deterministic for any fixed `(width, height)`; degenerate sizes simply
/// draw less (the [`Frame`] geometry invariant).
pub fn render_banner(width: usize, height: usize) -> Vec<u32> {
    let mut buffer = vec![0u32; width * height];
    {
        let frame = Frame::new(&mut buffer, width, height, width);
        let mut console = Console::new(frame, FG_WHITE, BG_BLACK);
        console.write_str(BANNER);
    }
    buffer
}

/// Hand-written SHA-256 (FIPS 180-4). No external crates. Verified against
/// the FIPS test vectors: `""` -> e3b0c442… and `"abc"` -> ba7816bf… (see
/// `sha256_known_vectors` in tests/fb_write.rs).
pub fn sha256(data: &[u8]) -> [u8; 32] {
    // FIPS 180-4 4.2.2: K[i] = first 32 bits of the fractional part of the
    // cube root of the i-th prime.
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];

    // FIPS 180-4 5.3.3: H0 = first 32 bits of the fractional part of the
    // square root of the first eight primes.
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];

    // Padding: '1' bit, zeros, 64-bit big-endian bit length; total a
    // multiple of 64 bytes.
    let bitlen = (data.len() as u64).wrapping_mul(8);
    let mut msg = Vec::with_capacity(data.len() + 72);
    msg.extend_from_slice(data);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bitlen.to_be_bytes());

    for block in msg.chunks(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                block[i * 4],
                block[i * 4 + 1],
                block[i * 4 + 2],
                block[i * 4 + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }

        let mut a = h[0];
        let mut b = h[1];
        let mut c = h[2];
        let mut d = h[3];
        let mut e = h[4];
        let mut f = h[5];
        let mut g = h[6];
        let mut hh = h[7];

        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);

            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }

        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }

    let mut out = [0u8; 32];
    for i in 0..8 {
        out[i * 4..i * 4 + 4].copy_from_slice(&h[i].to_be_bytes());
    }
    out
}

/// SHA-256 over the framebuffer bytes in X888 little-endian order.
pub fn frame_digest(w: usize, h: usize) -> [u8; 32] {
    let buffer = render_banner(w, h);
    let mut bytes = Vec::with_capacity(buffer.len() * 4);
    for px in &buffer {
        bytes.extend_from_slice(&px.to_le_bytes());
    }
    sha256(&bytes)
}
