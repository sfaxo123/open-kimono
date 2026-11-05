// Contract acceptance tests (part 1/2): binary16 conversion, scales, signed
// quants. Independent arithmetic half reference + literal expected vectors;
// mirrored bugs must not pass. Rust-only fixture logic. Explicit imports only.

#[path = "quantized/common.rs"]
mod q8common;

use lfm3_brain::quantized::{f16_to_f32, Q80Error, Q80Version, Q80View, Q80_BLOCK_SIZE};
use q8common::block;
use q8common::half_ref;
use q8common::lim;

#[test]
fn half_reference_literals() {
    let cases: [(u16, f64); 9] = [
        (0x0000, 0.0),
        (0x8000, -0.0),
        (0x3C00, 1.0),
        (0xBC00, -1.0),
        (0x3800, 0.5),
        (0x7BFF, 65504.0),               // max finite f16
        (0x0001, 5.960464477539063e-8),  // min subnormal
        (0x03FF, 6.097555160522461e-5),  // max subnormal
        (0x8001, -5.960464477539063e-8), // negative min subnormal
    ];
    for (bits, want) in cases {
        let got = f16_to_f32(bits).expect("finite encoding must convert");
        assert_eq!(got as f64, want, "literal bits {bits:#06x}");
        assert_eq!(
            half_ref(bits),
            Some(want),
            "reference agrees for {bits:#06x}"
        );
    }
    // Sign-bit mutation must flip the value (test goes RED if sign ignored).
    assert_eq!(f16_to_f32(0xC000), Some(-2.0)); // -2.0: sign 1, exp 0x10, frac 0
    assert_ne!(f16_to_f32(0xC000), f16_to_f32(0x4000));
}

#[test]
fn half_exhaustive_65536() {
    let mut finite = 0usize;
    for bits in 0u16..=u16::MAX {
        match half_ref(bits) {
            Some(want) => {
                finite += 1;
                let got = f16_to_f32(bits).expect("finite encoding must convert");
                assert_eq!(got as f64, want, "bits {bits:#06x}");
                assert_eq!(
                    f32::to_bits(got),
                    (want as f32).to_bits(),
                    "exact bits {bits:#06x}"
                );
            }
            None => assert_eq!(
                f16_to_f32(bits),
                None,
                "nonfinite bits {bits:#06x} rejected"
            ),
        }
    }
    assert_eq!(finite, 65536 - (2 * 1024), "all NaN/Inf encodings rejected");
}

#[test]
fn decode_signed_quants_literals() {
    let mut q = [0i8; Q80_BLOCK_SIZE];
    q[0] = 127;
    q[1] = -128;
    q[2] = -1;
    q[31] = 2;
    let p = block(0x3C00, q); // scale = 1.0
    let v = Q80View::new(Q80Version::V1, &p, &[32], lim()).unwrap();
    let d = v.decode_block(0).unwrap();
    assert_eq!(d[0], 127.0);
    assert_eq!(d[1], -128.0);
    assert_eq!(d[2], -1.0);
    assert_eq!(d[31], 2.0);
    assert_eq!(v.quant(0, 1).unwrap(), -128);
    assert_eq!(v.scale_f32(0).unwrap(), 1.0);
}

#[test]
fn quant_sign_mutation_is_red() {
    // Flipping the top bit of a stored quant byte must change the decode.
    let q = [0x7Fi8; Q80_BLOCK_SIZE];
    let mut p = block(0x3C00, q);
    let clean = Q80View::new(Q80Version::V1, &p, &[32], lim()).unwrap();
    assert_eq!(clean.decode_block(0).unwrap()[5], 127.0);
    p[2 + 5] ^= 0x80; // 0x7F -> 0xFF = -1 as i8
    let mutated = Q80View::new(Q80Version::V1, &p, &[32], lim()).unwrap();
    assert_eq!(mutated.decode_block(0).unwrap()[5], -1.0);
}

#[test]
fn scale_classes_accepted() {
    // negative, signed zero, subnormal, max-finite scales are all valid.
    for bits in [0x8400u16, 0x0000, 0x8000, 0x0001, 0x03FF, 0x7BFF] {
        let p = block(bits, [1i8; Q80_BLOCK_SIZE]);
        let v = Q80View::new(Q80Version::V1, &p, &[32], lim()).unwrap();
        let s = v.scale_f32(0).unwrap();
        assert!(s.is_finite(), "bits {bits:#06x} must be finite");
        assert_eq!(v.decode_block(0).unwrap()[7], s);
    }
    assert!(
        f16_to_f32(0x8400).unwrap() < 0.0,
        "negative scale keeps sign"
    );
    let zbuf = block(0x8000, [5; 32]);
    let z = Q80View::new(Q80Version::V1, &zbuf, &[32], lim()).unwrap();
    assert!(
        z.scale_f32(0).unwrap().is_sign_negative(),
        "signed zero scale"
    );
    let sbuf = block(0x0001, [1; 32]);
    let sub = Q80View::new(Q80Version::V1, &sbuf, &[32], lim()).unwrap();
    assert_eq!(
        sub.decode_element(0).unwrap(),
        5.960464477539063e-8,
        "subnormal preserved"
    );
}

#[test]
fn all_nonfinite_scales_rejected_by_constructor() {
    // Every NaN/Inf binary16 encoding (exponent all-ones: 0x7C00..0x7FFF and
    // 0xFC00..0xFFFF, 2048 total across both signs) must be rejected by
    // Q80View::new itself (constructor scale scan), not only the conversion
    // helper.
    let mut checked = 0usize;
    for hi in [0x7Cu8, 0x7D, 0x7E, 0x7F, 0xFC, 0xFD, 0xFE, 0xFF] {
        for lo in 0u8..=u8::MAX {
            let bits = (u16::from(hi) << 8) | u16::from(lo);
            assert_eq!(
                (bits >> 10) & 0x1F,
                0x1F,
                "prefix keeps all NaN/Inf encodings"
            );
            let p = block(bits, [0i8; Q80_BLOCK_SIZE]);
            assert_eq!(
                Q80View::new(Q80Version::V1, &p, &[32], lim()),
                Err(Q80Error::NonFiniteScale),
                "nonfinite scale {bits:#06x}"
            );
            checked += 1;
        }
    }
    assert_eq!(
        checked, 2048,
        "all NaN/Inf encodings covered via constructor"
    );
}

#[test]
fn out_of_range_access_and_repeat_parse() {
    let p = block(0x3C00, [0; 32]);
    let v = Q80View::new(Q80Version::V1, &p, &[32], lim()).unwrap();
    assert_eq!(
        v.decode_block(1),
        Err(Q80Error::IndexOutOfRange { index: 1, len: 1 })
    );
    assert_eq!(
        v.scale_bits(1),
        Err(Q80Error::IndexOutOfRange { index: 1, len: 1 })
    );
    assert_eq!(
        v.quant(0, 32),
        Err(Q80Error::IndexOutOfRange {
            index: 32,
            len: Q80_BLOCK_SIZE
        })
    );
    assert_eq!(
        v.decode_element(32),
        Err(Q80Error::IndexOutOfRange { index: 32, len: 32 })
    );
    // Repeated parse/decode workload of the same payload must stay stable.
    for _ in 0..10_000 {
        let w = Q80View::new(Q80Version::V1, &p, &[32], lim()).unwrap();
        assert_eq!(w.decode_block(0).unwrap(), [0.0f32; Q80_BLOCK_SIZE]);
    }
}
