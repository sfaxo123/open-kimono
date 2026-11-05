//! Independent analytic fixtures for the scalar reference kernels.
//! No PyTorch parity claim; values are hand-computed.

use super::*;

const TOL: f32 = 1e-6;

fn approx(a: &[f32], b: &[f32]) {
    assert_eq!(a.len(), b.len(), "length mismatch: {a:?} vs {b:?}");
    for (x, y) in a.iter().zip(b) {
        // inf <= inf would pass the relative test; both sides must be finite.
        assert!(
            x.is_finite() && y.is_finite(),
            "non-finite side: got {a:?}, want {b:?}"
        );
        assert!(
            (x - y).abs() <= TOL * y.abs().max(1.0),
            "expected {b:?}, got {a:?}"
        );
    }
}

/// Compare possibly-subnormal f32 values within at most ONE min-subnormal ULP
/// (f32::from_bits(1)); relative/absolute tolerances are meaningless subnormally.
fn approx_ulp1(a: f32, b: f32) {
    assert!(
        a.is_finite() && b.is_finite(),
        "non-finite side: got {a}, want {b}"
    );
    let bits = |v: f32| i64::from(v.to_bits());
    assert!(
        (bits(a) - bits(b)).abs() <= 1,
        "{a} vs {b} differ by more than one min-subnormal ULP"
    );
}

#[test]
fn matvec_analytic() {
    let y = matvec(&[1.0, 2.0, 3.0, 4.0], 2, 2, &[5.0, 6.0]).unwrap();
    approx(&y, &[17.0, 39.0]);
}

#[test]
fn matvec_rejects_bad_shapes() {
    assert!(matvec(&[1.0, 2.0, 3.0, 4.0], 2, 2, &[5.0]).is_err()); // short x
    assert!(matvec(&[1.0, 2.0, 3.0], 2, 2, &[5.0, 6.0]).is_err()); // short matrix
    assert!(matvec(&[1.0], 0, 1, &[1.0]).is_err()); // zero rows
    assert!(matvec(&[1.0, 2.0], 2, 2, &[5.0, 6.0]).is_err()); // cols mismatch
    let huge = matvec(&[], usize::MAX, 2, &[1.0, 2.0]);
    assert!(huge.is_err()); // rows*cols overflow before allocation
}

#[test]
fn matvec_rejects_nonfinite() {
    assert!(matvec(&[f32::NAN, 2.0, 3.0, 4.0], 2, 2, &[5.0, 6.0]).is_err());
    assert!(matvec(&[1.0, 2.0, 3.0, 4.0], 2, 2, &[f32::INFINITY, 6.0]).is_err());
}

#[test]
fn rms_norm_analytic() {
    // x=[3,4]: mean(x^2)=12.5; eps=0.5 -> inv = 1/sqrt(13).
    let y = rms_norm(&[3.0, 4.0], &[1.0, 1.0], 0.5).unwrap();
    let inv = 1.0f32 / 13.0f32.sqrt();
    approx(&y, &[3.0 * inv, 4.0 * inv]);
    // weights apply per element: doubling weight 1 doubles output 0.
    let w = rms_norm(&[3.0, 4.0], &[2.0, 1.0], 0.5).unwrap();
    approx(&w, &[6.0 * inv, 4.0 * inv]);
}

#[test]
fn rms_norm_rejects_bad_inputs() {
    assert!(rms_norm(&[], &[1.0], 1e-6).is_err()); // empty
    assert!(rms_norm(&[3.0, 4.0], &[1.0], 1e-6).is_err()); // weight len
    assert!(rms_norm(&[3.0, 4.0], &[1.0, 1.0], 0.0).is_err()); // eps = 0
    assert!(rms_norm(&[3.0, 4.0], &[1.0, 1.0], -1.0).is_err()); // eps < 0
    assert!(rms_norm(&[3.0, 4.0], &[1.0, 1.0], f32::NAN).is_err()); // eps NaN
    assert!(rms_norm(&[f32::NAN, 4.0], &[1.0, 1.0], 1e-6).is_err());
}

#[test]
fn softmax_stable_and_normalized() {
    let y = softmax(&[1000.0, 1000.0]).unwrap();
    approx(&y, &[0.5, 0.5]);
    let z = softmax(&[1.0, 2.0, 3.0]).unwrap();
    let e = (std::f32::consts::E as f64).powf(1.0) as f32; // exp of shifted [..] ratios
    let s = 1.0 + e + e * e;
    approx(&z, &[1.0 / s, e / s, e * e / s]);
    assert!(softmax(&[]).is_err());
    assert!(softmax(&[1.0, f32::NAN]).is_err());
}

#[test]
fn sigmoid_silu_extremes() {
    assert!((sigmoid(1000.0).unwrap() - 1.0).abs() <= TOL);
    let neg = sigmoid(-1000.0).unwrap();
    assert!(neg >= 0.0 && neg.is_finite()); // underflows toward 0, never NaN/inf
    assert!((sigmoid(0.0).unwrap() - 0.5).abs() <= TOL);
    assert!(sigmoid(f32::NAN).is_err());
    assert!((silu(1000.0).unwrap() - 1000.0).abs() <= 1e-3);
    // silu(-large) -> ~0 from above (x * ~0), never NaN/inf.
    let s = silu(-1000.0).unwrap();
    assert!(s.is_finite() && s >= -TOL && s <= 0.0);
    // SiLU regression: at x=-104 a bare sigmoid cast to f32 underflows to 0,
    // but the true product ~ -7.1e-44 is representable as an f32 subnormal;
    // the f64-intermediate reference must not lose it. Independent analytic
    // expected: -104 / (1 + exp(104)) in f64 (NOT -104/exp(-104), which is -inf).
    let s104 = silu(-104.0).unwrap();
    assert!(
        s104 < 0.0 && s104.is_subnormal(),
        "silu(-104) lost to underflow: {s104}"
    );
    let expected = (-104.0f64 / (1.0 + 104.0f64.exp())) as f32;
    assert!(
        expected.is_subnormal() && expected < 0.0,
        "fixture sanity: {expected}"
    );
    approx_ulp1(s104, expected);
    // The early-f32-sigmoid variant (x * (sigmoid(x) as f32)) is RED here:
    // its sigmoid(-104) rounds to 0f32, so the product collapses to -0.0.
    let buggy = {
        let d = f64::from(-104.0f32);
        let s_f32 = (d.exp() / (1.0 + d.exp())) as f32; // underflowed to 0 in f32
        assert_eq!(s_f32, 0.0);
        (-104.0f32) * s_f32
    };
    assert_ne!(
        s104, buggy,
        "fixed version must differ from early-f32-sigmoid"
    );
    approx_ulp1(buggy, -0.0); // documents the RED behaviour: buggy == -0.0
    assert!(silu(f32::INFINITY).is_err());
}

#[test]
fn rope_d4_factor_half_rotates_only_pair_zero_two_at_angle_one() {
    // D=4, factor=0.5 -> angles = floor(2)/2 = 1 rotated pair (indices 0 and 2);
    // theta=1 (any) with position*freq chosen so angle = 1 via freq=1: theta^0=1.
    let x = [1.0f32, 7.0, 2.0, 9.0];
    let y = partial_rope(&x, 1, 1.0, 0.5).unwrap();
    // indices 1 and 3 (zero frequency) exactly unchanged:
    assert_eq!(y[1], 7.0);
    assert_eq!(y[3], 9.0);
    // pair (0,2) rotated by angle 1*1 = 1 radian, rotate_half pairing:
    let (c, s) = (1.0f64.cos(), 1.0f64.sin());
    approx(
        &[y[0], y[2]],
        &[(1.0 * c - 2.0 * s) as f32, (2.0 * c + 1.0 * s) as f32],
    );
}

#[test]
fn rope_d8_factor_half_uses_full_dim_denominator() {
    // D=8, factor=0.5 -> angles = floor(4)/2 = 2 rotated pairs (0,4) and (1,5).
    // k=1 frequency must be theta^(-2*1/8) = 16^(-0.25) = 0.5, i.e. the angle at
    // position 1 is 0.5 rad. A wrong half-D denominator (2k/(D/2)) would give
    // 16^(-0.5) = 0.25 rad -> cos(0.25)=0.9689 detects the bug.
    let x = [0.0f32, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
    let y = partial_rope(&x, 1, 16.0, 0.5).unwrap();
    // pair (1,5): x1=1 at index 1, x2=0 -> out[1] = cos(0.5), out[5] = sin(0.5).
    approx(&[y[1]], &[0.5f64.cos() as f32]);
    approx(&[y[5]], &[0.5f64.sin() as f32]);
    assert!((y[1] - 0.9689124).abs() > 1e-3); // half-D denominator would land here
                                              // zero tail: pairs (2,6),(3,7) have zero frequency -> unchanged:
    assert_eq!(y[2], 0.0);
    assert_eq!(y[3], 0.0);
    assert_eq!(y[4], 0.0); // rotated by k=0 angle 1.0 with x=0 stays 0
    assert_eq!(y[6], 0.0);
    assert_eq!(y[7], 0.0);
}

#[test]
fn rope_rejects_invalid_params() {
    assert!(partial_rope(&[], 0, 1e4, 0.5).is_err()); // empty D
    assert!(partial_rope(&[1.0, 2.0, 3.0], 0, 1e4, 0.5).is_err()); // odd D
    assert!(partial_rope(&[1.0, 2.0], MAX_POSITION, 1e4, 0.5).is_err()); // position
    assert!(partial_rope(&[1.0, 2.0], 0, -1.0, 0.5).is_err()); // theta <= 0
    assert!(partial_rope(&[1.0, 2.0], 0, f64::NAN, 0.5).is_err()); // theta NaN
    assert!(partial_rope(&[1.0, 2.0], 0, 1e4, 1.5).is_err()); // factor > 1
    assert!(partial_rope(&[1.0, 2.0], 0, 1e4, -0.1).is_err()); // factor < 0
    assert!(partial_rope(&[f32::NAN, 2.0], 0, 1e4, 0.5).is_err()); // nonfinite x
}

#[test]
fn conv_impulse_cascades_weights_descending() {
    let mut c = CausalConv::new(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0], None).unwrap();
    let got: Vec<f32> = (0..8)
        .map(|i| c.step(if i == 0 { 1.0 } else { 0.0 }).unwrap())
        .collect();
    // impulse at t=0 uses weight[6]=7, then slides down to weight[0]=1, then 0.
    approx(&got, &[7.0, 6.0, 5.0, 4.0, 3.0, 2.0, 1.0, 0.0]);
}

#[test]
fn conv_steps_analytic_and_reset() {
    let mut c = CausalConv::new(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0], None).unwrap();
    approx(&[c.step(1.0).unwrap()], &[7.0]); // 1*7
    approx(&[c.step(2.0).unwrap()], &[20.0]); // 1*6 + 2*7 = 20
    approx(&[c.step(3.0).unwrap()], &[38.0]); // 1*5 + 2*6 + 3*7 = 38
    c.reset();
    approx(&[c.step(1.0).unwrap()], &[7.0]); // state zeroed
}

#[test]
fn conv_bias_and_rejects() {
    let mut c = CausalConv::new(&[0.0; 7], Some(2.5)).unwrap();
    approx(&[c.step(1.0).unwrap()], &[2.5]);
    assert!(CausalConv::new(&[1.0; 6], None).is_err()); // 6 weights
    assert!(CausalConv::new(&[1.0; 8], None).is_err()); // 8 weights
    assert!(CausalConv::new(&[f32::NAN, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0], None).is_err());
    assert!(CausalConv::new(&[1.0; 7], Some(f32::NAN)).is_err());
}

#[test]
fn conv_rejected_nan_does_not_advance_state() {
    let mut c = CausalConv::new(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0], None).unwrap();
    approx(&[c.step(1.0).unwrap()], &[7.0]);
    assert!(c.step(f32::NAN).is_err());
    // state unchanged: next real step matches the no-NAN sequence (2*6 + ... pattern)
    approx(&[c.step(2.0).unwrap()], &[20.0]);
}

#[test]
fn output_overflow_is_rejected_not_silent() {
    // matvec: 1e30 * 1e30 overflows f32 range -> error, not inf.
    assert!(matvec(&[1e30], 1, 1, &[1e30]).is_err());
    // rms_norm with huge inputs still finite (normalized), sanity:
    let ok = rms_norm(&[f32::MAX, f32::MAX], &[1.0, 1.0], 1e-6).unwrap();
    assert!(ok.iter().all(|v| v.is_finite()));
    // conv overflow: saturating weights make the f64 sum exceed f32::MAX -> error,
    // and the state must be left unchanged.
    let mut c = CausalConv::new(&[1e38, 1e38, 1e38, 1e38, 1e38, 1e38, 1e38], None).unwrap();
    assert!(c.step(1e30).is_err()); // 7e68: not representable as f32
                                    // state was still all zeros (rejected step did not advance it):
    approx(&[c.step(1.0).unwrap()], &[1e38]); // only weight[6]*1 contributes
}
