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
