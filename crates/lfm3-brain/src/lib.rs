//! Native Rust LFM3 foundations. No pretrained inference backend is ready yet.
pub const MODEL_ID: &str = "LiquidAI/LFM3-1.7B";
pub const SOURCE_REVISION: &str = "40ebe0aa6662c9d40389c2f5d565b6a2c6aebacd";

pub mod artifacts;
pub mod identity;
#[cfg(feature = "kernels")]
pub mod kernels;
#[cfg(feature = "model")]
pub mod model;
pub mod quantized;
pub mod quantized_identity;
#[cfg(feature = "service")]
pub mod service;
pub mod tokenizer;
