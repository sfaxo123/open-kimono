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
