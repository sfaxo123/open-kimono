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
