//! Pinned tensor-artifact identity/provenance validation feeding `parse_artifact`.
//!
//! The caller supplies a TRUSTED, OUT-OF-BAND manifest entry (published with the
//! model, not derived from the file at hand). This module checks that the bytes
//! in memory match that entry — exact model id and source revision (crate
//! `MODEL_ID`/`SOURCE_REVISION`), requested filename equal to the entry name,
//! declared format, expected byte length, and expected SHA-256 digest — BEFORE
//! any structural JSON parsing. Only after BOTH identity and structure validate
//! is a borrowed, structurally validated `Artifact` returned; its slices point
//! inside the caller-owned input buffer.
//!
//! This parser performs NO filesystem I/O. Manifest trust/signature verification
//! and secure file opening remain EXTERNAL contracts and are NOT proven here.
//! A caller self-asserted fingerprint over the same bytes is never a provenance
//! proof: only comparison against the out-of-band expected digest counts.
//! Quantized artifact declarations are rejected explicitly (support is a later
//! gate). Hashing streams the caller's byte slice via `Sha256::digest`; it does
//! not copy or allocate the payload.

use crate::{MODEL_ID, SOURCE_REVISION};
use sha2::{Digest, Sha256};

use super::artifacts::{parse_artifact, Artifact, ArtifactError, ArtifactLimits};

/// Declared artifact format. Typed enum: no stringly-typed fallback. Only the
/// byte-aligned safetensors container is supported by this gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactFormat {
    /// Plain byte-aligned safetensors (the structural reader's format).
    Safetensors,
    /// Quantized artifact — declared but NOT yet supported; rejected explicitly.
    Quantized,
}

/// Strict manifest-string policy: oversized entries are rejected outright, and
/// rejection never clones the hostile string (the error carries only its byte
/// length). These are explicit small constants, not derived from input.
pub const MAX_FILENAME_BYTES: usize = 255;
pub const MAX_MODEL_ID_BYTES: usize = 128;
pub const MAX_REVISION_BYTES: usize = 40;

/// Identity/provenance/structural failure categories. Each variant names the
/// violated contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdentityError {
    /// Entry model id does not equal crate `MODEL_ID` exactly.
    WrongModel(String),
    /// Entry source revision does not equal crate `SOURCE_REVISION` exactly.
    WrongRevision(String),
    /// Requested filename differs from the entry's artifact filename.
    WrongFilename,
    /// Entry filename violates the simple-basename `.safetensors` policy.
    UnsafeFilename,
    /// A manifest string (model id, revision, or filename) exceeded its strict
    /// length bound; carries the offending byte length, never the bytes.
    ManifestStringTooLong(&'static str, usize),
    /// Declared format is not supported by this parser (e.g. `Quantized`).
    UnsupportedFormat(ArtifactFormat),
    /// Actual byte length differs from the entry's expected length.
    SizeMismatch { expected: u64, actual: u64 },
    /// Actual SHA-256 differs from the entry's expected digest.
    HashMismatch,
    /// Expected digest was not exactly 64 lowercase hex characters.
    MalformedDigest,
    /// A caller `ArtifactLimits` bound would be exceeded; checked before hashing.
    LimitExceeded,
    /// Identity passed but structural parsing failed.
    Structure(ArtifactError),
}

/// One trusted, out-of-band manifest entry for a single tensor artifact file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestEntry {
    /// Model identifier; must equal crate `MODEL_ID` exactly.
    pub model_id: String,
    /// Source revision (commit hash); must equal crate `SOURCE_REVISION` exactly.
    pub source_revision: String,
    /// Safe relative artifact filename: simple basename ending `.safetensors`.
    pub filename: String,
    /// Exact expected file byte length.
    pub expected_bytes: u64,
    /// Expected SHA-256 of the exact file bytes: 64 lowercase hex characters.
    pub expected_sha256: String,
    /// Declared artifact format.
    pub format: ArtifactFormat,
}

/// Validate identity against `entry`, then structure under `limits`, and return
/// a borrowed structurally validated `Artifact` over `input`.
///
/// Ordering is deliberate: cheap identity/filename/format/limit checks run before
/// the O(n) hash; the hash runs before JSON header parsing allocates metadata.
/// `requested_filename` is what the caller asked to load — it must equal
/// `entry.filename`; expected values are never derived from file metadata.
pub fn validate_artifact<'a>(
    input: &'a [u8],
    requested_filename: &str,
    entry: &ManifestEntry,
    limits: &ArtifactLimits,
) -> Result<Artifact<'a>, IdentityError> {
    // --- trusted identity of the manifest entry itself ---------------------
    // Length bounds run FIRST: an oversized hostile string is rejected without
    // comparison work and without cloning its contents into any error.
    if entry.model_id.len() > MAX_MODEL_ID_BYTES {
        return Err(IdentityError::ManifestStringTooLong(
            "model_id",
            entry.model_id.len(),
        ));
    }
    if entry.source_revision.len() > MAX_REVISION_BYTES {
        return Err(IdentityError::ManifestStringTooLong(
            "source_revision",
            entry.source_revision.len(),
        ));
    }
    if entry.filename.len() > MAX_FILENAME_BYTES {
        return Err(IdentityError::ManifestStringTooLong(
            "filename",
            entry.filename.len(),
        ));
    }
    // The requested filename is caller-supplied too: bound it before compare.
    if requested_filename.len() > MAX_FILENAME_BYTES {
        return Err(IdentityError::ManifestStringTooLong(
            "requested_filename",
            requested_filename.len(),
        ));
    }
    if entry.model_id != MODEL_ID {
        return Err(IdentityError::WrongModel(entry.model_id.clone()));
    }
    if entry.source_revision != SOURCE_REVISION {
        return Err(IdentityError::WrongRevision(entry.source_revision.clone()));
    }
    check_filename(&entry.filename)?;
    if requested_filename != entry.filename {
        return Err(IdentityError::WrongFilename);
    }
    match entry.format {
        ArtifactFormat::Safetensors => {}
        other => return Err(IdentityError::UnsupportedFormat(other)),
    }

    // --- expected length vs actual, before hashing --------------------------
    let actual = input.len() as u64;
    if actual != entry.expected_bytes {
        return Err(IdentityError::SizeMismatch {
            expected: entry.expected_bytes,
            actual,
        });
    }
    if (entry.expected_bytes) > limits.max_total_bytes
        || (input.len() as u64) > limits.max_total_bytes
    {
        return Err(IdentityError::LimitExceeded);
    }

    // Cheap resource preflight; count/rank/dimension validation follows the hash.
    if let Some(prefix) = input.get(..8) {
        let header_len = u64::from_le_bytes(prefix.try_into().expect("eight-byte prefix"));
        if header_len > limits.max_header_bytes || header_len.checked_add(8).is_none() {
            return Err(IdentityError::LimitExceeded);
        }
    }

    // --- digest: strict expected spelling, then streaming hash --------------
    let expected = parse_digest(&entry.expected_sha256)?;
    let actual_digest: [u8; 32] = Sha256::digest(input).into();
    if actual_digest != expected {
        return Err(IdentityError::HashMismatch);
    }

    // --- structure, only after identity fully passed ------------------------
    match parse_artifact(input, limits) {
        Ok(artifact) => Ok(artifact),
        Err(e) => Err(IdentityError::Structure(e)),
    }
}

/// Parse a strict expected digest: exactly 64 lowercase hex characters.
/// Uppercase, short/long strings, and non-hex are rejected (`MalformedDigest`).
/// The length check above means an arbitrarily long hostile string is rejected
/// without scanning or cloning it.
pub fn parse_digest(hex: &str) -> Result<[u8; 32], IdentityError> {
    let bytes = hex.as_bytes();
    if bytes.len() != 64 || !hex.is_ascii() {
        return Err(IdentityError::MalformedDigest);
    }
    let mut out = [0u8; 32];
    for (slot, pair) in out.iter_mut().zip(bytes.chunks_exact(2)) {
        let hi = hex_nibble(pair[0]).ok_or(IdentityError::MalformedDigest)?;
        let lo = hex_nibble(pair[1]).ok_or(IdentityError::MalformedDigest)?;
        *slot = (hi << 4) | lo;
    }
    Ok(out)
}

fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        _ => None,
    }
}

/// Filename policy: simple non-empty basename ending `.safetensors`. Rejects
/// absolute paths, `/`, `\`, `.`/`..`, empty stem, and any NUL/control or
/// whitespace character anywhere in the name.
fn check_filename(name: &str) -> Result<(), IdentityError> {
    if name.is_empty() || !name.ends_with(".safetensors") {
        return Err(IdentityError::UnsafeFilename);
    }
    // Empty stem (the bare ".safetensors") is rejected; `.` and `..` never end
    // in `.safetensors` and are caught by the extension check anyway.
    if name.len() == ".safetensors".len() {
        return Err(IdentityError::UnsafeFilename);
    }
    if name.as_bytes().contains(&b'/') || name.as_bytes().contains(&b'\\') {
        return Err(IdentityError::UnsafeFilename);
    }
    for &b in name.as_bytes() {
        // Reject NUL, all control characters (incl. tab/newline/CR), and space.
        if b <= 0x20 || b >= 0x7f {
            return Err(IdentityError::UnsafeFilename);
        }
    }
    Ok(())
}
