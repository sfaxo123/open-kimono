// Shared Rust-only fixture helpers for the Q8_0 reader contract tests.

use lfm3_brain::artifacts::ArtifactLimits;
use lfm3_brain::quantized::{Q80Limits, Q80_BLOCK_SIZE};
use lfm3_brain::{MODEL_ID, SOURCE_REVISION};

pub fn lim() -> Q80Limits {
    Q80Limits::new(4, 4096, 1_000_000, 4_000_000)
}

pub fn art_limits() -> ArtifactLimits {
    ArtifactLimits {
        max_header_bytes: 4096,
        max_tensors: 16,
        max_rank: 4,
        max_dimension: u64::MAX,
        max_total_bytes: 1 << 20,
    }
}

/// Independent binary16 reference via exact dyadic integer arithmetic: the
/// value is (mantissa * 2^shift) with mantissa/shift derived from the fields,
/// evaluated by repeated doubling/halving only — no powi, no bit
/// reassembly of an f32 word (that is the production path under test).
/// Returns None for NaN/Inf encodings.
pub fn half_ref(bits: u16) -> Option<f64> {
    let negative = bits >> 15 == 1;
    let exp = ((bits >> 10) & 0x1f) as i32;
    let frac = (bits & 0x3ff) as u64;
    if exp == 31 {
        return None; // Inf (frac==0) or NaN (frac!=0): both rejected.
    }
    // Exact dyadic value: mantissa * 2^shift, with shift in [-24, 16].
    let (mantissa, shift) = if exp == 0 {
        (frac, -24) // subnormal or zero: frac * 2^-24
    } else {
        (frac | 0x400, exp - 25) // normal: (1024+frac) * 2^(exp-25)
    };
    let mut v = mantissa as f64;
    if shift > 0 {
        for _ in 0..shift {
            v *= 2.0;
        }
    } else {
        for _ in 0..(-shift) {
            v /= 2.0;
        }
    }
    Some(if negative { -v } else { v })
}

pub fn i8_bytes(q: &[i8; Q80_BLOCK_SIZE]) -> Vec<u8> {
    q.iter().map(|x| *x as u8).collect()
}

pub fn block(scale_bits: u16, quants: [i8; Q80_BLOCK_SIZE]) -> Vec<u8> {
    let mut v = scale_bits.to_le_bytes().to_vec();
    v.extend_from_slice(&i8_bytes(&quants));
    v
}

/// Minimal valid safetensors file for the identity-rejection test.
pub fn st_file() -> Vec<u8> {
    let header_json = r#"{"tensor":{"dtype":"F32","shape":[4],"data_offsets":[0,16]}}"#;
    let mut out = Vec::new();
    out.extend_from_slice(&(header_json.len() as u64).to_le_bytes());
    out.extend_from_slice(header_json.as_bytes());
    out.extend_from_slice(&[0u8; 16]);
    out
}

/// A manifest entry matching `file` exactly. This is a SELF-HASHED integrity
/// fixture only — it proves the parser accepts bytes consistent with their own
/// digest; it is NOT production trusted provenance (no out-of-band trust).
pub fn entry_for(file: &[u8], filename: &str) -> lfm3_brain::identity::ManifestEntry {
    use sha2::{Digest, Sha256};
    let digest: [u8; 32] = Sha256::digest(file).into();
    let mut s = String::with_capacity(64);
    for b in digest {
        s.push_str(&format!("{b:02x}"));
    }
    lfm3_brain::identity::ManifestEntry {
        model_id: MODEL_ID.to_string(),
        source_revision: SOURCE_REVISION.to_string(),
        filename: filename.to_string(),
        expected_bytes: file.len() as u64,
        expected_sha256: s,
        format: lfm3_brain::identity::ArtifactFormat::Safetensors,
    }
}
