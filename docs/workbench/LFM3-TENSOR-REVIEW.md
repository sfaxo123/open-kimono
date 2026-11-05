# Tensor reader integration review snapshot

Evidence snapshot only, not runtime code or a competing plan. New implementation f03784e, ordering765f92d.
The existing structural safetensors, identity and raw Q8 readers were reviewed incrementally in canonical HANDOFF, with58/80/93 passing suites respectively. They are reused, not rewritten.
This snapshot supplies COMPLETE new production identity-binding code, the independent literal fixture and critical ordering test, real root negative controls and combined test output. Other test bodies and full original worker history/diff are retained under RUST-CONTINUE-20261028T0000-* artifacts.
Scope of0.1.2 is byte-buffer reader APIs with caller-trusted out-of-band typed manifest entries. It does NOT accept manifest authenticity/signatures/acquisition, secure file opening, quantizer accuracy, actual weights or inference; those were explicit external/later contracts, not this implementation. The own manifest ID is a caller-trusted label; changing it to a different well-formed label is intentionally allowed. Base IDs/hash/name/length DO cross-check against an immutable proof made from validated actual base bytes.
Earlier dependency boundaries remain open: sha2 force-soft still has GenericArray/block-buffer unsafe casts and target-specific cpufeatures/libc build graph. No new dependencies/unsafe introduced here. This is not whole-runtime C retirement or leak-freedom.
Root reviewed full source/diffs and fixed the worker's unfinished test length expectation, borrowed wrapper identity instead of cloning, removed unused draft-only fixture helpers, preserved16 tests with0ignored. Exactly one10k-cycle process measured3264KiB RSS/0.13seconds on a34-byte fixture, not exact heap accounting/model fit.
Root source mutations skipping payload hash or base-manifest comparison separately made their real tests RED; source restored and combined suite109PASS/0ignored. Production manifest acquisition and the later pretrained gate remain NOT IMPLEMENTED.

## crates/lfm3-brain/src/quantized_identity.rs

```rust
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

```

## crates/lfm3-brain/src/quantized_identity_types.rs

```rust
//! Typed Q8_0 quantized-entry manifest types and error categories.
//! Contract: docs/workbench/LFM3-QUANTIZED-CONTRACT.md (root-reviewed).
//! Manifest strings are TRUSTED out-of-band caller preconditions — this module
//! proves byte/identity consistency, never signature authenticity, secure file
//! opening, or numerical conversion correctness.

use crate::artifacts::ArtifactError;
use crate::identity::IdentityError;
use crate::quantized::{Q80Error, Q80Version, Q80View};
use crate::MODEL_ID;

/// Algorithm recipe id pinned by the contract: the reference layout algorithm,
/// NOT an attestation that a production Rust quantizer exists or is correct.
pub const RECIPE_ID: &str = "ggml-q8_0-ref";
/// Exact ggml layout-reference commit identifying the recipe (read-only pin).
pub const RECIPE_REVISION: &str = "da8e2a54b919557b816b1c2beb432ccb766f7ce0";

/// Strict manifest-string policy for the typed quantized path. The digest field
/// reuses `identity::MAX_FILENAME_BYTES` (255) as its cap; only a strict 64
/// lowercase-hex spelling passes `identity::parse_digest`.
pub const MAX_TENSOR_NAME_BYTES: usize = 1024;
pub const MAX_RECIPE_ID_BYTES: usize = 128;
pub const MAX_RECIPE_REVISION_BYTES: usize = 64;

/// Identity/binding/structural failure categories for the typed quantized path.
/// String-carrying variants are only ever constructed from strings already
/// bounded by the manifest caps, so error copies stay small.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QIdError {
    /// A manifest string exceeded its strict cap; carries field name and
    /// offending length, never the hostile bytes.
    ManifestStringTooLong(&'static str, usize),
    /// Entry model id does not equal crate `MODEL_ID` exactly.
    WrongModel(String),
    /// Entry source revision does not equal crate `SOURCE_REVISION` exactly.
    WrongRevision(String),
    /// Entry recipe id does not equal `RECIPE_ID` exactly.
    WrongRecipeId,
    /// Recipe revision does not equal `RECIPE_REVISION` exactly.
    WrongRecipeRevision,
    /// Requested filename differs from the entry's filename.
    WrongFilename,
    /// Filename violates the simple-ASCII-basename `.q8` policy.
    UnsafeFilename,
    /// A digest field is not exactly 64 lowercase hex characters.
    MalformedDigest,
    /// Base manifest identity differs from the validated base proof.
    BaseLinkMismatch,
    /// Entry tensor name differs from the requested name or the binding.
    WrongTensorName,
    /// Entry shape differs from the base tensor's actual shape.
    ShapeMismatch,
    /// Base tensor dtype is not one of BF16/F16/F32.
    UnsupportedBaseDtype(String),
    /// Declared wire tag is not the Q8_0 v1 tag (1).
    WrongWireTag(u8),
    /// Actual payload length differs from the entry's expected length.
    SizeMismatch { expected: u64, actual: u64 },
    /// Actual SHA-256 of the payload differs from the entry's digest.
    HashMismatch,
    /// Rank/dimension/element/byte policy would be exceeded; checked before hash.
    LimitExceeded,
    /// Payload hash passed but Q8_0 scale/layout interpretation failed.
    Structure(Q80Error),
    /// The base bytes failed accepted structural parsing (carries the accepted
    /// `ArtifactError` verbatim — never disguised as a Q8_0 failure).
    BaseStructure(ArtifactError),
}

/// Keep the accepted base parser's failures visible through the typed API.
impl From<IdentityError> for QIdError {
    fn from(error: IdentityError) -> Self {
        match error {
            IdentityError::WrongModel(s) => Self::WrongModel(s),
            IdentityError::WrongRevision(s) => Self::WrongRevision(s),
            IdentityError::WrongFilename => Self::WrongFilename,
            IdentityError::UnsafeFilename => Self::UnsafeFilename,
            IdentityError::ManifestStringTooLong(f, n) => Self::ManifestStringTooLong(f, n),
            IdentityError::UnsupportedFormat(_) => Self::BaseLinkMismatch,
            IdentityError::SizeMismatch { expected, actual } => {
                Self::SizeMismatch { expected, actual }
            }
            IdentityError::HashMismatch => Self::HashMismatch,
            IdentityError::MalformedDigest => Self::MalformedDigest,
            IdentityError::LimitExceeded => Self::LimitExceeded,
            IdentityError::Structure(error) => Self::BaseStructure(error),
        }
    }
}

/// Trusted, out-of-band manifest entry for one typed Q8_0 quantized file.
/// Every field is caller-published; none is derived from the bytes at hand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuantizedEntry {
    /// Model id; must equal crate `MODEL_ID` exactly (same pin as base path).
    pub model_id: String,
    /// Source revision; must equal crate `SOURCE_REVISION` exactly.
    pub source_revision: String,
    /// Safe simple basename ending `.q8` (ASCII, no path separators).
    pub filename: String,
    /// Tensor this file quantizes; must exist in the validated base artifact.
    pub tensor_name: String,
    /// Shape; must equal the base tensor's shape exactly (compared as u64).
    pub shape: Vec<usize>,
    /// Wire version tag; must be 1 (Q8_0 v1).
    pub wire_tag: u8,
    /// Exact expected payload byte length.
    pub expected_bytes: u64,
    /// Expected SHA-256 of the exact payload bytes: 64 lowercase hex.
    pub expected_sha256: String,
    /// This file's own manifest identity: strict 64 lowercase hex over its
    /// canonical manifest record. Caller trust precondition, not a proof that
    /// any signature or canonical serialization was verified.
    pub manifest_identity: String,
    /// Base artifact's out-of-band manifest identity (64 lowercase hex); must
    /// equal the validated base proof's identity exactly.
    pub base_manifest_identity: String,
    /// Base artifact filename; must equal the validated base proof's name.
    pub base_filename: String,
    /// Base artifact expected byte length; must equal the proof's length.
    pub base_expected_bytes: u64,
    /// Base artifact digest (64 lowercase hex); must equal the proof's digest.
    pub base_sha256: String,
    /// Algorithm recipe id; must equal `RECIPE_ID` exactly.
    pub recipe_id: String,
    /// Recipe layout-reference revision; must equal `RECIPE_REVISION` exactly.
    pub recipe_revision: String,
}

/// Read-only accessors over a fully validated typed quantized artifact. All
/// data is borrowed from the caller's payload buffer and entry; no dequantized
/// tensor is ever allocated here. The wrapper holds the accepted, validated
/// `Q80View` and exposes it read-only — nothing is re-derived or discarded.
#[derive(Debug)]
pub struct ValidatedQuantized<'a> {
    view: Q80View<'a>,
    shape: &'a [usize],
    filename: &'a str,
    tensor_name: &'a str,
    manifest_identity: &'a str,
}

impl<'a> ValidatedQuantized<'a> {
    pub(super) fn new(
        view: Q80View<'a>,
        shape: &'a [usize],
        filename: &'a str,
        tensor_name: &'a str,
        manifest_identity: &'a str,
    ) -> Self {
        Self {
            view,
            shape,
            filename,
            tensor_name,
            manifest_identity,
        }
    }

    /// The accepted scale-reading Q8_0 view (validated before this wrapper
    /// existed); read-only getter, never a copy or reconstruction.
    pub fn view(&self) -> &Q80View<'a> {
        &self.view
    }
    pub fn version(&self) -> Q80Version {
        self.view.version()
    }
    /// Borrowed validated payload — points inside the caller's input buffer.
    pub fn payload(&self) -> &'a [u8] {
        self.view.payload()
    }
    /// Borrowed validated shape (equal to the base tensor's shape).
    pub fn shape(&self) -> &'a [usize] {
        self.shape
    }
    pub fn filename(&self) -> &str {
        self.filename
    }
    pub fn tensor_name(&self) -> &str {
        self.tensor_name
    }
    pub fn manifest_identity(&self) -> &str {
        self.manifest_identity
    }
    pub fn model_id(&self) -> &'static str {
        MODEL_ID
    }
}

```

## crates/lfm3-brain/tests/quantized_identity/digest_vectors.rs

```rust
// Independent literal digest vectors (positive payload identity) and the
// 10,000-cycle repeated workload over ONE prevalidated base proof.
//
// Pinned anchor: root-retained Rust fixture generator (independent of this
// test file): RUST-CONTINUE-20261028T0000-ROOT-FIXTURE.rs emits exactly 34
// bytes — LE binary16 scale 1.0 (0x3c00) then lanes 0..31 — with .bin digest
// from the unchanged coreutils sha256sum oracle:
//   8694fb0e0737529d0fd84430327afc942b711d41c362b789587d6dddf0c45655

use super::{
    art_limits, base_entry, link_base, q80_limits, q_entry, sha256_hex, BASE_MANIFEST_ID,
    BASE_NAME, Q8_NAME, TENSOR,
};
use lfm3_brain::quantized_identity::{validate_quantized, ValidatedBase};

/// The root fixture bytes reproduced by Rust arithmetic identical to the
/// retained generator: LE 0x3c00 scale, then lanes 0..=31.
fn anchor_payload() -> Vec<u8> {
    let mut v = vec![0x00u8, 0x3c];
    v.extend((0u8..=31).collect::<Vec<u8>>());
    v
}

/// Literal digest pinned from the root-retained fixture .bin (coreutils oracle
/// receipt: `sha256sum RUST-CONTINUE-20261028T0000-ROOT-FIXTURE.bin`).
const ANCHOR_SHA256: &str = "8694fb0e0737529d0fd84430327afc942b711d41c362b789587d6dddf0c45655";

#[test]
fn pinned_literal_digest_matches_payload_identity() {
    let p = anchor_payload();
    assert_eq!(p.len(), 34); // exactly one complete Q8_0 block
                             // The literal is independent of the crate's hasher at test time: if the
                             // fixture bytes drifted from the retained generator, this goes RED.
    assert_eq!(sha256_hex(&p), ANCHOR_SHA256);
}

#[test]
fn positive_path_with_pinned_literal_digest_and_borrowed_view() {
    let base = super::base_file(1); // weight F32 [1,32]
    let be = base_entry(&base);
    let proof = ValidatedBase::new(&base, BASE_NAME, &be, &art_limits(), BASE_MANIFEST_ID)
        .expect("base proof");

    let payload = anchor_payload();
    let mut e = q_entry(&payload);
    e.expected_sha256 = ANCHOR_SHA256.to_string(); // literal, not a test-time hash
    link_base(&mut e, &base);

    let v = validate_quantized(&payload, Q8_NAME, &e, &proof, &art_limits(), &q80_limits())
        .expect("literal-digest positive path must succeed");
    assert_eq!(v.payload().as_ptr(), payload.as_ptr()); // borrowed, not copied
    assert_eq!(v.shape(), &[1usize, 32][..]);
    assert_eq!(v.tensor_name(), TENSOR);
    assert_eq!(v.view().scale_f32(0).unwrap(), 1.0);
    for lane in 0..32 {
        assert_eq!(v.view().quant(0, lane).unwrap(), lane as i8);
    }
}

#[test]
fn repeated_10000_cycles_with_one_prewarmed_base_proof() {
    let base = super::base_file(1);
    let be = base_entry(&base);
    // ONE prevalidated proof reused across every cycle.
    let proof = ValidatedBase::new(&base, BASE_NAME, &be, &art_limits(), BASE_MANIFEST_ID)
        .expect("base proof");
    let payload = super::q8_payload(1, 0x3c00);
    let mut e = q_entry(&payload);
    link_base(&mut e, &base);

    for cycle in 0..10_000 {
        let v = validate_quantized(&payload, Q8_NAME, &e, &proof, &art_limits(), &q80_limits())
            .unwrap_or_else(|_| panic!("cycle {cycle} must validate"));
        assert_eq!(v.payload().as_ptr(), payload.as_ptr());
        assert_eq!(v.view().scale_f32(0).unwrap(), 1.0);
    }

    // A tampered entry after the loop still fails — cycles did not "warm" a hole.
    let mut bad = e.clone();
    bad.expected_sha256 = sha256_hex(b"nope");
    assert!(matches!(
        validate_quantized(
            &payload,
            Q8_NAME,
            &bad,
            &proof,
            &art_limits(),
            &q80_limits()
        ),
        Err(lfm3_brain::quantized_identity::QIdError::HashMismatch)
    ));
}

```

## crates/lfm3-brain/tests/quantized_identity/ordering.rs

```rust
use super::{art_limits, base_and_payload, base_entry, link_base, q_entry, BASE_NAME, Q8_NAME};
use lfm3_brain::quantized::Q80Limits;
use lfm3_brain::quantized_identity::{validate_quantized, QIdError, ValidatedBase};

#[test]
fn matching_shape_resource_policy_precedes_payload_hash() {
    let (base, payload, identity) = base_and_payload();
    let base_entry = base_entry(&base);
    let proof = ValidatedBase::new(&base, BASE_NAME, &base_entry, &art_limits(), &identity)
        .expect("valid base");
    let mut entry = q_entry(&payload);
    link_base(&mut entry, &base);
    entry.expected_sha256 = "0".repeat(64); // Syntax valid, content wrong.

    // The real base and entry both have shape [1, 32]. Each tighter policy
    // must reject BEFORE hashing; shape mismatch cannot hide a missing check.
    for limits in [
        Q80Limits::new(1, 4096, 1_000_000, 4_000_000),
        Q80Limits::new(4, 31, 1_000_000, 4_000_000),
        Q80Limits::new(4, 4096, 31, 4_000_000),
        Q80Limits::new(4, 4096, 1_000_000, 33),
    ] {
        assert!(matches!(
            validate_quantized(&payload, Q8_NAME, &entry, &proof, &art_limits(), &limits),
            Err(QIdError::LimitExceeded)
        ));
    }
}

```

## RUST-CONTINUE-20261028T0000-HASH-RED.log

```text
   Compiling lfm3-brain v0.1.0 ($HOME/artifacts/handoff/redox-perplexity/q8-identity-worktree/crates/lfm3-brain)
warning: unused variable: `expected_digest`
   --> src/quantized_identity.rs:157:9
    |
157 |     let expected_digest = parse_digest(&entry.expected_sha256)?;
    |         ^^^^^^^^^^^^^^^ help: if this is intentional, prefix it with an underscore: `_expected_digest`
    |
    = note: `#[warn(unused_variables)]` (part of `#[warn(unused)]`) on by default

warning: unused variable: `actual_digest`
   --> src/quantized_identity.rs:236:9
    |
236 |     let actual_digest: [u8; 32] = Sha256::digest(payload).into();
    |         ^^^^^^^^^^^^^ help: if this is intentional, prefix it with an underscore: `_actual_digest`

warning: `lfm3-brain` (lib) generated 2 warnings (run `cargo fix --lib -p lfm3-brain` to apply 2 suggestions)
warning: function `f16_bits` is never used
  --> tests/quantized_identity/common.rs:64:8
   |
64 | pub fn f16_bits(value: f32) -> u16 {
   |        ^^^^^^^^
   |
   = note: `#[warn(dead_code)]` (part of `#[warn(unused)]`) on by default

warning: function `pinned_payload` is never used
   --> tests/quantized_identity/common.rs:120:8
    |
120 | pub fn pinned_payload() -> Vec<u8> {
    |        ^^^^^^^^^^^^^^

warning: function `base_file_rows` is never used
   --> tests/quantized_identity/common.rs:129:8
    |
129 | pub fn base_file_rows(rows: usize) -> Vec<u8> {
    |        ^^^^^^^^^^^^^^

warning: `lfm3-brain` (test "tensor_quantized_identity") generated 3 warnings
    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.37s
     Running tests/tensor_quantized_identity.rs (crates/lfm3-brain/target/debug/deps/tensor_quantized_identity-2d5836f4ac6fe12f)

running 1 test
test bindings::altered_payload_rejects_with_hash_mismatch ... FAILED

failures:

---- bindings::altered_payload_rejects_with_hash_mismatch stdout ----

thread 'bindings::altered_payload_rejects_with_hash_mismatch' (3456689) panicked at tests/quantized_identity/bindings.rs:88:5:
assertion failed: matches!(validate_quantized(&payload, Q8_NAME, &e, &proof, &art_limits(),
    &q80_limits()), Err(HashMismatch))
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace


failures:
    bindings::altered_payload_rejects_with_hash_mismatch

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 14 filtered out; finished in 0.00s

error: test failed, to rerun pass `--test tensor_quantized_identity`

```

## RUST-CONTINUE-20261028T0000-BASE-LINK-RED.log

```text
   Compiling lfm3-brain v0.1.0 ($HOME/artifacts/handoff/redox-perplexity/q8-identity-worktree/crates/lfm3-brain)
warning: function `f16_bits` is never used
  --> tests/quantized_identity/common.rs:64:8
   |
64 | pub fn f16_bits(value: f32) -> u16 {
   |        ^^^^^^^^
   |
   = note: `#[warn(dead_code)]` (part of `#[warn(unused)]`) on by default

warning: function `pinned_payload` is never used
   --> tests/quantized_identity/common.rs:120:8
    |
120 | pub fn pinned_payload() -> Vec<u8> {
    |        ^^^^^^^^^^^^^^

warning: function `base_file_rows` is never used
   --> tests/quantized_identity/common.rs:129:8
    |
129 | pub fn base_file_rows(rows: usize) -> Vec<u8> {
    |        ^^^^^^^^^^^^^^

warning: `lfm3-brain` (test "tensor_quantized_identity") generated 3 warnings
    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.31s
     Running tests/tensor_quantized_identity.rs (crates/lfm3-brain/target/debug/deps/tensor_quantized_identity-2d5836f4ac6fe12f)

running 1 test
test bindings::altered_base_links_reject ... FAILED

failures:

---- bindings::altered_base_links_reject stdout ----

thread 'bindings::altered_base_links_reject' (3463047) panicked at tests/quantized_identity/bindings.rs:110:5:
assertion failed: matches!(validate_quantized(&payload, Q8_NAME, &e, &p1, &art_limits(),
    &q80_limits()), Err(BaseLinkMismatch))
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace


failures:
    bindings::altered_base_links_reject

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 14 filtered out; finished in 0.00s

error: test failed, to rerun pass `--test tensor_quantized_identity`

```

## RUST-CONTINUE-20261028T0000-GATE-GREEN.log

```text
warning: function `art_limits` is never used
  --> tests/quantized/common.rs:11:8
   |
11 | pub fn art_limits() -> ArtifactLimits {
   |        ^^^^^^^^^^
   |
   = note: `#[warn(dead_code)]` (part of `#[warn(unused)]`) on by default

warning: function `st_file` is never used
  --> tests/quantized/common.rs:63:8
   |
63 | pub fn st_file() -> Vec<u8> {
   |        ^^^^^^^

warning: function `entry_for` is never used
  --> tests/quantized/common.rs:75:8
   |
75 | pub fn entry_for(file: &[u8], filename: &str) -> lfm3_brain::identity::ManifestEntry {
   |        ^^^^^^^^^

warning: function `half_ref` is never used
  --> tests/quantized/common.rs:26:8
   |
26 | pub fn half_ref(bits: u16) -> Option<f64> {
   |        ^^^^^^^^
   |
   = note: `#[warn(dead_code)]` (part of `#[warn(unused)]`) on by default

warning: `lfm3-brain` (test "tensor_quantized") generated 3 warnings
warning: `lfm3-brain` (test "tensor_quantized_limits") generated 1 warning
    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.08s
     Running unittests src/lib.rs (crates/lfm3-brain/target/debug/deps/lfm3_brain-ef5ca2cb7a59c22a)

running 30 tests
..............................
test result: ok. 30 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s

     Running tests/model_contract.rs (crates/lfm3-brain/target/debug/deps/model_contract-41a2177b58fa87eb)

running 1 test
.
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

     Running tests/tensor_adversarial.rs (crates/lfm3-brain/target/debug/deps/tensor_adversarial-d2e990af460f21de)

running 6 tests
......
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.09s

     Running tests/tensor_artifacts.rs (crates/lfm3-brain/target/debug/deps/tensor_artifacts-4c8b027dbc45a72b)

running 18 tests
..................
test result: ok. 18 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

     Running tests/tensor_identity.rs (crates/lfm3-brain/target/debug/deps/tensor_identity-3cba4dcc770705ee)

running 22 tests
......................
test result: ok. 22 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.28s

     Running tests/tensor_quantized.rs (crates/lfm3-brain/target/debug/deps/tensor_quantized-82f31d24f1196486)

running 7 tests
.......
test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s

     Running tests/tensor_quantized_identity.rs (crates/lfm3-brain/target/debug/deps/tensor_quantized_identity-2d5836f4ac6fe12f)

running 16 tests
................
test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.16s

     Running tests/tensor_quantized_limits.rs (crates/lfm3-brain/target/debug/deps/tensor_quantized_limits-763460d308483bf8)

running 6 tests
......
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

     Running tests/tensor_views.rs (crates/lfm3-brain/target/debug/deps/tensor_views-68719f1efe441077)

running 3 tests
...
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

   Doc-tests lfm3_brain

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s


```
