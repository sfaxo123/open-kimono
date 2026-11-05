// Proof construction + binding-link negatives for the typed quantized path:
// failed base bytes, altered payload, altered base links, and altered
// name/shape/tag/recipe/model/revision must all reject.

use super::{
    art_limits, base_and_payload, base_entry, link_base, q80_limits, q_entry, sha256_hex,
    BASE_MANIFEST_ID, BASE_NAME, Q8_NAME,
};
use lfm3_brain::identity::ManifestEntry;
use lfm3_brain::quantized_identity::{
    validate_quantized,
    QIdError::{
        BaseLinkMismatch, BaseStructure, HashMismatch, MalformedDigest, ShapeMismatch,
        SizeMismatch, WrongFilename, WrongModel, WrongRecipeId, WrongRecipeRevision, WrongRevision,
        WrongTensorName, WrongWireTag,
    },
    QuantizedEntry, ValidatedBase,
};

fn run(payload: &[u8], name: &str, e: &QuantizedEntry, base: &ValidatedBase<'_>) -> bool {
    validate_quantized(payload, name, e, base, &art_limits(), &q80_limits()).is_ok()
}

#[test]
fn positive_anchor_binds_and_reads_back() {
    let (base, payload, bid) = base_and_payload();
    let be = base_entry(&base);
    let proof = ValidatedBase::new(&base, BASE_NAME, &be, &art_limits(), &bid).unwrap();
    let mut e = q_entry(&payload);
    link_base(&mut e, &base);
    let v = validate_quantized(&payload, Q8_NAME, &e, &proof, &art_limits(), &q80_limits())
        .expect("positive path");
    assert_eq!(v.payload().as_ptr(), payload.as_ptr()); // borrowed, not copied
    assert_eq!(v.shape(), &[1usize, 32][..]);
    assert_eq!(v.view().scale_f32(0).unwrap(), 1.0);
    assert_eq!(v.manifest_identity(), &e.manifest_identity);
    assert!(run(&payload, Q8_NAME, &e, &proof));
}

/// The canonical 1-row base file plus its trusted entry and identity.
fn anchor_ids() -> (Vec<u8>, ManifestEntry, String) {
    let base = super::base_file(1);
    let be = base_entry(&base);
    (base, be, BASE_MANIFEST_ID.to_string())
}

#[test]
fn failed_base_bytes_cannot_produce_proof() {
    let (mut base, be, bid) = anchor_ids();
    base.push(0xAB); // actual bytes no longer match the original entry
    assert!(matches!(
        ValidatedBase::new(&base, BASE_NAME, &be, &art_limits(), &bid),
        Err(SizeMismatch { .. })
    ));
    // Corrupt a middle byte: length matches, digest does not -> HashMismatch.
    let (good, be2, _) = anchor_ids();
    let mut bad = good.clone();
    let mid = bad.len() / 2;
    bad[mid] ^= 0xFF;
    assert!(matches!(
        ValidatedBase::new(&bad, BASE_NAME, &be2, &art_limits(), &bid),
        Err(HashMismatch)
    ));
    // Altered tensor length INSIDE the header (weight claims 132 bytes but
    // only 128 remain): header JSON byte length is unchanged, so the file
    // length still matches a fresh entry and identity passes; the accepted
    // parser then rejects -> BaseStructure. Built by rebuilding the same file
    // from common helpers with one digit changed in the weight end offset.
    let w = super::lcg_bytes(128);
    let o = super::lcg_bytes(128);
    let header = "{\"weight\":{\"dtype\":\"F32\",\"shape\":[1,32],\"data_offsets\":[0,132]},\"other\":{\"dtype\":\"F16\",\"shape\":[64],\"data_offsets\":[128,256]}}";
    let hdr_bad = super::st(header, &w);
    let hdr_bad = {
        let mut f = hdr_bad;
        f.extend_from_slice(&o);
        f
    };
    assert_eq!(hdr_bad.len(), good.len()); // same total length as the anchor
    let be_hdr = super::base_entry_named(&hdr_bad);
    assert!(matches!(
        ValidatedBase::new(&hdr_bad, BASE_NAME, &be_hdr, &art_limits(), &bid),
        Err(BaseStructure(_))
    ));

    // Malformed out-of-band identity is rejected BEFORE hashing the base.
    assert!(matches!(
        ValidatedBase::new(&good, BASE_NAME, &be2, &art_limits(), "not-hex"),
        Err(MalformedDigest)
    ));
}

#[test]
fn altered_payload_rejects_with_hash_mismatch() {
    let (base, mut payload, bid) = base_and_payload();
    let be = base_entry(&base);
    let proof = ValidatedBase::new(&base, BASE_NAME, &be, &art_limits(), &bid).unwrap();
    let mut e = q_entry(&payload);
    link_base(&mut e, &base);
    assert!(run(&payload, Q8_NAME, &e, &proof));
    payload[0] ^= 0xFF; // entry still claims the old digest
    assert!(matches!(
        validate_quantized(&payload, Q8_NAME, &e, &proof, &art_limits(), &q80_limits()),
        Err(HashMismatch)
    ));
}

#[test]
fn altered_base_links_reject() {
    let (base, payload, bid) = base_and_payload();
    let be = base_entry(&base);

    // Proof identity differs from the entry's claimed base manifest identity.
    let p1 = ValidatedBase::new(
        &base,
        BASE_NAME,
        &be,
        &art_limits(),
        "2222000000000000000000000000000000000000000000000000000000000000",
    )
    .unwrap();
    let mut e = q_entry(&payload);
    link_base(&mut e, &base);
    assert!(matches!(
        validate_quantized(&payload, Q8_NAME, &e, &p1, &art_limits(), &q80_limits()),
        Err(BaseLinkMismatch)
    ));

    // Proof entry (filename) differs from the entry's claimed base filename.
    let mut be2 = be.clone();
    be2.filename = "alt.safetensors".to_string();
    let p2 = ValidatedBase::new(&base, "alt.safetensors", &be2, &art_limits(), &bid).unwrap();
    assert!(matches!(
        validate_quantized(&payload, Q8_NAME, &e, &p2, &art_limits(), &q80_limits()),
        Err(BaseLinkMismatch)
    ));

    // Proof over a different real base file (size/digest differ).
    let other = super::base_file(2);
    let be3 = base_entry(&other);
    let p3 = ValidatedBase::new(&other, BASE_NAME, &be3, &art_limits(), &bid).unwrap();
    assert!(matches!(
        validate_quantized(&payload, Q8_NAME, &e, &p3, &art_limits(), &q80_limits()),
        Err(BaseLinkMismatch)
    ));

    // Entry's claimed base_sha256 alone is tampered (proof untouched).
    let p4 = ValidatedBase::new(&base, BASE_NAME, &be, &art_limits(), &bid).unwrap();
    let mut e4 = e.clone();
    e4.base_sha256 = sha256_hex(b"tampered-base");
    assert!(matches!(
        validate_quantized(&payload, Q8_NAME, &e4, &p4, &art_limits(), &q80_limits()),
        Err(BaseLinkMismatch)
    ));
}

#[test]
fn altered_name_shape_tag_recipe_model_revision_reject() {
    let (base, payload, bid) = base_and_payload();
    let be = base_entry(&base);
    let proof = ValidatedBase::new(&base, BASE_NAME, &be, &art_limits(), &bid).unwrap();
    let mut e = q_entry(&payload);
    link_base(&mut e, &base);

    let mut a = e.clone();
    a.filename = "other.q8".into();
    assert!(matches!(
        validate_quantized(&payload, Q8_NAME, &a, &proof, &art_limits(), &q80_limits()),
        Err(WrongFilename)
    ));
    // requested name differs from entry: rejected even when the entry is fine.
    assert!(matches!(
        validate_quantized(&payload, "x.q8", &e, &proof, &art_limits(), &q80_limits()),
        Err(WrongFilename)
    ));

    let mut a = e.clone();
    a.tensor_name = "missing".into(); // not in the base at all
    assert!(matches!(
        validate_quantized(&payload, Q8_NAME, &a, &proof, &art_limits(), &q80_limits()),
        Err(WrongTensorName)
    ));

    let mut a = e.clone();
    a.shape = vec![2, 32]; // base tensor is [1,32]
    assert!(matches!(
        validate_quantized(&payload, Q8_NAME, &a, &proof, &art_limits(), &q80_limits()),
        Err(ShapeMismatch)
    ));

    let mut a = e.clone();
    a.wire_tag = 2;
    assert!(matches!(
        validate_quantized(&payload, Q8_NAME, &a, &proof, &art_limits(), &q80_limits()),
        Err(WrongWireTag(2))
    ));

    let mut a = e.clone();
    a.recipe_id = "ggml-q4_0-ref".into();
    assert!(matches!(
        validate_quantized(&payload, Q8_NAME, &a, &proof, &art_limits(), &q80_limits()),
        Err(WrongRecipeId)
    ));

    let mut a = e.clone();
    a.recipe_revision = "0".repeat(40);
    assert!(matches!(
        validate_quantized(&payload, Q8_NAME, &a, &proof, &art_limits(), &q80_limits()),
        Err(WrongRecipeRevision)
    ));

    let mut a = e.clone();
    a.model_id = "Other/Model".into();
    assert!(matches!(
        validate_quantized(&payload, Q8_NAME, &a, &proof, &art_limits(), &q80_limits()),
        Err(WrongModel(_))
    ));

    let mut a = e.clone();
    a.source_revision = "0".repeat(40);
    assert!(matches!(
        validate_quantized(&payload, Q8_NAME, &a, &proof, &art_limits(), &q80_limits()),
        Err(WrongRevision(_))
    ));
}

#[test]
fn own_manifest_identity_is_trusted_not_cross_checked() {
    // Documented limit: the entry's OWN manifest identity has no cross-check
    // (nothing to check it against); altering it must not change validation.
    let (base, payload, bid) = base_and_payload();
    let be = base_entry(&base);
    let proof = ValidatedBase::new(&base, BASE_NAME, &be, &art_limits(), &bid).unwrap();
    let mut e = q_entry(&payload);
    link_base(&mut e, &base);
    let mut a = e.clone();
    a.manifest_identity = "ff".repeat(32);
    assert!(
        validate_quantized(&payload, Q8_NAME, &a, &proof, &art_limits(), &q80_limits()).is_ok()
    );
}
