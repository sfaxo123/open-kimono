//! Self-contained SHA-256 (FIPS 180-4) and lowercase-hex encoding.
//!
//! The crate's dependency list is pinned (regex, serde, serde_json) and
//! the build runs offline, so the inventory tool carries its own minimal
//! implementation rather than adding a dependency (the dependency-free
//! choice is deliberate: pinned deps + offline build + zero new deps is
//! the packet's rule). It is checked against the four FIPS 180-4
//! example digests and a chunked 1 MiB stream (see `tests` below).

/// Streaming SHA-256 state: eight words plus the ≤64-byte block buffer,
/// so a digest never allocates.
#[derive(Clone, Copy)]
pub struct Sha256 {
    state: [u32; 8],
    block: [u8; 64],
    buffered: usize,
    total_len: u64,
}

/// FIPS 180-4 §4.2.2 initial hash value H0.
const H0: [u32; 8] = [
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
];

/// FIPS 180-4 §4.2.3 round constants K[0..64].
const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

impl Sha256 {
    pub fn new() -> Self {
        Self {
            state: H0,
            block: [0u8; 64],
            buffered: 0,
            total_len: 0,
        }
    }

    pub fn update(&mut self, data: &[u8]) {
        self.total_len = self.total_len.wrapping_add(data.len() as u64);
        let mut start = 0usize;
        if self.buffered > 0 {
            let take = core::cmp::min(64 - self.buffered, data.len());
            self.block[self.buffered..self.buffered + take].copy_from_slice(&data[..take]);
            self.buffered += take;
            start = take;
            if self.buffered == 64 {
                let block = self.block;
                self.compress(&block);
                self.buffered = 0;
            }
        }
        let mut rest = &data[start..];
        while rest.len() >= 64 {
            let (head, tail) = rest.split_at(64);
            let mut block = [0u8; 64];
            block.copy_from_slice(head);
            self.compress(&block);
            rest = tail;
        }
        if !rest.is_empty() {
            let len = rest.len();
            self.block[..len].copy_from_slice(rest);
            self.buffered = len;
        }
    }

    /// Appends the padding and returns the 32-byte digest.
    pub fn finish(mut self) -> [u8; 32] {
        let bit_len = self.total_len.wrapping_mul(8).to_be_bytes();
        self.update(&[0x80]);
        while self.buffered != 56 {
            self.update(&[0u8]);
        }
        self.update(&bit_len);
        let mut out = [0u8; 32];
        for (word, slot) in self.state.iter().zip(out.chunks_exact_mut(4)) {
            slot.copy_from_slice(&word.to_be_bytes());
        }
        out
    }

    fn compress(&mut self, block: &[u8; 64]) {
        let mut w = [0u32; 64];
        for t in 0..16 {
            w[t] = u32::from_be_bytes([
                block[t * 4],
                block[t * 4 + 1],
                block[t * 4 + 2],
                block[t * 4 + 3],
            ]);
        }
        for t in 16..64 {
            let s0 = w[t - 15].rotate_right(7) ^ w[t - 15].rotate_right(18) ^ (w[t - 15] >> 3);
            let s1 = w[t - 2].rotate_right(17) ^ w[t - 2].rotate_right(19) ^ (w[t - 2] >> 10);
            w[t] = w[t - 16]
                .wrapping_add(s0)
                .wrapping_add(w[t - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = self.state;
        for t in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[t])
                .wrapping_add(w[t]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        for (value, slot) in [a, b, c, d, e, f, g, h]
            .into_iter()
            .zip(self.state.iter_mut())
        {
            *slot = slot.wrapping_add(value);
        }
    }
}

/// Lowercase hex of the SHA-256 digest of `input`.
pub fn hex_digest(input: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input);
    hex_lower(&hasher.finish())
}

/// Lowercase hex encoding, assembled without format macros so ledger
/// digests are byte-stable everywhere the tool runs.
pub fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fips_180_4_example_digests() {
        // The FIPS 180-4 example messages: empty, "abc", the 56-byte
        // two-block "abcde…nopq", and the 1,000,000 × 'a' streaming
        // vector. The digests below are the FIPS 180-4 / NIST CAVP
        // values, cross-checked against the system `sha256sum` (the
        // 56-byte vector is `sha256sum`-verified: 248d6a61…19db06c1;
        // the earlier test text carried a corrupted copy of this
        // digest). This pins the algorithm in-crate so the unit test
        // needs no oracle.
        assert_eq!(
            hex_digest(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            hex_digest(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            hex_digest(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        let million = vec![b'a'; 1_000_000];
        assert_eq!(
            hex_digest(&million),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
    }

    #[test]
    fn chunked_updates_match_one_shot() {
        // Feed 1 MiB in 1-byte, 10-byte and 100_000-byte chunks and
        // compare against the one-shot digest, proving the block buffer
        // carries across chunk boundaries in both directions.
        let million = vec![b'a'; 1_048_576];
        let reference = hex_digest(&million);
        assert_eq!(reference.len(), 64);
        for chunk in [1usize, 10, 100_000] {
            let mut hasher = Sha256::new();
            for part in million.chunks(chunk) {
                hasher.update(part);
            }
            assert_eq!(hex_lower(&hasher.finish()), reference, "chunk {chunk}");
        }
    }
}
