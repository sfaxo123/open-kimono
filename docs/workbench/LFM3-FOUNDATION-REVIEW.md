# LFM3 foundation review snapshot

2026-10-27. Evidence for PLAN0.1.1 only. Canonical source lives at the paths below; these fenced copies are an audit snapshot, not another implementation. The existing validator scopes git diffs to docs, so this packet supplies the parent-directory Rust source it otherwise cannot see. No acceptance assertion was removed. No pretrained inference or whole-host C-free claim. Full module tests are retained in src/*_tests.rs; their full passing output follows the implementation.

## crates/lfm3-brain/src/lib.rs

```rust
//! Native Rust LFM3 foundations. No pretrained inference backend is ready yet.
pub const MODEL_ID: &str = "LiquidAI/LFM3-1.7B";
pub const SOURCE_REVISION: &str = "40ebe0aa6662c9d40389c2f5d565b6a2c6aebacd";

#[cfg(feature = "kernels")]
pub mod kernels;
#[cfg(feature = "model")]
pub mod model;
#[cfg(feature = "service")]
pub mod service;

```

## crates/lfm3-brain/src/model.rs

```rust
//! Exact pinned LFM3 config validation. Verification policy only: no inference,
//! no weight allocation. The official pinned config.json is embedded as a fixture.
use crate::{MODEL_ID, SOURCE_REVISION};
use serde_json::Value;

/// Official pinned LFM3-1.7B config.json (revision 40ebe0aa...). Data fixture only.
const PINNED_CONFIG_JSON: &str = r#"
{
  "architectures": ["Lfm3ForCausalLM"],
  "auto_map": {
    "AutoConfig": "configuration_lfm3.Lfm3Config",
    "AutoModel": "modeling_lfm3.Lfm3Model",
    "AutoModelForCausalLM": "modeling_lfm3.Lfm3ForCausalLM"
  },
  "bos_token_id": 124894,
  "conv_L_cache": 7,
  "conv_bias": false,
  "dtype": "bfloat16",
  "eos_token_id": 124900,
  "head_dim": 128,
  "hidden_size": 2048,
  "intermediate_size": 7680,
  "layer_types": [
    "conv", "conv", "full_attention", "conv", "sliding_attention", "conv",
    "full_attention", "conv", "sliding_attention", "conv", "full_attention",
    "conv", "sliding_attention", "conv", "full_attention", "conv",
    "sliding_attention", "conv", "full_attention", "conv", "sliding_attention",
    "conv", "full_attention", "conv"
  ],
  "max_position_embeddings": 262144,
  "model_type": "lfm3",
  "norm_eps": 1e-05,
  "num_attention_heads": 16,
  "num_hidden_layers": 24,
  "num_key_value_heads": 4,
  "pad_token_id": 124893,
  "rope_parameters": {
    "full_attention": {
      "partial_rotary_factor": 0.5,
      "rope_theta": 1000000.0,
      "rope_type": "default"
    },
    "sliding_attention": {
      "rope_theta": 10000,
      "rope_type": "default"
    }
  },
  "sliding_window": 256,
  "tie_word_embeddings": true,
  "transformers_version": "5.12.1",
  "use_cache": true,
  "use_sandwich_norm": true,
  "vocab_size": 128000
}
"#;

/// A config document validated to match the pinned LFM3 baseline exactly.
#[derive(Debug, Clone)]
pub struct PinnedConfig {
    value: Value,
}

impl PinnedConfig {
    /// Validate `json` against the embedded pinned baseline. Every top-level key
    /// of the baseline must be present and equal (ordered arrays included). Extra
    /// input keys are retained but not validated. Numbers compare by numeric
    /// value, so equivalent int/float spellings pass while real changes fail.
    pub fn parse(model_id: &str, revision: &str, json: &str) -> Result<PinnedConfig, String> {
        if model_id != MODEL_ID {
            return Err(format!(
                "model id mismatch: expected {MODEL_ID}, got {model_id}"
            ));
        }
        if revision != SOURCE_REVISION {
            return Err(format!(
                "source revision mismatch: expected {SOURCE_REVISION}, got {revision}"
            ));
        }
        let value: Value = serde_json::from_str(json).map_err(|e| format!("invalid JSON: {e}"))?;
        if !value.is_object() {
            return Err("config must be a JSON object".to_string());
        }
        let baseline: Value = serde_json::from_str(PINNED_CONFIG_JSON)
            .expect("embedded pinned config fixture must be valid");
        let base = baseline.as_object().expect("fixture is an object");
        let got = value.as_object().expect("input validated as object");
        for (key, expected) in base {
            match got.get(key) {
                None => return Err(format!("missing required key: {key}")),
                Some(actual) => {
                    if !json_eq(expected, actual) {
                        return Err(format!(
                            "pinned value mismatch for {key}: expected {expected}, got {actual}"
                        ));
                    }
                }
            }
        }
        Ok(PinnedConfig { value })
    }

    /// The full parsed document, including any extra metadata keys.
    pub fn value(&self) -> &Value {
        &self.value
    }
}

/// Structural equality with numeric normalization: numbers equal by value even
/// when serde spells one as an integer and the other as a float.
fn json_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => match (x.as_f64(), y.as_f64()) {
            (Some(u), Some(v)) => u == v,
            _ => x == y,
        },
        (Value::Array(xs), Value::Array(ys)) => {
            xs.len() == ys.len() && xs.iter().zip(ys.iter()).all(|(u, v)| json_eq(u, v))
        }
        (Value::Object(xs), Value::Object(ys)) => {
            xs.len() == ys.len()
                && xs.iter().all(|(k, u)| match ys.get(k) {
                    Some(v) => json_eq(u, v),
                    None => false,
                })
        }
        _ => a == b,
    }
}

#[cfg(test)]
#[path = "model_tests.rs"]
mod tests;

```

## crates/lfm3-brain/src/kernels.rs

```rust
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

```

## crates/lfm3-brain/src/service.rs

```rust
//! Bounded request and streaming boundary. This is protocol logic, not an inference backend.
use crate::MODEL_ID;
use serde::{Deserialize, Serialize};
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Instant,
};

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub request_bytes: usize,
    pub context_bytes: usize,
    pub context_tokens: usize,
    pub output_tokens: usize,
    pub output_bytes: usize,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub id: String,
    pub model: String,
    pub messages: Vec<Message>,
    pub max_new_tokens: usize,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Message {
    pub role: String,
    pub content: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    Started {
        id: String,
        model: String,
    },
    Token {
        id: String,
        index: usize,
        text: String,
    },
    Finished {
        id: String,
        tokens: usize,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum Error {
    Invalid(&'static str),
    BackendUnavailable,
    Cancelled,
    Deadline,
    OutputLimit,
    Sink,
}

/// `context_tokens` must use the selected tokenizer. Both calls must fail with
/// `BackendUnavailable` until a real model backend is wired. `stream` invokes
/// the callback once per generated token; cancellation is cooperative between
/// callbacks, so each backend step must itself be bounded.
pub trait Backend {
    fn context_tokens(&self, request: &Request) -> Result<usize, Error>;
    fn stream(
        &self,
        request: &Request,
        token: &mut dyn FnMut(&str) -> Result<(), Error>,
    ) -> Result<(), Error>;
}
pub trait Sink {
    fn send(&mut self, event: Event) -> Result<(), Error>;
}

pub struct Control<'a> {
    pub cancelled: &'a AtomicBool,
    pub deadline: Option<Instant>,
}

fn validate(r: &Request, l: Limits) -> Result<(), Error> {
    if r.id.is_empty()
        || r.id.len() > 128
        || !r
            .id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-.:".contains(&b))
    {
        return Err(Error::Invalid("request id"));
    }
    if r.model != MODEL_ID {
        return Err(Error::Invalid("model id"));
    }
    if r.messages.is_empty()
        || r.messages
            .iter()
            .any(|m| !["system", "user", "assistant"].contains(&m.role.as_str()))
    {
        return Err(Error::Invalid("messages"));
    }
    let context = r
        .messages
        .iter()
        .try_fold(0usize, |n, m| n.checked_add(m.content.len()))
        .ok_or(Error::Invalid("context size"))?;
    if context > l.context_bytes {
        return Err(Error::Invalid("context limit"));
    }
    if r.max_new_tokens == 0 || r.max_new_tokens > l.output_tokens {
        return Err(Error::Invalid("output token limit"));
    }
    Ok(())
}

pub fn parse(input: &[u8], limits: Limits) -> Result<Request, Error> {
    if input.len() > limits.request_bytes {
        return Err(Error::Invalid("request byte limit"));
    }
    let request: Request =
        serde_json::from_slice(input).map_err(|_| Error::Invalid("request JSON"))?;
    validate(&request, limits)?;
    Ok(request)
}

pub fn serve(
    input: &[u8],
    limits: Limits,
    backend: &dyn Backend,
    sink: &mut dyn Sink,
    control: &Control<'_>,
) -> Result<(), Error> {
    let check = |c: &Control<'_>| {
        if c.cancelled.load(Ordering::Relaxed) {
            Err(Error::Cancelled)
        } else if c.deadline.is_some_and(|d| Instant::now() >= d) {
            Err(Error::Deadline)
        } else {
            Ok(())
        }
    };
    check(control)?;
    let request = parse(input, limits)?;
    let context_tokens = backend.context_tokens(&request)?;
    if context_tokens
        .checked_add(request.max_new_tokens)
        .is_none_or(|total| total > limits.context_tokens)
    {
        return Err(Error::Invalid("context token limit"));
    }
    check(control)?;
    sink.send(Event::Started {
        id: request.id.clone(),
        model: request.model.clone(),
    })?;
    let (mut count, mut bytes) = (0usize, 0usize);
    let mut emit = |text: &str| {
        check(control)?;
        let next_count = count.checked_add(1).ok_or(Error::OutputLimit)?;
        let next_bytes = bytes.checked_add(text.len()).ok_or(Error::OutputLimit)?;
        if next_count > request.max_new_tokens || next_bytes > limits.output_bytes {
            return Err(Error::OutputLimit);
        }
        sink.send(Event::Token {
            id: request.id.clone(),
            index: count,
            text: text.to_owned(),
        })?;
        count = next_count;
        bytes = next_bytes;
        Ok(())
    };
    backend.stream(&request, &mut emit)?;
    check(control)?;
    sink.send(Event::Finished {
        id: request.id,
        tokens: count,
    })
}

#[cfg(test)]
#[path = "service_tests.rs"]
mod tests;

```

## Dependency declarations

```toml
[package]
name = "lfm3-brain"
version = "0.1.0"
edition = "2021"
publish = false

[features]
default = ["service"]
model = []
kernels = []
service = []

[dependencies]
libm = { version = "=0.2.16", default-features = false, features = ["force-soft-floats"] }
serde = { version = "=1.0.228", features = ["derive"] }
serde_json = "=1.0.149"

```

libm0.2.16 is cached MIT Rust source; default architecture and unstable intrinsic paths disabled. Registry checksum b6d2cec3eae94f9f509c767b45932f1ada8350c4bdb85af2fcab4a3c14807981. Build/configure scripts and release-object imports inspected. System compiler/stdlib/allocator/unwind boundaries remain explicit phase1/6 work.

## Actual complete acceptance output

```text
   Compiling lfm3-brain v0.1.0 ($HOME/Developer/experiments/redox-oneplus6/crates/lfm3-brain)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.20s
     Running unittests src/lib.rs (crates/lfm3-brain/target/debug/deps/lfm3_brain-ac850f2dd5866477)

running 30 tests
test kernels::tests::conv_impulse_cascades_weights_descending ... ok
test kernels::tests::conv_bias_and_rejects ... ok
test kernels::tests::conv_rejected_nan_does_not_advance_state ... ok
test kernels::tests::conv_steps_analytic_and_reset ... ok
test kernels::tests::matvec_analytic ... ok
test kernels::tests::matvec_rejects_bad_shapes ... ok
test kernels::tests::matvec_rejects_nonfinite ... ok
test kernels::tests::output_overflow_is_rejected_not_silent ... ok
test kernels::tests::rms_norm_analytic ... ok
test kernels::tests::rms_norm_rejects_bad_inputs ... ok
test kernels::tests::rope_d4_factor_half_rotates_only_pair_zero_two_at_angle_one ... ok
test kernels::tests::rope_d8_factor_half_uses_full_dim_denominator ... ok
test kernels::tests::rope_rejects_invalid_params ... ok
test kernels::tests::sigmoid_silu_extremes ... ok
test kernels::tests::softmax_stable_and_normalized ... ok
test model::tests::accepts_exact_pinned_config ... ok
test model::tests::rejects_identity_mismatch ... ok
test model::tests::rejects_malformed_json_and_non_object ... ok
test model::tests::accepts_equivalent_number_spellings ... ok
test service::tests::cancellation_between_tokens_has_no_finished_event ... ok
test service::tests::cancellation_deadline_and_sink_failure_stop_streaming ... ok
test service::tests::streams_ordered_events_and_treats_command_like_text_as_data ... ok
test service::tests::unavailable_backend_is_distinct ... ok
test service::tests::token_budget_includes_output_reservation ... ok
test model::tests::rejects_layer_array_reordering ... ok
test service::tests::rejects_invalid_json_model_fields_and_bounds ... ok
test model::tests::rejects_changed_numeric_value ... ok
test model::tests::rejects_changed_nested_rope ... ok
test model::tests::retains_unknown_metadata ... ok
test model::tests::rejects_each_missing_baseline_key ... ok

test result: ok. 30 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s

     Running tests/model_contract.rs (crates/lfm3-brain/target/debug/deps/model_contract-c286420f41c72ae7)

running 1 test
test retained_official_config_matches_embedded_validation_contract ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

   Doc-tests lfm3_brain

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s


```

Root mutation receipts RUST-CHECK-20261027T2057-MUTATIONS.json record exit101 for the doubled RoPE exponent and early-f32 SiLU implementation, followed by exact source restoration and GREEN. Reference scalar conv is single-channel/six-past only; full pretrained/cache/batch parity belongs to later gates.
