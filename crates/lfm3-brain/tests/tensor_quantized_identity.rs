//! Integration tests for typed Q8_0 quantized-artifact identity binding and
//! the private validated-base proof. Covers: independent literal digest anchor
//! (root-retained Rust fixture + coreutils oracle); positive borrowed path;
//! failed base bytes cannot produce a proof; every altered link (payload /
//! base / name / shape / tag / recipe / manifest / model / revision / hash /
//! size); excessive string/rank/dim/element/byte/overflow policy; missing
//! tensor and non-float dtype; bad scale + wrong digest -> HashMismatch, bad
//! scale + matching digest -> Structure failure; 10,000-cycle reuse of one
//! prevalidated base proof.

#[path = "quantized_identity/common.rs"]
mod common;
use common::{
    art_limits, base_and_payload, base_entry, base_entry_named, base_file, lcg_bytes, link_base,
    q80_limits, q8_payload, q_entry, sha256_hex, st, BASE_MANIFEST_ID, BASE_NAME, Q8_NAME, TENSOR,
};

#[path = "quantized_identity/bindings.rs"]
mod bindings;
#[path = "quantized_identity/digest_vectors.rs"]
mod digest_vectors;
#[path = "quantized_identity/ordering.rs"]
mod ordering;
#[path = "quantized_identity/policy.rs"]
mod policy;
