// Shared Rust-only fixture helpers for the typed quantized-identity tests.

use lfm3_brain::artifacts::ArtifactLimits;
use lfm3_brain::identity::{ArtifactFormat, ManifestEntry};
use lfm3_brain::quantized::Q80Limits;
use lfm3_brain::quantized_identity::{QuantizedEntry, RECIPE_ID, RECIPE_REVISION};
use lfm3_brain::{MODEL_ID, SOURCE_REVISION};

pub const BASE_NAME: &str = "base.safetensors";
pub const Q8_NAME: &str = "weight.q8";
pub const TENSOR: &str = "weight";
/// Out-of-band base-manifest identity used by every positive-path fixture.
pub const BASE_MANIFEST_ID: &str =
    "1111222233334444555566667777888899990000aaaabbbbccccddddeeeeffff";

pub fn art_limits() -> ArtifactLimits {
    ArtifactLimits {
        max_header_bytes: 4096,
        max_tensors: 16,
        max_rank: 4,
        max_dimension: u64::MAX,
        max_total_bytes: 1 << 20,
    }
}

pub fn q80_limits() -> Q80Limits {
    Q80Limits::new(4, 4096, 1_000_000, 4_000_000)
}

/// Build a complete safetensors file: 8-byte LE header length + JSON + payload.
pub fn st(header_json: &str, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&(header_json.len() as u64).to_le_bytes());
    out.extend_from_slice(header_json.as_bytes());
    out.extend_from_slice(payload);
    out
}

/// Deterministic pseudo-random bytes (LCG, no external RNG).
pub fn lcg_bytes(len: usize) -> Vec<u8> {
    let mut v = Vec::with_capacity(len);
    let mut state: u64 = 0x9E3779B97F4A7C15;
    for _ in 0..len {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        v.push((state >> 33) as u8);
    }
    v
}

pub fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest: [u8; 32] = Sha256::digest(data).into();
    let mut s = String::with_capacity(64);
    for b in digest {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Synthetic Q8_0 v1 payload: `blocks` blocks, scale bits `scale_bits`, then
/// 32 i8 lanes from the LCG stream. Exact length = 34 * blocks.
pub fn q8_payload(blocks: usize, scale_bits: u16) -> Vec<u8> {
    let mut out = Vec::with_capacity(blocks * 34);
    for _ in 0..blocks {
        out.extend_from_slice(&scale_bits.to_le_bytes());
        out.extend_from_slice(&lcg_bytes(32));
    }
    out
}

/// Base artifact: tensor `weight` F32 [rows, 32] (real f32 bytes) plus a second
/// tensor `other` F16 [64] so missing-name negatives have a real file.
pub fn base_file(rows: usize) -> Vec<u8> {
    let w = lcg_bytes(rows * 32 * 4);
    let o = lcg_bytes(64 * 2);
    let header = format!(
        "{{\"weight\":{{\"dtype\":\"F32\",\"shape\":[{rows},32],\"data_offsets\":[0,{}]}},\
         \"other\":{{\"dtype\":\"F16\",\"shape\":[64],\"data_offsets\":[{},{}]}}}}",
        rows * 32 * 4,
        rows * 32 * 4,
        rows * 32 * 4 + 128
    );
    let mut file = st(&header, &w);
    file.extend_from_slice(&o);
    file
}

/// Trusted base manifest entry matching `file` exactly (filename BASE_NAME).
pub fn base_entry(file: &[u8]) -> ManifestEntry {
    ManifestEntry {
        model_id: MODEL_ID.to_string(),
        source_revision: SOURCE_REVISION.to_string(),
        filename: BASE_NAME.to_string(),
        expected_bytes: file.len() as u64,
        expected_sha256: sha256_hex(file),
        format: ArtifactFormat::Safetensors,
    }
}

/// Base manifest entry with the "weight" tensor name (no "other" tensor).
pub fn base_entry_named(file: &[u8]) -> ManifestEntry {
    base_entry(file)
}

/// Trusted quantized entry matching `base_file(1)` + `q8_payload(1, 0x3c00)`.
pub fn q_entry(payload: &[u8]) -> QuantizedEntry {
    QuantizedEntry {
        model_id: MODEL_ID.to_string(),
        source_revision: SOURCE_REVISION.to_string(),
        filename: Q8_NAME.to_string(),
        tensor_name: TENSOR.to_string(),
        shape: vec![1, 32],
        wire_tag: 1,
        expected_bytes: payload.len() as u64,
        expected_sha256: sha256_hex(payload),
        manifest_identity: "0000000000000000000000000000000000000000000000000000000000000001"
            .to_string(),
        base_manifest_identity: BASE_MANIFEST_ID.to_string(),
        base_filename: BASE_NAME.to_string(),
        base_expected_bytes: 0, // set by link_base from the actual base bytes
        base_sha256: String::new(), // set by link_base from the actual base bytes
        recipe_id: RECIPE_ID.to_string(),
        recipe_revision: RECIPE_REVISION.to_string(),
    }
}

/// Fill the base links of `e` from the actual validated base file bytes.
pub fn link_base(e: &mut QuantizedEntry, base_file_bytes: &[u8]) {
    e.base_expected_bytes = base_file_bytes.len() as u64;
    e.base_sha256 = sha256_hex(base_file_bytes);
}

/// The canonical 1-row base file, its Q8_0 payload, and the shared identity.
pub fn base_and_payload() -> (Vec<u8>, Vec<u8>, String) {
    (
        base_file(1),
        q8_payload(1, 0x3c00),
        BASE_MANIFEST_ID.to_string(),
    )
}
