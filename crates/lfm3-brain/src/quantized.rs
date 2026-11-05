// Raw Q8_0 little-endian v1 block reader (partial, unauthenticated).
// Contract: docs/workbench/LFM3-QUANTIZED-CONTRACT.md (root-reviewed).
// Layout frozen from ggml-common.h L251-256 / ggml-quants.c L553-565.
// This is a borrowed-payload reader only: it does NOT authenticate artifacts
// and does NOT make ArtifactFormat::Quantized acceptable.

pub const Q80_BLOCK_SIZE: usize = 32;
pub const Q80_BLOCK_BYTES: usize = 34; // 2 LE f16 scale bytes + 32 i8 quants.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Q80Version {
    V1,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Q80Error {
    WrongVersionTag(u8),
    RankTooLarge {
        rank: usize,
        max_rank: usize,
    },
    RankZero,
    DimensionTooLarge {
        dim: usize,
        max_dim: usize,
    },
    DimensionZero {
        index: usize,
    },
    LastDimNotMultiple {
        last_dim: usize,
    },
    ElementCountOverflow,
    TooManyElements {
        elements: usize,
        max_elements: usize,
    },
    ByteProductOverflow,
    PayloadTooShort {
        needed: usize,
        got: usize,
    },
    PayloadTooLong {
        needed: usize,
        got: usize,
    },
    ByteLimitExceeded {
        bytes: usize,
        max_bytes: usize,
    },
    NonFiniteScale,
    IndexOutOfRange {
        index: usize,
        len: usize,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Q80Limits {
    pub max_rank: usize,
    pub max_dim: usize,
    pub max_elements: usize,
    pub max_bytes: usize,
}

impl Q80Limits {
    pub const fn new(
        max_rank: usize,
        max_dim: usize,
        max_elements: usize,
        max_bytes: usize,
    ) -> Self {
        Self {
            max_rank,
            max_dim,
            max_elements,
            max_bytes,
        }
    }
}

/// Validated borrowed view. Fields are private so no construction path can
/// bypass validation; access is through read-only getters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Q80View<'a> {
    version: Q80Version,
    payload: &'a [u8],
    shape: &'a [usize],
    limits: Q80Limits,
}

impl<'a> Q80View<'a> {
    pub fn new(
        version: Q80Version,
        payload: &'a [u8],
        shape: &'a [usize],
        limits: Q80Limits,
    ) -> Result<Self, Q80Error> {
        let v = Self {
            version,
            payload,
            shape,
            limits,
        };
        v.validate()?;
        Ok(v)
    }

    pub fn from_tag(
        tag: u8,
        payload: &'a [u8],
        shape: &'a [usize],
        limits: Q80Limits,
    ) -> Result<Self, Q80Error> {
        let version = match tag {
            1 => Q80Version::V1,
            other => return Err(Q80Error::WrongVersionTag(other)),
        };
        Self::new(version, payload, shape, limits)
    }

    pub fn version(&self) -> Q80Version {
        self.version
    }

    pub fn payload(&self) -> &'a [u8] {
        self.payload
    }

    pub fn shape(&self) -> &'a [usize] {
        self.shape
    }

    pub fn limits(&self) -> Q80Limits {
        self.limits
    }

    /// Exact number of complete blocks implied by the validated shape.
    pub fn block_count(&self) -> usize {
        // Invariant: shape product is a multiple of 32 and bytes fit payload.
        self.payload.len() / Q80_BLOCK_BYTES
    }

    pub fn element_count(&self) -> Result<usize, Q80Error> {
        product_checked(self.shape, self.limits.max_elements)
    }

    pub fn scale_bits(&self, block: usize) -> Result<u16, Q80Error> {
        let off = self.block_off(block)?;
        Ok(u16::from_le_bytes([
            self.payload[off],
            self.payload[off + 1],
        ]))
    }

    pub fn scale_f32(&self, block: usize) -> Result<f32, Q80Error> {
        let bits = self.scale_bits(block)?;
        match f16_to_f32(bits) {
            Some(f) if f.is_finite() => Ok(f),
            _ => Err(Q80Error::NonFiniteScale),
        }
    }

    pub fn quant(&self, block: usize, lane: usize) -> Result<i8, Q80Error> {
        if lane >= Q80_BLOCK_SIZE {
            return Err(Q80Error::IndexOutOfRange {
                index: lane,
                len: Q80_BLOCK_SIZE,
            });
        }
        let off = self.block_off(block)?;
        Ok(self.payload[off + 2 + lane] as i8)
    }

    pub fn decode_block(&self, block: usize) -> Result<[f32; Q80_BLOCK_SIZE], Q80Error> {
        let scale = self.scale_f32(block)?;
        let off = self.block_off(block)?;
        let mut out = [0.0f32; Q80_BLOCK_SIZE];
        for (i, slot) in out.iter_mut().enumerate() {
            *slot = scale * (self.payload[off + 2 + i] as i8) as f32;
        }
        Ok(out)
    }

    pub fn decode_element(&self, index: usize) -> Result<f32, Q80Error> {
        let n = self.element_count()?;
        if index >= n {
            return Err(Q80Error::IndexOutOfRange { index, len: n });
        }
        let decoded = self.decode_block(index / Q80_BLOCK_SIZE)?;
        Ok(decoded[index % Q80_BLOCK_SIZE])
    }

    fn block_off(&self, block: usize) -> Result<usize, Q80Error> {
        let nb = self.block_count();
        if block >= nb {
            return Err(Q80Error::IndexOutOfRange {
                index: block,
                len: nb,
            });
        }
        Ok(block * Q80_BLOCK_BYTES)
    }

    fn validate(&self) -> Result<(), Q80Error> {
        if self.version != Q80Version::V1 {
            return Err(Q80Error::WrongVersionTag(0));
        }
        let rank = self.shape.len();
        if rank == 0 {
            return Err(Q80Error::RankZero);
        }
        if rank > self.limits.max_rank {
            return Err(Q80Error::RankTooLarge {
                rank,
                max_rank: self.limits.max_rank,
            });
        }
        for (i, &d) in self.shape.iter().enumerate() {
            if d == 0 {
                return Err(Q80Error::DimensionZero { index: i });
            }
            if d > self.limits.max_dim {
                return Err(Q80Error::DimensionTooLarge {
                    dim: d,
                    max_dim: self.limits.max_dim,
                });
            }
        }
        let last = *self.shape.last().unwrap();
        if last % Q80_BLOCK_SIZE != 0 {
            return Err(Q80Error::LastDimNotMultiple { last_dim: last });
        }
        // Checked byte multiplication FIRST so absurd shapes are rejected
        // before element-limit comparison.
        let mut blocks: usize = 1;
        for &d in self.shape {
            blocks = match blocks.checked_mul(d) {
                Some(v) => v,
                None => return Err(Q80Error::ByteProductOverflow),
            };
        }
        blocks /= Q80_BLOCK_SIZE; // exact: last dim is a multiple of 32
        let bytes = match blocks.checked_mul(Q80_BLOCK_BYTES) {
            Some(b) => b,
            None => return Err(Q80Error::ByteProductOverflow),
        };
        if bytes > self.limits.max_bytes {
            return Err(Q80Error::ByteLimitExceeded {
                bytes,
                max_bytes: self.limits.max_bytes,
            });
        }
        product_checked(self.shape, self.limits.max_elements)?;
        // Exact length: truncation AND trailing bytes rejected before scales.
        if self.payload.len() < bytes {
            return Err(Q80Error::PayloadTooShort {
                needed: bytes,
                got: self.payload.len(),
            });
        }
        if self.payload.len() > bytes {
            return Err(Q80Error::PayloadTooLong {
                needed: bytes,
                got: self.payload.len(),
            });
        }
        for b in 0..blocks {
            let bits = self.scale_bits(b)?;
            match f16_to_f32(bits) {
                Some(f) if f.is_finite() => {}
                _ => return Err(Q80Error::NonFiniteScale),
            }
        }
        Ok(())
    }
}

fn product_checked(shape: &[usize], max_elements: usize) -> Result<usize, Q80Error> {
    let mut n: usize = 1;
    for &d in shape {
        n = match n.checked_mul(d) {
            Some(v) => v,
            None => return Err(Q80Error::ElementCountOverflow),
        };
        if n > max_elements {
            return Err(Q80Error::TooManyElements {
                elements: n,
                max_elements,
            });
        }
    }
    Ok(n)
}

/// Pure-Rust IEEE binary16 -> f32 via integer bit operations. Returns None for
/// NaN/Inf encodings (rejected by contract); preserves signed zeros/subnormals.
pub fn f16_to_f32(bits: u16) -> Option<f32> {
    let sign = ((bits >> 15) & 1) as u32;
    let exp = ((bits >> 10) & 0x1f) as u32;
    let frac = (bits & 0x3ff) as u32;
    if exp == 0x1f {
        return None; // Inf or NaN: rejected by contract.
    }
    let out_bits = if exp == 0 {
        if frac == 0 {
            sign << 31
        } else {
            let mut e = 127 - 15 + 1;
            let mut f = frac;
            while f & 0x400 == 0 {
                f <<= 1;
                e -= 1;
            }
            (sign << 31) | (e << 23) | ((f & 0x3ff) << 13)
        }
    } else {
        (sign << 31) | ((exp + 127 - 15) << 23) | (frac << 13)
    };
    Some(f32::from_bits(out_bits))
}
