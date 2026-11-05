//! Integration tests for pinned tensor-artifact identity/provenance validation.
//! Covers: independent published SHA-256 known-answer vectors for the digest
//! parser; a real synthetic safetensors fixture; every identity negative
//! (wrong model/revision/filename/format/size/digest); relabeling resistance;
//! ordering (hash mismatch reported before structural parse failure); limit
//! rejection before hashing; and borrowed views pointing inside caller input.

use lfm3_brain::artifacts::{ArtifactError, ArtifactLimits};
use lfm3_brain::identity::{
    parse_digest, validate_artifact, ArtifactFormat,
    IdentityError::{
        HashMismatch, LimitExceeded, MalformedDigest, SizeMismatch, Structure, UnsafeFilename,
        UnsupportedFormat, WrongFilename, WrongModel, WrongRevision,
    },
    ManifestEntry,
};
use lfm3_brain::{MODEL_ID, SOURCE_REVISION};

// ---------- helpers ----------

/// Build a complete safetensors file: 8-byte LE header length + JSON + payload.
fn st(header_json: &str, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&(header_json.len() as u64).to_le_bytes());
    out.extend_from_slice(header_json.as_bytes());
    out.extend_from_slice(payload);
    out
}

/// Deterministic pseudo-random payload bytes (no external RNG needed).
fn tensor_bytes(len: usize) -> Vec<u8> {
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

fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest: [u8; 32] = Sha256::digest(data).into();
    let mut s = String::with_capacity(64);
    for b in digest {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn limits() -> ArtifactLimits {
    ArtifactLimits {
        max_header_bytes: 4096,
        max_tensors: 16,
        max_rank: 4,
        max_dimension: u64::MAX,
        max_total_bytes: 1 << 20,
    }
}

macro_rules! assert_err {
    ($expr:expr, $pat:pat $(if $guard:expr)?) => {
        match $expr {
            Err($pat) $(if $guard)? => {}
            Err(other) => panic!("expected {}, got Err({other:?})", stringify!($pat)),
            Ok(_) => panic!("expected {}, got Ok", stringify!($pat)),
        }
    };
}

/// Assert rejection with this exact error variant, with a message naming context.
macro_rules! assert_err_msg {
    ($expr:expr, $pat:pat, $msg:expr) => {
        match $expr {
            Err($pat) => {}
            Err(other) => panic!(
                "expected {} ({}), got Err({other:?})",
                stringify!($pat),
                $msg
            ),
            Ok(_) => panic!("expected {} ({}), got Ok", stringify!($pat), $msg),
        }
    };
}

/// A trusted out-of-band entry matching `file` exactly.
fn entry_for(file: &[u8], filename: &str) -> ManifestEntry {
    ManifestEntry {
        model_id: MODEL_ID.to_string(),
        source_revision: SOURCE_REVISION.to_string(),
        filename: filename.to_string(),
        expected_bytes: file.len() as u64,
        expected_sha256: sha256_hex(file),
        format: ArtifactFormat::Safetensors,
    }
}

/// Real synthetic safetensors file with two tensors and real payload bytes.
fn fixture() -> Vec<u8> {
    let w = tensor_bytes(24); // F32 [3,2]
    let b = tensor_bytes(8); // F32 [2]
    let header = r#"{"weight":{"dtype":"F32","shape":[3,2],"data_offsets":[0,24]},"bias":{"dtype":"F32","shape":[2],"data_offsets":[24,32]}}"#;
    let mut file = st(header, &w);
    file.extend_from_slice(&b);
    file
}

/// Independently pinned SHA-256 of `fixture()` (computed offline over the exact
/// 160 bytes: LE header length + this literal header + the LCG payload). The
/// positive path must pass against this literal, not only a test-time hash.
const FIXTURE_SHA256: &str = "2fef6acb44773355afd01c9366ea439cb272c5b6c8235a11275b59d4a848e337";

// ---------- positive path ----------

#[test]
fn accepts_correct_fixture_and_returns_borrowed_artifact() {
    let file = fixture();
    // Trusted entry pinned to the independently computed literal digest.
    let entry = ManifestEntry {
        model_id: MODEL_ID.to_string(),
        source_revision: SOURCE_REVISION.to_string(),
        filename: "model.safetensors".to_string(),
        expected_bytes: file.len() as u64,
        expected_sha256: FIXTURE_SHA256.to_string(),
        format: ArtifactFormat::Safetensors,
    };
    assert_eq!(sha256_hex(&file), FIXTURE_SHA256);
    let art = validate_artifact(&file, "model.safetensors", &entry, &limits())
        .expect("fixture matches its trusted entry");
    assert_eq!(art.tensors.len(), 2);
    assert_eq!(art.tensor("weight").unwrap().shape, vec![3, 2]);
    // The weight view is exactly the first 24 payload bytes.
    assert_eq!(art.tensor("weight").unwrap().bytes, &art.payload[..24]);
    assert_eq!(art.tensor("bias").unwrap().bytes, &art.payload[24..32]);
}

#[test]
fn artifact_views_point_inside_caller_input() {
    let file = fixture();
    let entry = entry_for(&file, "model.safetensors");
    let art = validate_artifact(&file, "model.safetensors", &entry, &limits()).unwrap();
    let base = file.as_ptr() as usize;
    let end = base + file.len();
    for t in &art.tensors {
        let p = t.bytes.as_ptr() as usize;
        assert!(p >= base && p + t.bytes.len() <= end, "view not borrowed");
    }
    let p = art.payload.as_ptr() as usize;
    assert!(p >= base && p + art.payload.len() <= end);
}

// ---------- identity negatives ----------

#[test]
fn rejects_wrong_model_id() {
    let file = fixture();
    let mut entry = entry_for(&file, "model.safetensors");
    entry.model_id = "LiquidAI/LFM3-1.7B-evil".into();
    assert_err!(
        validate_artifact(&file, "model.safetensors", &entry, &limits()),
        WrongModel(m) if m == "LiquidAI/LFM3-1.7B-evil"
    );
}

#[test]
fn rejects_wrong_source_revision() {
    let file = fixture();
    let mut entry = entry_for(&file, "model.safetensors");
    entry.source_revision = "0000000000000000000000000000000000000000".into();
    assert_err!(
        validate_artifact(&file, "model.safetensors", &entry, &limits()),
        WrongRevision(_)
    );
}

#[test]
fn rejects_requested_filename_not_matching_entry() {
    let file = fixture();
    let entry = entry_for(&file, "model.safetensors");
    assert_err!(
        validate_artifact(&file, "other.safetensors", &entry, &limits()),
        WrongFilename
    );
}

#[test]
fn rejects_unsafe_entry_filenames() {
    let file = fixture();
    for bad in [
        "/etc/model.safetensors",
        "dir/model.safetensors",
        "..\\model.safetensors",
        "..",
        ".",
        ".safetensors",
        "model.safetensors\0",
        "mod\tel.safetensors",
        "mod el.safetensors",
        "model\n.safetensors",
        "model.bin",
        "",
    ] {
        let mut entry = entry_for(&file, "model.safetensors");
        entry.filename = bad.to_string();
        assert_err_msg!(
            validate_artifact(&file, bad, &entry, &limits()),
            UnsafeFilename,
            format!("filename {bad:?} must be rejected")
        );
    }
}

#[test]
fn rejects_quantized_declaration_explicitly() {
    let file = fixture();
    let mut entry = entry_for(&file, "model.safetensors");
    entry.format = ArtifactFormat::Quantized;
    assert_err!(
        validate_artifact(&file, "model.safetensors", &entry, &limits()),
        UnsupportedFormat(ArtifactFormat::Quantized)
    );
}

#[test]
fn rejects_size_mismatch_before_hashing() {
    let file = fixture();
    let mut entry = entry_for(&file, "model.safetensors");
    entry.expected_bytes = file.len() as u64 - 1;
    assert_err!(
        validate_artifact(&file, "model.safetensors", &entry, &limits()),
        SizeMismatch { expected, actual } if expected as usize == file.len() - 1 && actual as usize == file.len()
    );
    // Truncated bytes with correct expected length also fail on size.
    let short = &file[..file.len() - 1];
    assert_err!(
        validate_artifact(
            short,
            "model.safetensors",
            &entry_for(&file, "model.safetensors"),
            &limits()
        ),
        SizeMismatch { .. }
    );
}

#[test]
fn rejects_one_byte_tamper_via_hash() {
    let mut file = fixture();
    let entry = entry_for(&file, "model.safetensors"); // digest of clean bytes
    let last = file.len() - 1;
    file[last] ^= 0x01; // one-byte tamper, same length
    assert_err!(
        validate_artifact(&file, "model.safetensors", &entry, &limits()),
        HashMismatch
    );
}

#[test]
fn rejects_wrong_expected_hash_for_clean_bytes() {
    let file = fixture();
    let mut entry = entry_for(&file, "model.safetensors");
    let mut chars: Vec<char> = entry.expected_sha256.chars().collect();
    chars[0] = if chars[0] == 'a' { 'b' } else { 'a' };
    entry.expected_sha256 = chars.into_iter().collect();
    assert_err!(
        validate_artifact(&file, "model.safetensors", &entry, &limits()),
        HashMismatch
    );
}

#[test]
fn requested_name_must_match_trusted_entry_even_for_identical_bytes() {
    let file = fixture();
    let entry = entry_for(&file, "model.safetensors");
    // Relabeling the request to a different (safe) name fails on filename.
    assert_err!(
        validate_artifact(&file, "weights.safetensors", &entry, &limits()),
        WrongFilename
    );
    // And a trusted entry for a DIFFERENT name never matches this request.
    let mut other = entry.clone();
    other.filename = "weights.safetensors".into();
    assert_err!(
        validate_artifact(&file, "model.safetensors", &other, &limits()),
        WrongFilename
    );
}

// ---------- ordering: identity before structure ----------

#[test]
fn wrong_hash_with_malformed_json_reports_hash_mismatch_first() {
    // Structurally invalid header AND wrong digest: hash mismatch must win.
    let header = r#"{"weight":{"dtype":"F32""shape":[2],"data_offsets":[0,8]}}"#; // broken JSON
    let file = st(header, &tensor_bytes(8));
    let mut entry = entry_for(&file, "model.safetensors");
    entry.expected_sha256 = sha256_hex(b"something else entirely");
    assert_err!(
        validate_artifact(&file, "model.safetensors", &entry, &limits()),
        HashMismatch
    );
}

#[test]
fn correctly_hashed_but_structurally_invalid_still_fails() {
    // Header claims an offset range beyond the payload: identity passes,
    // structure must reject.
    let payload = tensor_bytes(8);
    let header = r#"{"weight":{"dtype":"F32","shape":[4],"data_offsets":[0,16]}}"#;
    let file = st(header, &payload);
    let entry = entry_for(&file, "model.safetensors");
    assert_err!(
        validate_artifact(&file, "model.safetensors", &entry, &limits()),
        Structure(ArtifactError::OffsetOutOfRange)
    );
}

#[test]
fn rejects_exceeding_caller_limits_before_hashing() {
    let file = fixture();
    let entry = entry_for(&file, "model.safetensors");
    let tiny = ArtifactLimits {
        max_total_bytes: (file.len() as u64) - 1,
        ..limits()
    };
    assert_err!(
        validate_artifact(&file, "model.safetensors", &entry, &tiny),
        LimitExceeded
    );
    // Even with a WRONG digest, the limit error comes first (no hashing).
    let mut entry2 = entry.clone();
    entry2.expected_sha256 = "0".repeat(64);
    assert_err!(
        validate_artifact(&file, "model.safetensors", &entry2, &tiny),
        LimitExceeded
    );
}

#[test]
fn malformed_expected_digest_rejected_before_hashing() {
    let file = fixture();
    let mut entry = entry_for(&file, "model.safetensors");
    entry.expected_sha256 = "not-hex".into();
    assert_err!(
        validate_artifact(&file, "model.safetensors", &entry, &limits()),
        MalformedDigest
    );
}

#[path = "identity/digest.rs"]
mod digest;
#[path = "identity/resources.rs"]
mod resources;
