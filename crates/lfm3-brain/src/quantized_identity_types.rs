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
