//! Typed base-proof + quantized-identity binding for Q8_0 artifacts.
//! Contract: docs/workbench/LFM3-QUANTIZED-CONTRACT.md "Next integration packet".
//!
//! `ValidatedBase` is a private-field proof obtainable ONLY by running the
//! accepted `identity::validate_artifact` over the ACTUAL base bytes against a
//! trusted `ManifestEntry`. It retains the borrowed validated artifact, the
//! immutable borrowed entry (which cannot mutate through the proof), and a
//! strict 64-lowercase-hex out-of-band base-manifest identity. A plain
//! `Artifact` or caller-made tuple is not a substitute.
//!
//! `validate_quantized` binds a trusted `QuantizedEntry` to that proof: exact
//! model/revision, safe `.q8` basename, tensor name/shape equal to the real
//! named base tensor (dtype BF16/F16/F32 only), wire tag 1, base links equal
//! the validated proof, recipe pinned to `RECIPE_ID`/`RECIPE_REVISION`. All
//! manifest strings are bounded and every digest/identity field parsed strict
//! BEFORE any hashing or error-string copy; rank/dim/element/byte policy runs
//! before hashing; the payload hash is verified BEFORE Q80View scale
//! interpretation. Reuses accepted parsing/hash logic — no second parser.
//! Manifest identities are caller trust preconditions, NOT signature, secure
//! file-opening, or conversion-correctness proofs.

use crate::artifacts::{Artifact, ArtifactLimits};
use crate::identity::{parse_digest, validate_artifact, ManifestEntry};
use crate::quantized::{Q80Limits, Q80View};
use crate::SOURCE_REVISION;

#[path = "quantized_identity_types.rs"]
mod types;
use crate::identity::{MAX_FILENAME_BYTES, MAX_MODEL_ID_BYTES, MAX_REVISION_BYTES};
pub use types::{
    QIdError, QuantizedEntry, ValidatedQuantized, MAX_RECIPE_ID_BYTES, MAX_RECIPE_REVISION_BYTES,
    MAX_TENSOR_NAME_BYTES, RECIPE_ID, RECIPE_REVISION,
};

/// Base tensor dtypes the Q8_0 recipe is defined over. Anything else (including
/// quantized dtypes) is rejected — never a fallback.
const SUPPORTED_BASE_DTYPES: [&str; 3] = ["BF16", "F16", "F32"];

/// Private-field proof that ACTUAL base bytes passed the accepted identity +
/// structural validation against a trusted manifest entry. Constructible only
/// via `ValidatedBase::new`; fields are private and immutable through the proof.
#[derive(Debug)]
pub struct ValidatedBase<'a> {
    artifact: Artifact<'a>,
    entry: &'a ManifestEntry,
    identity_hex: String,
}

impl<'a> ValidatedBase<'a> {
    /// Run `identity::validate_artifact` on the actual base bytes against
    /// `entry`, then bind the strict out-of-band base-manifest identity. On any
    /// failure no proof exists — failed base bytes cannot produce one.
    pub fn new(
        input: &'a [u8],
        requested_filename: &str,
        entry: &'a ManifestEntry,
        limits: &ArtifactLimits,
        base_manifest_identity: &str,
    ) -> Result<Self, QIdError> {
        // Cheap strict syntax FIRST: a malformed out-of-band identity is
        // rejected before the O(n) base hash runs.
        if base_manifest_identity.len() > MAX_FILENAME_BYTES {
            return Err(QIdError::ManifestStringTooLong(
                "base_manifest_identity",
                base_manifest_identity.len(),
            ));
        }
        parse_digest(base_manifest_identity)?;
        let artifact =
            validate_artifact(input, requested_filename, entry, limits).map_err(QIdError::from)?;
        Ok(Self {
            artifact,
            entry,
            identity_hex: base_manifest_identity.to_string(),
        })
    }

    /// Borrowed structurally validated base artifact.
    pub fn artifact(&self) -> &Artifact<'a> {
        &self.artifact
    }
    /// The immutable trusted manifest entry the proof was made against.
    pub fn entry(&self) -> &ManifestEntry {
        self.entry
    }
    /// Strict 64 lowercase hex out-of-band base-manifest identity.
    pub fn manifest_identity(&self) -> &str {
        &self.identity_hex
    }
}

/// Validate actual Q8_0 payload bytes against a trusted `QuantizedEntry` bound
/// to a `ValidatedBase` proof, under caller-supplied artifact/Q8_0 limits.
/// Ordering: manifest-string caps → exact identity/recipe pins → filename and
/// wire-tag links → base-link equality vs the proof (exact, nothing else) →
/// strict digest/identity spellings → real named base tensor with equal shape
/// and float dtype → rank/dim/element/byte policy incl. row alignment and
/// checked shape-to-bytes policy → payload hash → Q80View scale reading.
pub fn validate_quantized<'a>(
    payload: &'a [u8],
    requested_filename: &str,
    entry: &'a QuantizedEntry,
    base: &ValidatedBase<'_>,
    artifact_limits: &ArtifactLimits,
    q80_limits: &Q80Limits,
) -> Result<ValidatedQuantized<'a>, QIdError> {
    // --- 1. manifest string caps FIRST (before compares/error copies/hash) --
    bound(entry.model_id.len(), MAX_MODEL_ID_BYTES, "model_id")?;
    bound(
        entry.source_revision.len(),
        MAX_REVISION_BYTES,
        "source_revision",
    )?;
    bound(entry.filename.len(), MAX_FILENAME_BYTES, "filename")?;
    bound(
        requested_filename.len(),
        MAX_FILENAME_BYTES,
        "requested_filename",
    )?;
    bound(
        entry.tensor_name.len(),
        MAX_TENSOR_NAME_BYTES,
        "tensor_name",
    )?;
    for (label, s) in [
        ("manifest_identity", &entry.manifest_identity),
        ("base_manifest_identity", &entry.base_manifest_identity),
        ("base_filename", &entry.base_filename),
        ("base_sha256", &entry.base_sha256),
        ("expected_sha256", &entry.expected_sha256),
    ] {
        bound(s.len(), MAX_FILENAME_BYTES, label)?;
    }
    bound(entry.recipe_id.len(), MAX_RECIPE_ID_BYTES, "recipe_id")?;
    bound(
        entry.recipe_revision.len(),
        MAX_RECIPE_REVISION_BYTES,
        "recipe_revision",
    )?;

    // --- 2. exact identity and recipe pins ----------------------------------
    if entry.model_id != crate::MODEL_ID {
        return Err(QIdError::WrongModel(entry.model_id.clone()));
    }
    if entry.source_revision != SOURCE_REVISION {
        return Err(QIdError::WrongRevision(entry.source_revision.clone()));
    }
    if entry.recipe_id != RECIPE_ID {
        return Err(QIdError::WrongRecipeId);
    }
    if entry.recipe_revision != RECIPE_REVISION {
        return Err(QIdError::WrongRecipeRevision);
    }
    check_q8_filename(&entry.filename)?;
    if requested_filename != entry.filename {
        return Err(QIdError::WrongFilename);
    }
    if entry.wire_tag != 1 {
        return Err(QIdError::WrongWireTag(entry.wire_tag));
    }

    // --- 3. base links must equal the validated proof, exactly ---------------
    let proof_entry = base.entry();
    if entry.base_filename != proof_entry.filename
        || entry.base_expected_bytes != proof_entry.expected_bytes
        || entry.base_sha256 != proof_entry.expected_sha256
        || entry.base_manifest_identity != base.manifest_identity()
    {
        return Err(QIdError::BaseLinkMismatch);
    }

    // --- 4. strict spellings for ALL digest/identity fields ------------------
    let expected_digest = parse_digest(&entry.expected_sha256)?;
    parse_digest(&entry.manifest_identity)?;
    parse_digest(&entry.base_sha256)?;
    // base_manifest_identity re-parsed: cheap, and keeps the invariant local.
    parse_digest(&entry.base_manifest_identity)?;

    // --- 5. real named base tensor: exists, equal shape, float dtype --------
    let base_tensor = match base.artifact().tensor(&entry.tensor_name) {
        Some(t) => t,
        None => return Err(QIdError::WrongTensorName),
    };
    if base_tensor.shape.len() != entry.shape.len() {
        return Err(QIdError::ShapeMismatch);
    }
    for (&want, &have) in entry.shape.iter().zip(base_tensor.shape.iter()) {
        let Ok(wide) = u64::try_from(want) else {
            return Err(QIdError::ShapeMismatch);
        };
        if wide != have {
            return Err(QIdError::ShapeMismatch);
        }
    }
    if !SUPPORTED_BASE_DTYPES.contains(&base_tensor.dtype.as_str()) {
        return Err(QIdError::UnsupportedBaseDtype(base_tensor.dtype.clone()));
    }

    // --- 6. length + resource policy BEFORE hashing --------------------------
    let actual = payload.len() as u64;
    if actual != entry.expected_bytes {
        return Err(QIdError::SizeMismatch {
            expected: entry.expected_bytes,
            actual,
        });
    }
    if actual > artifact_limits.max_total_bytes || actual > q80_limits.max_bytes as u64 {
        return Err(QIdError::LimitExceeded);
    }
    let rank = entry.shape.len();
    if rank == 0 || rank > q80_limits.max_rank {
        return Err(QIdError::LimitExceeded);
    }
    let mut elements: u64 = 1;
    for &d in &entry.shape {
        let Ok(wide) = u64::try_from(d) else {
            return Err(QIdError::LimitExceeded);
        };
        if d == 0 || wide > q80_limits.max_dim as u64 {
            return Err(QIdError::LimitExceeded);
        }
        elements = match elements.checked_mul(wide) {
            Some(v) => v,
            None => return Err(QIdError::LimitExceeded),
        };
        if elements > q80_limits.max_elements as u64 {
            return Err(QIdError::LimitExceeded);
        }
    }
    // Row alignment: Q8_0 blocks span the innermost row; last dim % 32 == 0.
    if *entry.shape.last().unwrap() % crate::quantized::Q80_BLOCK_SIZE != 0 {
        return Err(QIdError::LimitExceeded);
    }
    // Checked shape-to-bytes policy (blocks * 34) before any hashing.
    let blocks = match elements.checked_div(crate::quantized::Q80_BLOCK_SIZE as u64) {
        Some(v) => v,
        None => return Err(QIdError::LimitExceeded),
    };
    let want_bytes = match blocks.checked_mul(crate::quantized::Q80_BLOCK_BYTES as u64) {
        Some(v) => v,
        None => return Err(QIdError::LimitExceeded),
    };
    if want_bytes != entry.expected_bytes || want_bytes != actual {
        return Err(QIdError::SizeMismatch {
            expected: want_bytes,
            actual,
        });
    }

    // --- 7. payload hash BEFORE any scale interpretation ---------------------
    use sha2::{Digest, Sha256};
    let actual_digest: [u8; 32] = Sha256::digest(payload).into();
    if actual_digest != expected_digest {
        return Err(QIdError::HashMismatch);
    }

    // --- 8. Q8_0 structure via the accepted scale-reading reader -------------
    let view = Q80View::from_tag(entry.wire_tag, payload, &entry.shape, *q80_limits)
        .map_err(QIdError::Structure)?;

    Ok(ValidatedQuantized::new(
        view,
        &entry.shape,
        &entry.filename,
        &entry.tensor_name,
        &entry.manifest_identity,
    ))
}

fn bound(len: usize, max: usize, field: &'static str) -> Result<(), QIdError> {
    if len > max {
        return Err(QIdError::ManifestStringTooLong(field, len));
    }
    Ok(())
}

/// Filename policy for `.q8` files: simple non-empty ASCII basename ending
/// `.q8`, no path separators, no NUL/control/whitespace, no DEL or high bytes.
fn check_q8_filename(name: &str) -> Result<(), QIdError> {
    if name.is_empty() || !name.ends_with(".q8") || name.len() == ".q8".len() {
        return Err(QIdError::UnsafeFilename);
    }
    for &b in name.as_bytes() {
        if b <= 0x20 || b >= 0x7f || b == b'/' || b == b'\\' {
            return Err(QIdError::UnsafeFilename);
        }
    }
    Ok(())
}
