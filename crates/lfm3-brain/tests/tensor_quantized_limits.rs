// Contract acceptance tests (part 2/2): shape/limits, truncation, version,
// multi-row/endian/borrowed identity, and identity-path rejection.
// Explicit imports only. Rust-only fixture logic.

#[path = "quantized/common.rs"]
mod q8common;

use lfm3_brain::identity::{validate_artifact, ArtifactFormat, IdentityError, ManifestEntry};
use lfm3_brain::quantized::{
    Q80Error, Q80Limits, Q80Version, Q80View, Q80_BLOCK_BYTES, Q80_BLOCK_SIZE,
};
use q8common::art_limits;
use q8common::block;
use q8common::entry_for;
use q8common::lim;
use q8common::st_file;

#[test]
fn multirow_endian_and_borrow_identity() {
    let mut p = block(0x4000, [1; 32]); // row 0: scale 2.0
    p.extend_from_slice(&block(0x3800, [-1; 32])); // row 1: scale 0.5
    let shape = [2usize, 32];
    let v = Q80View::new(Q80Version::V1, &p, &shape, lim()).unwrap();
    assert_eq!(v.block_count(), 2);
    assert_eq!(v.decode_element(31).unwrap(), 2.0);
    assert_eq!(v.decode_element(32).unwrap(), -0.5);
    assert_eq!(v.element_count().unwrap(), 64);
    // Borrowed payload/shape: getters point at the caller's storage itself.
    assert_eq!(v.payload().as_ptr(), p.as_ptr());
    assert_eq!(v.shape().as_ptr(), shape.as_ptr());
    assert_eq!(v.version(), Q80Version::V1);
    // Little-endian scale byte order verified directly against raw bytes.
    assert_eq!(v.scale_bits(1).unwrap(), 0x3800);
    assert_eq!(&p[34..36], &[0x00, 0x38]);
}

#[test]
fn all_truncations_and_trailing_bytes_rejected() {
    let good = block(0x3C00, [0; 32]);
    for cut in 0..Q80_BLOCK_BYTES {
        assert_eq!(
            Q80View::new(Q80Version::V1, &good[..cut], &[32], lim()),
            Err(Q80Error::PayloadTooShort {
                needed: Q80_BLOCK_BYTES,
                got: cut
            }),
            "truncation at {cut}"
        );
    }
    // Any trailing byte past the exact block multiple is rejected too.
    for extra in 1..=3usize {
        let mut trail = good.clone();
        trail.extend_from_slice(&[0x9A; 8][..extra]);
        assert_eq!(
            Q80View::new(Q80Version::V1, &trail, &[32], lim()),
            Err(Q80Error::PayloadTooLong {
                needed: Q80_BLOCK_BYTES,
                got: Q80_BLOCK_BYTES + extra
            }),
            "trailing {extra} bytes"
        );
    }
}

#[test]
fn shape_and_limit_errors() {
    let p = block(0x3C00, [0; 32]);
    assert_eq!(
        Q80View::new(Q80Version::V1, &p, &[], lim()),
        Err(Q80Error::RankZero)
    );
    let r5 = [1usize, 1, 1, 1, 32];
    assert_eq!(
        Q80View::new(Q80Version::V1, &p, &r5, lim()),
        Err(Q80Error::RankTooLarge {
            rank: 5,
            max_rank: 4
        }),
    );
    assert_eq!(
        Q80View::new(Q80Version::V1, &p, &[0, 32], lim()),
        Err(Q80Error::DimensionZero { index: 0 }),
    );
    assert_eq!(
        Q80View::new(Q80Version::V1, &p, &[5000, 32], lim()),
        Err(Q80Error::DimensionTooLarge {
            dim: 5000,
            max_dim: 4096
        }),
    );
    assert_eq!(
        Q80View::new(Q80Version::V1, &p, &[2, 33], lim()),
        Err(Q80Error::LastDimNotMultiple { last_dim: 33 }),
    );
}

#[test]
fn element_and_byte_limits_and_overflow() {
    let p = block(0x3C00, [0; 32]);
    assert_eq!(
        Q80View::new(
            Q80Version::V1,
            &p,
            &[32],
            Q80Limits::new(4, 4096, 16, 4_000_000)
        ),
        Err(Q80Error::TooManyElements {
            elements: 32,
            max_elements: 16
        }),
    );
    assert_eq!(
        Q80View::new(
            Q80Version::V1,
            &p,
            &[32],
            Q80Limits::new(4, 4096, 1_000_000, 33)
        ),
        Err(Q80Error::ByteLimitExceeded {
            bytes: Q80_BLOCK_BYTES,
            max_bytes: 33
        }),
    );
    // Element product past configured limit surfaces via the byte budget.
    let wide = [4096usize, 4096, 32];
    assert_eq!(
        Q80View::new(Q80Version::V1, &p, &wide, lim()),
        Err(Q80Error::ByteLimitExceeded {
            bytes: 570_425_344,
            max_bytes: 4_000_000
        }),
    );
    // Element-product overflow in checked shape multiplication.
    let huge = [usize::MAX, usize::MAX, 32];
    assert_eq!(
        Q80View::new(
            Q80Version::V1,
            &p,
            &huge,
            Q80Limits::new(4, usize::MAX, usize::MAX, usize::MAX)
        ),
        Err(Q80Error::ByteProductOverflow),
    );
    // True byte-multiplication overflow: blocks = usize::MAX/32 exactly
    // (blocks*32 == usize::MAX-31, no element overflow) but blocks*34 wraps.
    let maxblocks = usize::MAX / Q80_BLOCK_SIZE;
    let byteoverflow = [maxblocks, Q80_BLOCK_SIZE];
    assert_eq!(
        Q80View::new(
            Q80Version::V1,
            &p,
            &byteoverflow,
            Q80Limits::new(2, usize::MAX, usize::MAX, usize::MAX)
        ),
        Err(Q80Error::ByteProductOverflow),
    );
}

#[test]
fn wrong_version_tag_rejected() {
    let p = block(0x3C00, [0; 32]);
    for tag in [0u8, 2, 42, 255] {
        assert_eq!(
            Q80View::from_tag(tag, &p, &[32], lim()),
            Err(Q80Error::WrongVersionTag(tag)),
            "tag {tag}"
        );
    }
    assert!(
        Q80View::from_tag(1, &p, &[32], lim()).is_ok(),
        "tag 1 accepted"
    );
}

#[test]
fn quantized_identity_path_stays_rejected() {
    // Real identity rejection (not a tautology): declaring Quantized in a
    // manifest entry must fail validate_artifact even for a valid file. The
    // fixture entry is SELF-HASHED integrity only, not trusted provenance.
    let file = st_file();
    let mut entry: ManifestEntry = entry_for(&file, "model.safetensors");
    entry.format = ArtifactFormat::Quantized;
    let err = validate_artifact(&file, "model.safetensors", &entry, &art_limits())
        .err()
        .expect("Quantized declaration must be rejected");
    assert_eq!(
        err,
        IdentityError::UnsupportedFormat(ArtifactFormat::Quantized)
    );
}
