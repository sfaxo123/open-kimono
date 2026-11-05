//! Public API check against the independently retained official source config.
#![cfg(feature = "model")]

use lfm3_brain::{model::PinnedConfig, MODEL_ID, SOURCE_REVISION};

#[test]
fn retained_official_config_matches_embedded_validation_contract() {
    // Source: LiquidAI/LFM3-1.7B, revision 40ebe0aa6662c9d40389c2f5d565b6a2c6aebacd.
    let official = include_str!("fixtures/lfm3-config.json");
    let validated = PinnedConfig::parse(MODEL_ID, SOURCE_REVISION, official).unwrap();
    let expected: serde_json::Value = serde_json::from_str(official).unwrap();
    assert_eq!(validated.value(), &expected);
}
