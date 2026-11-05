//! Resource limits and validation ordering; no device or model execution.
use super::{entry_for, fixture, limits, sha256_hex, validate_artifact};
use lfm3_brain::artifacts::ArtifactLimits;
use lfm3_brain::identity::{
    IdentityError::{LimitExceeded, ManifestStringTooLong},
    MAX_FILENAME_BYTES, MAX_MODEL_ID_BYTES, MAX_REVISION_BYTES,
};
// ---------- manifest string bounds (hostile oversized input) ----------

#[test]
fn rejects_oversized_manifest_strings_without_cloning_them() {
    let file = fixture();
    let mut entry = entry_for(&file, "model.safetensors");

    entry.model_id = "x".repeat(MAX_MODEL_ID_BYTES + 1);
    assert_err!(
        validate_artifact(&file, "model.safetensors", &entry, &limits()),
        ManifestStringTooLong("model_id", n) if n == MAX_MODEL_ID_BYTES + 1
    );

    entry = entry_for(&file, "model.safetensors");
    entry.source_revision = "y".repeat(MAX_REVISION_BYTES + 1);
    assert_err!(
        validate_artifact(&file, "model.safetensors", &entry, &limits()),
        ManifestStringTooLong("source_revision", n) if n == MAX_REVISION_BYTES + 1
    );

    entry = entry_for(&file, "model.safetensors");
    let long_name = format!("{}model.safetensors", "z".repeat(MAX_FILENAME_BYTES));
    entry.filename = long_name.clone();
    assert_err!(
        validate_artifact(&file, &long_name, &entry, &limits()),
        ManifestStringTooLong("filename", n) if n == long_name.len()
    );

    // Oversized *requested* filename (entry itself is fine) is bounded too.
    entry = entry_for(&file, "model.safetensors");
    let long_req = format!("{}x.safetensors", "w".repeat(MAX_FILENAME_BYTES));
    assert_err!(
        validate_artifact(&file, &long_req, &entry, &limits()),
        ManifestStringTooLong("requested_filename", n) if n == long_req.len()
    );

    // At the bound exactly, normal rules still apply (accepted name passes).
    entry = entry_for(&file, "model.safetensors");
    let mut ok_name = "q".repeat(MAX_FILENAME_BYTES - ".safetensors".len() - 1);
    ok_name.push('a');
    ok_name.push_str(".safetensors");
    assert_eq!(ok_name.len(), MAX_FILENAME_BYTES);
    entry.filename = ok_name.clone();
    entry.expected_sha256 = sha256_hex(&file);
    assert!(validate_artifact(&file, &ok_name, &entry, &limits()).is_ok());
}

#[test]
fn oversized_hostile_model_id_rejected_even_with_matching_format_and_size() {
    // Bound check runs before any comparison or hashing: a megabyte-long model
    // id fails as TooLong (not WrongModel), and the error carries only length.
    let file = fixture();
    let mut entry = entry_for(&file, "model.safetensors");
    entry.model_id = "M".repeat(1 << 20);
    match validate_artifact(&file, "model.safetensors", &entry, &limits()) {
        Err(ManifestStringTooLong(field, n)) => {
            assert_eq!(field, "model_id");
            assert_eq!(n, 1 << 20);
        }
        other => panic!("expected ManifestStringTooLong, got {other:?}"),
    }
}

#[test]
fn header_resource_policy_precedes_hashing() {
    let file = fixture();
    let mut entry = entry_for(&file, "model.safetensors");
    entry.expected_sha256 = "0".repeat(64);
    let tiny = ArtifactLimits {
        max_header_bytes: 8,
        ..limits()
    };
    assert_err!(
        validate_artifact(&file, "model.safetensors", &entry, &tiny),
        LimitExceeded
    );
}

#[test]
fn repeated_identity_validation_drops_each_borrowed_view() {
    let file = fixture();
    let entry = entry_for(&file, "model.safetensors");
    let limits = limits();
    for _ in 0..10_000 {
        let artifact = validate_artifact(&file, "model.safetensors", &entry, &limits).unwrap();
        assert_eq!(artifact.tensors.len(), 2);
    }
    // Measured separately as one process; not a proof of leak-freedom or fit.
}
