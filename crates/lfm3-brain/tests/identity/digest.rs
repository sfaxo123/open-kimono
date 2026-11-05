//! Independent SHA256 vectors and strict digest syntax tests.
use super::{parse_digest, sha256_hex};
use lfm3_brain::identity::IdentityError::MalformedDigest;

#[test]
fn selected_sha256_block_layout_matches_byte_array() {
    use sha2::digest::{generic_array::GenericArray, typenum::U64};
    type Block = GenericArray<u8, U64>;
    assert_eq!(std::mem::size_of::<Block>(), 64);
    assert_eq!(std::mem::align_of::<Block>(), 1);
    // This concrete invariant supports the audited cast; it does not prove
    // every unsafe operation in generic-array/block-buffer memory-safe.
}
// ---------- independent published SHA-256 known-answer vectors ----------
// These are the canonical FIPS 180-4 / NIST-published digests, independent of
// this crate and of the model format loader.

#[test]
fn digest_parser_accepts_published_known_vectors() {
    // SHA-256("")
    let empty = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
    // SHA-256("abc")
    let abc = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
    // SHA-256("abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq") — the
    // canonical NIST 56-byte two-block multiblock vector.
    let multiblock = "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1";
    let msg: &[u8] = b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq";
    assert_eq!(sha256_hex(msg), multiblock);

    for hex in [empty, abc, multiblock] {
        let parsed = parse_digest(hex).expect("valid published digest");
        assert_eq!(parsed.len(), 32);
    }
    // Round-trip: parser output equals the raw digest bytes.
    assert_eq!(
        parse_digest(abc).unwrap().as_slice(),
        sha256_digest(b"abc").as_slice()
    );
    assert_eq!(
        parse_digest(empty).unwrap().as_slice(),
        sha256_digest(b"").as_slice()
    );
}

fn sha256_digest(data: &[u8]) -> Vec<u8> {
    use sha2::{Digest, Sha256};
    Sha256::digest(data).as_slice().to_vec()
}

#[test]
fn digest_parser_rejects_malformed_spellings() {
    assert_err!(parse_digest(""), MalformedDigest);
    assert_err!(parse_digest("ba78"), MalformedDigest); // too short
    assert_err!(
        parse_digest(&"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015add"),
        MalformedDigest
    ); // 65 chars
    assert_err!(
        // uppercase hex is NOT accepted by this strict parser
        parse_digest("BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD"),
        MalformedDigest
    );
    assert_err!(
        parse_digest("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015g-"),
        MalformedDigest
    ); // non-hex
    assert_err!(
        parse_digest("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015a "),
        MalformedDigest
    ); // embedded whitespace
}
