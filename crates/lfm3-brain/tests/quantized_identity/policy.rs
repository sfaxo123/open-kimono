// Policy, digest-spelling, string-limit and hash-vs-structure ordering
// negatives for the typed quantized path.

use super::{
    art_limits, base_and_payload, base_entry, lcg_bytes, link_base, q80_limits, q_entry,
    sha256_hex, st, BASE_NAME, Q8_NAME,
};
use lfm3_brain::quantized::Q80Limits;
use lfm3_brain::quantized_identity::{
    validate_quantized,
    QIdError::{
        BaseLinkMismatch, BaseStructure, HashMismatch, LimitExceeded, MalformedDigest,
        ManifestStringTooLong, ShapeMismatch, SizeMismatch, Structure, UnsafeFilename,
        UnsupportedBaseDtype, WrongTensorName,
    },
    ValidatedBase,
};

#[test]
fn altered_hash_size_and_digest_spellings_reject() {
    let (base, payload, bid) = base_and_payload();
    let be = base_entry(&base);
    let proof = ValidatedBase::new(&base, BASE_NAME, &be, &art_limits(), &bid).unwrap();
    let mut e = q_entry(&payload);
    link_base(&mut e, &base);

    let mut a = e.clone();
    a.expected_sha256 = sha256_hex(b"altered");
    assert!(matches!(
        validate_quantized(&payload, Q8_NAME, &a, &proof, &art_limits(), &q80_limits()),
        Err(HashMismatch)
    ));

    let mut a = e.clone();
    a.expected_bytes += 34; // claims one more block than provided
    assert!(matches!(
        validate_quantized(&payload, Q8_NAME, &a, &proof, &art_limits(), &q80_limits()),
        Err(SizeMismatch {
            expected: 68,
            actual: 34
        })
    ));

    for spelling in [
        "BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD".to_string(), // upper
        "ba78".to_string(),                                                             // short
        format!("{}ff", e.expected_sha256),                                             // long
    ] {
        let mut a = e.clone();
        a.expected_sha256 = spelling;
        assert!(matches!(
            validate_quantized(&payload, Q8_NAME, &a, &proof, &art_limits(), &q80_limits()),
            Err(MalformedDigest)
        ));
    }

    let mut a = e.clone();
    a.base_manifest_identity = "GG".repeat(32); // uppercase: differs from strict proof id
    assert!(matches!(
        validate_quantized(&payload, Q8_NAME, &a, &proof, &art_limits(), &q80_limits()),
        Err(BaseLinkMismatch)
    ));

    let mut a = e.clone();
    a.base_sha256 = "zz".repeat(32); // differs from the proof's strict digest
    assert!(matches!(
        validate_quantized(&payload, Q8_NAME, &a, &proof, &art_limits(), &q80_limits()),
        Err(BaseLinkMismatch)
    ));

    let mut a = e.clone();
    a.manifest_identity = "UPPER".to_string(); // own identity parsed strict too
    assert!(matches!(
        validate_quantized(&payload, Q8_NAME, &a, &proof, &art_limits(), &q80_limits()),
        Err(MalformedDigest)
    ));
}

#[test]
fn excessive_strings_and_unsafe_names_reject() {
    let (base, payload, bid) = base_and_payload();
    let be = base_entry(&base);
    let proof = ValidatedBase::new(&base, BASE_NAME, &be, &art_limits(), &bid).unwrap();
    let mut e = q_entry(&payload);
    link_base(&mut e, &base);

    let long300 = "a".repeat(300); // > MAX_FILENAME_BYTES (255)
    for field in 0..4 {
        let mut a = e.clone();
        match field {
            // filename alone is checked for UNSAFE first; long-but-safe names
            // hit the length cap, so use a long safe name here.
            0 => a.filename = "w".repeat(296) + ".q8",
            1 => a.manifest_identity = long300.clone(),
            2 => a.base_manifest_identity = long300.clone(),
            _ => a.expected_sha256 = long300.clone(),
        }
        let (expected_field, expected_len) = match field {
            0 => ("filename", a.filename.len()),
            1 => ("manifest_identity", a.manifest_identity.len()),
            2 => ("base_manifest_identity", a.base_manifest_identity.len()),
            _ => ("expected_sha256", a.expected_sha256.len()),
        };
        assert!(matches!(
            validate_quantized(&payload, Q8_NAME, &a, &proof, &art_limits(), &q80_limits()),
            Err(ManifestStringTooLong(f, n)) if f == expected_field && n == expected_len
        ));
    }

    let mut a = e.clone();
    a.tensor_name = "t".repeat(1025); // > MAX_TENSOR_NAME_BYTES (1024)
    assert!(matches!(
        validate_quantized(&payload, Q8_NAME, &a, &proof, &art_limits(), &q80_limits()),
        Err(ManifestStringTooLong("tensor_name", 1025))
    ));

    for bad in [
        ".q8",
        "sub/dir.q8",
        "back\\slash.q8",
        "has space.q8",
        "nul\0tail.q8",
    ] {
        let mut a = e.clone();
        a.filename = bad.to_string();
        assert!(matches!(
            validate_quantized(&payload, Q8_NAME, &a, &proof, &art_limits(), &q80_limits()),
            Err(UnsafeFilename)
        ));
    }

    // Oversized requested filename is bounded before any comparison.
    let huge_name = format!("{}.q8", "z".repeat(300));
    assert!(matches!(
        validate_quantized(
            &payload,
            &huge_name,
            &e,
            &proof,
            &art_limits(),
            &q80_limits()
        ),
        Err(ManifestStringTooLong("requested_filename", 303))
    ));

    // A base entry with an oversized model id cannot make a proof.
    let mut be2 = be.clone();
    be2.model_id = "m".repeat(129);
    assert!(matches!(
        ValidatedBase::new(&base, BASE_NAME, &be2, &art_limits(), &bid),
        Err(ManifestStringTooLong("model_id", 129))
    ));
}

#[test]
fn excessive_rank_dim_element_byte_and_overflow_policy_reject() {
    let (base, payload, bid) = base_and_payload();
    let be = base_entry(&base);
    let proof = ValidatedBase::new(&base, BASE_NAME, &be, &art_limits(), &bid).unwrap();
    let mut e = q_entry(&payload);
    link_base(&mut e, &base);

    let mut a = e.clone();
    a.shape = vec![1, 1, 1, 1, 32]; // rank 5 > max_rank 4 (and != base shape)
    assert!(matches!(
        validate_quantized(&payload, Q8_NAME, &a, &proof, &art_limits(), &q80_limits()),
        Err(ShapeMismatch)
    ));

    let mut a = e.clone();
    a.shape = vec![]; // rank zero rejected
    assert!(matches!(
        validate_quantized(&payload, Q8_NAME, &a, &proof, &art_limits(), &q80_limits()),
        Err(ShapeMismatch)
    ));

    // Huge dimensions do not match this small, actually validated base;
    // rejection here proves the binding gate, not an arithmetic-overflow path.
    let mut a = e.clone();
    a.shape = vec![1, usize::MAX]; // != base [1,32] -> shape gate first
    assert!(matches!(
        validate_quantized(&payload, Q8_NAME, &a, &proof, &art_limits(), &q80_limits()),
        Err(ShapeMismatch)
    ));

    // Policy-only rejections need a base whose shape MATCHES the hostile entry.
    // Base weight F32 [1,4097]: dim > max_dim 4096 -> policy before hash.
    let w = lcg_bytes(4097 * 4);
    let header = r#"{"weight":{"dtype":"F32","shape":[1,4097],"data_offsets":[0,16388]}}"#;
    let b4097 = st(header, &w);
    let be4097 = base_entry(&b4097);
    let p4097 = ValidatedBase::new(&b4097, BASE_NAME, &be4097, &art_limits(), &bid).unwrap();
    let mut a = q_entry(&payload);
    link_base(&mut a, &b4097);
    a.shape = vec![1, 4097];
    assert!(matches!(
        validate_quantized(&payload, Q8_NAME, &a, &p4097, &art_limits(), &q80_limits()),
        Err(LimitExceeded)
    ));

    // Row alignment: base+entry [1,33] (not a multiple of 32) -> policy.
    let w = lcg_bytes(33 * 4);
    let header = r#"{"weight":{"dtype":"F32","shape":[1,33],"data_offsets":[0,132]}}"#;
    let b33 = st(header, &w);
    let be33 = base_entry(&b33);
    let p33 = ValidatedBase::new(&b33, BASE_NAME, &be33, &art_limits(), &bid).unwrap();
    let mut a = q_entry(&payload);
    link_base(&mut a, &b33);
    a.shape = vec![1, 33];
    assert!(matches!(
        validate_quantized(&payload, Q8_NAME, &a, &p33, &art_limits(), &q80_limits()),
        Err(LimitExceeded)
    ));

    // Byte policy: max_bytes below the exact payload size -> LimitExceeded.
    let tiny = Q80Limits::new(4, 4096, 1_000_000, 33);
    assert!(matches!(
        validate_quantized(&payload, Q8_NAME, &e, &proof, &art_limits(), &tiny),
        Err(LimitExceeded)
    ));

    // Element policy: max_elements below the shape product -> LimitExceeded.
    let few = Q80Limits::new(4, 4096, 16, 4_000_000);
    assert!(matches!(
        validate_quantized(&payload, Q8_NAME, &e, &proof, &art_limits(), &few),
        Err(LimitExceeded)
    ));

    // Shape-implied bytes vs claimed size: entry lies about size in a way the
    // payload length confirms but the shape product contradicts.
    let fat = q8_payload_local(2); // 68 bytes for [2,32]
    let b2 = base_two_rows();
    let be2 = base_entry(&b2);
    let p2 = ValidatedBase::new(&b2, BASE_NAME, &be2, &art_limits(), &bid).unwrap();
    let mut a = q_entry(&fat);
    link_base(&mut a, &b2);
    a.shape = vec![2, 32]; // base [2,32]: policy passes; honest digest -> OK
    assert!(validate_quantized(&fat, Q8_NAME, &a, &p2, &art_limits(), &q80_limits()).is_ok());
    let mut a = q_entry(&payload); // claims 68 bytes but hands 34
    link_base(&mut a, &b2);
    a.shape = vec![2, 32];
    a.expected_bytes = 68;
    assert!(matches!(
        validate_quantized(&payload, Q8_NAME, &a, &p2, &art_limits(), &q80_limits()),
        Err(SizeMismatch {
            expected: 68,
            actual: 34
        })
    ));
}

fn q8_payload_local(blocks: usize) -> Vec<u8> {
    super::q8_payload(blocks, 0x3c00)
}

/// Base with weight F32 [2,32] only, for shape-matched policy probes.
fn base_two_rows() -> Vec<u8> {
    let w = lcg_bytes(2 * 32 * 4);
    let header = r#"{"weight":{"dtype":"F32","shape":[2,32],"data_offsets":[0,256]}}"#;
    st(header, &w)
}

#[test]
fn missing_tensor_and_nonfloat_dtype_reject() {
    let (_, payload, bid) = base_and_payload();
    let mut e = q_entry(&payload);

    // Base with an INT32 "weight": dtype gate rejects.
    let w = lcg_bytes(32 * 4);
    let header = r#"{"weight":{"dtype":"I32","shape":[1,32],"data_offsets":[0,128]}}"#;
    let ibase = st(header, &w);
    let ibe = base_entry(&ibase);
    let iproof = ValidatedBase::new(&ibase, BASE_NAME, &ibe, &art_limits(), &bid).unwrap();
    link_base(&mut e, &ibase);
    assert!(matches!(
        validate_quantized(&payload, Q8_NAME, &e, &iproof, &art_limits(), &q80_limits()),
        Err(e) if matches!(&e, UnsupportedBaseDtype(d) if d == "I32")
    ));

    // Base where "weight" is absent (only "other"): name gate rejects.
    let o = lcg_bytes(128);
    let header = r#"{"other":{"dtype":"F32","shape":[32],"data_offsets":[0,128]}}"#;
    let obase = st(header, &o);
    let obe = base_entry(&obase);
    let oproof = ValidatedBase::new(&obase, BASE_NAME, &obe, &art_limits(), &bid).unwrap();
    let mut e2 = q_entry(&payload);
    link_base(&mut e2, &obase);
    assert!(matches!(
        validate_quantized(
            &payload,
            Q8_NAME,
            &e2,
            &oproof,
            &art_limits(),
            &q80_limits()
        ),
        Err(WrongTensorName)
    ));

    // A quantized dtype string in the base: the accepted parser rejects it, so
    // no proof is ever produced.
    let w = lcg_bytes(34);
    let header = r#"{"weight":{"dtype":"Q8_0","shape":[1,32],"data_offsets":[0,34]}}"#;
    let qbase = st(header, &w);
    assert!(matches!(
        ValidatedBase::new(&qbase, BASE_NAME, &base_entry(&qbase), &art_limits(), &bid),
        Err(BaseStructure(_))
    ));
}

#[test]
fn bad_scale_wrong_hash_reports_hash_failure_first() {
    let (base, mut payload, bid) = base_and_payload();
    // Non-finite scale bits (Inf) AND the entry digest does not match.
    payload[0..2].copy_from_slice(&0x7c00u16.to_le_bytes());
    let be = base_entry(&base);
    let proof = ValidatedBase::new(&base, BASE_NAME, &be, &art_limits(), &bid).unwrap();
    let mut e = q_entry(&payload); // honest digest over tampered payload...
    link_base(&mut e, &base);
    e.expected_sha256 = sha256_hex(b"stale"); // ...but claims an OLD digest
    assert!(matches!(
        validate_quantized(&payload, Q8_NAME, &e, &proof, &art_limits(), &q80_limits()),
        Err(HashMismatch)
    ));
}

#[test]
fn matching_hash_bad_scale_reports_structure_failure() {
    let (base, mut payload, bid) = base_and_payload();
    payload[0..2].copy_from_slice(&0x7c00u16.to_le_bytes()); // Inf scale bits
    let be = base_entry(&base);
    let proof = ValidatedBase::new(&base, BASE_NAME, &be, &art_limits(), &bid).unwrap();
    let mut e = q_entry(&payload); // honest digest over the bad-scale payload
    link_base(&mut e, &base);
    assert!(matches!(
        validate_quantized(&payload, Q8_NAME, &e, &proof, &art_limits(), &q80_limits()),
        Err(Structure(_))
    ));
}
