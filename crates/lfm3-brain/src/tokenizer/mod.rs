//! Bounded loading of pinned tokenizer artifacts (source identity + indexing
//! only — NOT encoding).
//!
//! This module verifies a caller-supplied `tokenizer.json` against the frozen
//! source contract (`docs/workbench/LFM3-TOKENIZER-CONTRACT.md`) BEFORE any
//! JSON parsing, then parses it into bounded tables. It performs NO
//! pre-tokenization, NO merge application, and NO decode: those are separate
//! later packets and their absence here is deliberate, not an omission to
//! paper over. A loader is only a loader: accepting a file proves its identity
//! and its shape, never that it can be used to encode text.
//!
//! Trust model, following the accepted `identity`/`quantized_identity`
//! pattern: what the file *claims* about itself (its own counts, its own
//! digests, its own hashes) is never trusted, and never consulted as a gate.
//! The gate is the caller — who owns the trust anchor (path, expected byte
//! length, expected digest) out-of-band and passes it in. The loader's job is
//! to refuse *before* parsing a file whose bytes, length, or structure fail
//! to match that anchor, and to parse within bounded caps when it does match.

pub mod loader;

pub use loader::{
    load_tokenizer, load_verified_buffer, AddedToken, ContractMode, LoadReport, LoaderError,
    TokenLimits, TokenizerTables, ADDED_COUNT, ADDED_ID_RANGE, FIXTURE_ADDED_COUNT,
    FIXTURE_MERGE_COUNT, FIXTURE_VOCAB_COUNT, MAX_READ_CHUNK, MERGE_COUNT, READ_CAP_SLACK,
    TOKENIZER_BYTES, TOKENIZER_SHA256, VOCAB_COUNT,
};
