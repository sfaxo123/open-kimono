//! Scalar reference kernels for LFM3 numerics: matvec, RMSNorm, softmax,
//! sigmoid/SiLU, partial RoPE (zero-tail convention) and a single-channel
//! width-7 causal conv decode step.
//!
//! Scope: single-channel scalar decode only. No packed-sequence prefill, no
//! batched/packed conv, no PyTorch bit parity, no pretrained inference.
//! Reference: pinned modeling_lfm3.py lines 51-68 (RMSNorm), 94-111 and
//! 160-165 (zero-tail partial RoPE + rotate_half), lfm3_conv.py `_decode`.
//! f64 intermediates; every output is validated finite and f32-representable.
//! Transcendentals use pinned Rust libm, with architecture/FFI paths disabled.

/// Max accepted RoPE position (matches the pinned 262144 context bound).
pub const MAX_POSITION: usize = 262_144;

/// Row-major matrix-vector product: `y = M x`, M is `rows x cols`.
pub fn matvec(matrix: &[f32], rows: usize, cols: usize, x: &[f32]) -> Result<Vec<f32>, String> {
    if rows == 0 || cols == 0 {
        return Err("matvec: rows and cols must be nonzero".into());
    }
    if x.len() != cols {
        return Err(format!(
            "matvec: x has {} elements, expected {cols}",
            x.len()
        ));
    }
    let n = rows
        .checked_mul(cols)
        .ok_or("matvec: shape overflows usize")?;
    if matrix.len() != n {
        return Err(format!(
            "matvec: matrix has {} elements, expected {n}",
            matrix.len()
        ));
    }
    if !matrix.iter().all(|v| v.is_finite()) || !x.iter().all(|v| v.is_finite()) {
        return Err("matvec: non-finite input".into());
    }
    let mut out = Vec::with_capacity(rows);
    for r in 0..rows {
        let mut acc = 0.0f64;
        for c in 0..cols {
            acc += f64::from(matrix[r * cols + c]) * f64::from(x[c]);
        }
        out.push(to_f32(acc, "matvec")?);
    }
    Ok(out)
}

/// Weighted RMSNorm: `out[i] = weights[i] * x[i] / sqrt(mean(x^2) + eps)`.
pub fn rms_norm(x: &[f32], weights: &[f32], eps: f32) -> Result<Vec<f32>, String> {
    if x.is_empty() {
        return Err("rms_norm: empty input".into());
    }
    if weights.len() != x.len() {
        return Err(format!(
            "rms_norm: {} weights for {} inputs",
            weights.len(),
            x.len()
        ));
    }
    if !(eps.is_finite() && eps > 0.0) {
        return Err("rms_norm: eps must be finite and > 0".into());
    }
    if !x.iter().all(|v| v.is_finite()) || !weights.iter().all(|v| v.is_finite()) {
        return Err("rms_norm: non-finite input".into());
    }
    let mut sum = 0.0f64;
    for v in x {
        let d = f64::from(*v);
        sum += d * d;
    }
    let inv = 1.0 / libm::sqrt(sum / x.len() as f64 + f64::from(eps));
    x.iter()
        .zip(weights)
        .map(|(v, w)| to_f32(f64::from(*w) * f64::from(*v) * inv, "rms_norm"))
        .collect()
}

/// Numerically stable softmax: subtracts the largest logit first.
pub fn softmax(x: &[f32]) -> Result<Vec<f32>, String> {
    if x.is_empty() {
        return Err("softmax: empty input".into());
    }
    if !x.iter().all(|v| v.is_finite()) {
        return Err("softmax: non-finite input".into());
    }
    let max = x.iter().fold(f32::NEG_INFINITY, |a, &b| a.max(b));
    let mut out: Vec<f32> = Vec::with_capacity(x.len());
    let mut sum = 0.0f64;
    for v in x {
        let e = libm::exp(f64::from(*v) - f64::from(max)) as f32;
        if !e.is_finite() {
            return Err("softmax: exp overflow".into());
        }
        sum += f64::from(e);
        out.push(e);
    }
    if !sum.is_finite() || sum <= 0.0 {
        return Err("softmax: sum overflow".into());
    }
    for v in out.iter_mut() {
        *v = to_f32(f64::from(*v) / sum, "softmax")?;
    }
    Ok(out)
}

/// Stable logistic sigmoid.
pub fn sigmoid(x: f32) -> Result<f32, String> {
    if !x.is_finite() {
        return Err("sigmoid: non-finite input".into());
    }
    let d = f64::from(x);
    let y = if d >= 0.0 {
        1.0 / (1.0 + libm::exp(-d))
    } else {
        let e = libm::exp(d);
        e / (1.0 + e)
    };
    to_f32(y, "sigmoid")
}

/// SiLU / swish: `x * sigmoid(x)`. The product is computed in f64 so tiny
/// subnormal results (e.g. silu(-104) ~ -7.1e-44) survive even though a bare
/// sigmoid cast to f32 would underflow to 0.
pub fn silu(x: f32) -> Result<f32, String> {
    if !x.is_finite() {
        return Err("silu: non-finite input".into());
    }
    let d = f64::from(x);
    let s = if d >= 0.0 {
        1.0 / (1.0 + libm::exp(-d))
    } else {
        let e = libm::exp(d);
        e / (1.0 + e)
    };
    to_f32(d * s, "silu")
}

/// Partial RoPE with LFM3's zero-tail convention (modeling_lfm3.py:105-110):
/// frequencies `theta^(-2k/D)` for `k < floor(factor*D)/2`, zero elsewhere;
/// rotate_half pairs index `i` with `i + D/2` (modeling_lfm3.py:160-165),
/// NOT adjacent pairs. No scaled-RoPE variants here.
pub fn partial_rope(
    x: &[f32],
    position: usize,
    theta: f64,
    factor: f64,
) -> Result<Vec<f32>, String> {
    let d = x.len();
    if d == 0 || d % 2 != 0 {
        return Err(format!(
            "partial_rope: head dim {d} must be nonzero and even"
        ));
    }
    if position >= MAX_POSITION {
        return Err(format!(
            "partial_rope: position {position} >= {MAX_POSITION}"
        ));
    }
    if !theta.is_finite() || theta <= 0.0 {
        return Err("partial_rope: theta must be finite and > 0".into());
    }
    if !factor.is_finite() || factor < 0.0 || factor > 1.0 {
        return Err("partial_rope: factor must be finite in [0, 1]".into());
    }
    if !x.iter().all(|v| v.is_finite()) {
        return Err("partial_rope: non-finite input".into());
    }
    // Validated nonnegative factor makes truncation equal floor, without a
    // platform math call. The saturated cast also bounds oversized products.
    let angles = (factor * d as f64) as usize / 2;
    let half = d / 2;
    let mut out = x.to_vec();
    for k in 0..half {
        let (lo, hi) = (k, k + half);
        let freq = if k < angles {
            libm::pow(theta, -(2.0 * k as f64) / d as f64)
        } else {
            0.0
        };
        let a = position as f64 * freq;
        let (c, s) = (libm::cos(a), libm::sin(a));
        let x1 = f64::from(x[lo]);
        let x2 = f64::from(x[hi]);
        out[lo] = to_f32(x1 * c - x2 * s, "partial_rope")?;
        out[hi] = to_f32(x2 * c + x1 * s, "partial_rope")?;
    }
    Ok(out)
}

/// Single-channel scalar reference for LFM3's width-7 causal depthwise conv
/// decode step (lfm3_conv.py `_decode` with K=6 state). Convolution window is
/// the 6 stored past samples plus the current one: current sample uses
/// `weights[6]`, oldest stored sample uses `weights[0]`. State holds the 6
/// past scalar values oldest -> newest. No packed sequences / multi-channel.
pub struct CausalConv {
    weights: [f32; 7],
    bias: Option<f32>,
    state: [f32; 6],
}

impl CausalConv {
    pub fn new(weights: &[f32], bias: Option<f32>) -> Result<Self, String> {
        if weights.len() != 7 {
            return Err(format!(
                "CausalConv: expected 7 weights, got {}",
                weights.len()
            ));
        }
        if !weights.iter().all(|v| v.is_finite()) {
            return Err("CausalConv: non-finite weight".into());
        }
        if let Some(b) = bias {
            if !b.is_finite() {
                return Err("CausalConv: non-finite bias".into());
            }
        }
        let mut w = [0.0f32; 7];
        w.copy_from_slice(weights);
        Ok(Self {
            weights: w,
            bias,
            state: [0.0; 6],
        })
    }

    /// Advance one sample. On any error (non-finite input or non-representable
    /// output) the state is left unchanged.
    pub fn step(&mut self, x: f32) -> Result<f32, String> {
        if !x.is_finite() {
            return Err("CausalConv: non-finite input".into());
        }
        let mut acc = self.bias.map(f64::from).unwrap_or(0.0);
        for k in 0..6 {
            acc += f64::from(self.state[k]) * f64::from(self.weights[k]);
        }
        acc += f64::from(x) * f64::from(self.weights[6]);
        let y = to_f32(acc, "CausalConv")?;
        self.state.rotate_left(1);
        self.state[5] = x;
        Ok(y)
    }

    pub fn reset(&mut self) {
        self.state = [0.0; 6];
    }
}

/// Convert an f64 intermediate to a finite, f32-representable value.
fn to_f32(v: f64, what: &str) -> Result<f32, String> {
    if !v.is_finite() {
        return Err(format!("{what}: non-finite result"));
    }
    let y = v as f32;
    if !y.is_finite() || (y as f64).abs() > f32::MAX as f64 {
        return Err(format!("{what}: result not representable as f32"));
    }
    Ok(y)
}

#[cfg(test)]
#[path = "kernels_tests.rs"]
mod tests;
