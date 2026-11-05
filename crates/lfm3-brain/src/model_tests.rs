use super::*;
use crate::{MODEL_ID, SOURCE_REVISION};

const GOOD: &str = r#"{
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
}"#;

fn parse(json: &str) -> Result<PinnedConfig, String> {
    PinnedConfig::parse(MODEL_ID, SOURCE_REVISION, json)
}

fn mutate(f: impl Fn(&mut Value)) -> String {
    let mut v: Value = serde_json::from_str(GOOD).unwrap();
    f(&mut v);
    v.to_string()
}

#[test]
fn accepts_exact_pinned_config() {
    let c = parse(GOOD).expect("pinned config must validate");
    assert_eq!(c.value()["num_hidden_layers"], 24);
    assert_eq!(c.value()["layer_types"].as_array().unwrap().len(), 24);
}

#[test]
fn accepts_equivalent_number_spellings() {
    // rope_theta 1000000.0 spelled as int, norm_eps as plain decimal: same value.
    let j = mutate(|v| {
        v["rope_parameters"]["full_attention"]["rope_theta"] = serde_json::json!(1000000);
        v["norm_eps"] = serde_json::json!(0.00001);
    });
    assert!(parse(&j).is_ok());
}

#[test]
fn retains_unknown_metadata() {
    let j = mutate(|v| v["custom_note"] = serde_json::json!("phone build 7"));
    let c = parse(&j).expect("extra metadata must be retained");
    assert_eq!(c.value()["custom_note"], "phone build 7");
}

#[test]
fn rejects_each_missing_baseline_key() {
    let keys: Vec<String> = {
        let v: Value = serde_json::from_str(GOOD).unwrap();
        v.as_object().unwrap().keys().cloned().collect()
    };
    assert_eq!(keys.len(), 25);
    for k in keys {
        let j = mutate(|v| {
            v.as_object_mut().unwrap().remove(&k);
        });
        let e = parse(&j).unwrap_err();
        assert!(
            e.contains("missing required key") && e.contains(&k),
            "{k}: {e}"
        );
    }
}

#[test]
fn rejects_changed_numeric_value() {
    let j = mutate(|v| v["hidden_size"] = serde_json::json!(4096));
    assert!(parse(&j).unwrap_err().contains("hidden_size"));
    let j = mutate(|v| v["norm_eps"] = serde_json::json!(0.001));
    assert!(parse(&j).unwrap_err().contains("norm_eps"));
}

#[test]
fn rejects_layer_array_reordering() {
    let j = mutate(|v| {
        let arr = v["layer_types"].as_array().unwrap().clone();
        let mut swapped = arr.clone();
        swapped.swap(2, 4); // full_attention <-> sliding_attention
        assert_ne!(arr, swapped);
        v["layer_types"] = Value::Array(swapped);
    });
    assert!(parse(&j).unwrap_err().contains("layer_types"));
}

#[test]
fn rejects_changed_nested_rope() {
    let j = mutate(|v| {
        v["rope_parameters"]["sliding_attention"]["rope_theta"] = serde_json::json!(5000);
    });
    assert!(parse(&j).unwrap_err().contains("rope_parameters"));
    let j = mutate(|v| {
        v["rope_parameters"]["full_attention"]["partial_rotary_factor"] = serde_json::json!(1.0);
    });
    assert!(parse(&j).unwrap_err().contains("rope_parameters"));
}

#[test]
fn rejects_identity_mismatch() {
    assert!(
        PinnedConfig::parse("elsewhere/model", SOURCE_REVISION, GOOD)
            .unwrap_err()
            .contains("model id mismatch")
    );
    assert!(PinnedConfig::parse(MODEL_ID, "deadbeef", GOOD)
        .unwrap_err()
        .contains("source revision mismatch"));
}

#[test]
fn rejects_malformed_json_and_non_object() {
    assert!(parse("{\"vocab_size\": ")
        .unwrap_err()
        .contains("invalid JSON"));
    assert!(parse("[]").unwrap_err().contains("must be a JSON object"));
    assert!(parse("\"x\"")
        .unwrap_err()
        .contains("must be a JSON object"));
}
