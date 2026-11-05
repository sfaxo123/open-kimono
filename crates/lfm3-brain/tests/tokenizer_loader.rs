//! Acceptance tests for the BR-TOK-01R tokenizer artifact loader.
//!
//! Every fixture is a small synthetic `tokenizer.json` written to a temp dir;
//! the real pinned artifact is never touched here (only the example reads it).
//! The loader's contract — length and SHA-256 verified BEFORE parsing, then a
//! bounded contract-checked parse — is exercised end to end.

use lfm3_brain::tokenizer::{
    load_tokenizer, AddedToken, LoaderError, TokenLimits, ADDED_COUNT, ADDED_ID_RANGE, MERGE_COUNT,
    VOCAB_COUNT,
};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

/// Fixture shape counts: small but self-consistent, mirroring the contract's
/// structure (BPE model, vocab object, ordered merge pairs, flagged added
/// tokens). The added-token ids keep the pinned range so the id-range gate is
/// exercised with the same constants as the real artifact.
const FIXTURE_VOCAB: usize = 5;
const FIXTURE_MERGES: usize = 3;
const FIXTURE_ADDED: usize = 2;

fn fixture_limits() -> TokenLimits {
    // BR-TOK-01D: fixture mode is the EXPLICIT constructor; the fixture
    // counts (5/3/2) are frozen in the library, not caller numbers.
    TokenLimits::for_fixture(1_048_576)
}

/// A valid synthetic tokenizer.json (indent-2, same key spellings as the
/// pinned artifact). `merges` pairs are two-string arrays like the real file.
fn valid_fixture() -> String {
    let mut s = String::from("{\n  \"version\": \"1.0\",\n");
    s.push_str("  \"added_tokens\": [\n");
    s.push_str("    {\n      \"id\": 124893,\n      \"content\": \"<|pad|>\",\n");
    s.push_str("      \"single_word\": false,\n      \"lstrip\": false,\n");
    s.push_str("      \"rstrip\": false,\n      \"normalized\": false,\n");
    s.push_str("      \"special\": true\n    },\n");
    s.push_str("    {\n      \"id\": 125032,\n      \"content\": \"<|eos|>\",\n");
    s.push_str("      \"single_word\": true,\n      \"lstrip\": true,\n");
    s.push_str("      \"rstrip\": false,\n      \"normalized\": true,\n");
    s.push_str("      \"special\": true\n    }\n  ],\n");
    s.push_str("  \"model\": {\n    \"type\": \"BPE\",\n    \"vocab\": {\n");
    for i in 0..FIXTURE_VOCAB {
        let comma = if i + 1 < FIXTURE_VOCAB { "," } else { "" };
        s.push_str(&format!("      \"tok{i}\": {i}{comma}\n"));
    }
    s.push_str("    },\n    \"merges\": [\n");
    for i in 0..FIXTURE_MERGES {
        let comma = if i + 1 < FIXTURE_MERGES { "," } else { "" };
        s.push_str(&format!("      [\"m{i}a\", \"m{i}b\"]{comma}\n"));
    }
    s.push_str("    ]\n  }\n}\n");
    s
}

struct FixtureDir(PathBuf);

impl Drop for FixtureDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn temp_fixture_dir(name: &str) -> FixtureDir {
    let mut root = std::env::temp_dir();
    let unique = format!(
        "dsh-tokenizer-loader-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    root.push(unique);
    fs::create_dir_all(&root).expect("create temp fixture dir");
    FixtureDir(root)
}

/// Write `content` to a fixture file and return (path, byte length, digest).
fn write_fixture(dir: &Path, name: &str, content: &[u8]) -> (PathBuf, u64, String) {
    let path = dir.join(name);
    fs::write(&path, content).expect("write fixture");
    let digest: [u8; 32] = Sha256::digest(content).into();
    let hex = digest
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    (path, content.len() as u64, hex)
}

fn expect_err(err: LoaderError, want: &str) {
    let got = format!("{err:?}");
    assert!(got.contains(want), "expected {want}, got {got}");
}

#[test]
fn wrong_length_rejected_before_parse() {
    let dir = temp_fixture_dir("len");
    // A file whose declared shape is fine but whose length is wrong: the
    // loader must reject on stat alone, without reading or hashing it.
    let (path, _len, digest) = write_fixture(&dir.0, "tok.json", valid_fixture().as_bytes());
    let result = load_tokenizer(&path, 999_999, &digest, &fixture_limits());
    match result {
        Err(LoaderError::SizeMismatch { expected, actual }) => {
            assert_eq!(expected, 999_999);
            assert_eq!(actual, fs::metadata(&path).unwrap().len());
        }
        other => panic!("expected SizeMismatch, got {other:?}"),
    }
}

#[test]
fn truncated_file_rejected_before_parse() {
    let dir = temp_fixture_dir("trunc");
    // Truncation changes the length: phase 1 rejects before any hashing or
    // parsing happens (the byte length of a truncated file can never equal
    // the pinned/expected length).
    let full = valid_fixture();
    let cut = &full[..full.len() - 40];
    let (path, len, _digest_of_full) = write_fixture(&dir.0, "tok.json", cut.as_bytes());
    let expected_digest = {
        let d: [u8; 32] = Sha256::digest(cut.as_bytes()).into();
        d.iter().map(|b| format!("{b:02x}")).collect::<String>()
    };
    // Even with the truncated file's own correct digest supplied, the expected
    // (original) length no longer matches: rejected before parse.
    let result = load_tokenizer(
        &path,
        full.len() as u64,
        &expected_digest,
        &fixture_limits(),
    );
    expect_err(result.unwrap_err(), "SizeMismatch");
    assert_eq!(len, cut.len() as u64);
}

#[test]
fn truncated_json_same_length_rejected_as_malformed() {
    let dir = temp_fixture_dir("trunc2");
    // A file that matches length + digest but is structurally truncated JSON:
    // the parse phase rejects it as MalformedJson, never returning tables.
    let full = valid_fixture();
    let mut cut: String = full[..full.len() - 40].into();
    // Pad with spaces so the byte length equals the original file's length.
    while cut.len() < full.len() {
        cut.push(' ');
    }
    let (path, len, digest) = write_fixture(&dir.0, "tok.json", cut.as_bytes());
    assert_eq!(len, full.len() as u64);
    let result = load_tokenizer(&path, len, &digest, &fixture_limits());
    expect_err(result.unwrap_err(), "MalformedJson");
}

#[test]
fn wrong_sha_rejected_before_parse() {
    let dir = temp_fixture_dir("sha");
    let (path, len, _real) = write_fixture(&dir.0, "tok.json", valid_fixture().as_bytes());
    // A well-formed but WRONG digest: the streamed hash rejects before parse.
    let wrong = "0".repeat(64);
    let result = load_tokenizer(&path, len, &wrong, &fixture_limits());
    expect_err(result.unwrap_err(), "HashMismatch");

    // A non-hex digest of the right length is rejected too (still before read).
    let result = load_tokenizer(&path, len, &"z".repeat(64), &fixture_limits());
    expect_err(result.unwrap_err(), "HashMismatch");
}

#[test]
fn malformed_merge_rejected() {
    let dir = temp_fixture_dir("merge");
    // One merge element is a 3-element array: MalformedMerge.
    let bad = valid_fixture().replace("[\"m1a\", \"m1b\"]", "[\"m1a\", \"m1b\", \"m1c\"]");
    let (path, len, digest) = write_fixture(&dir.0, "tok.json", bad.as_bytes());
    let result = load_tokenizer(&path, len, &digest, &fixture_limits());
    expect_err(result.unwrap_err(), "MalformedMerge");

    // One merge element is not an array at all: also MalformedMerge.
    let bad = valid_fixture().replace("[\"m2a\", \"m2b\"]", "\"m2a m2b\"");
    let (path, len, digest) = write_fixture(&dir.0, "tok.json", bad.as_bytes());
    let result = load_tokenizer(&path, len, &digest, &fixture_limits());
    expect_err(result.unwrap_err(), "MalformedMerge");
}

#[test]
fn count_mismatch_rejected() {
    let dir = temp_fixture_dir("count");
    // One vocab entry removed against a fixture policy that requires 5:
    // the contract-count gate rejects it.
    let bad = valid_fixture().replace("      \"tok4\": 4\n", "");
    let (path, len, digest) = write_fixture(&dir.0, "tok.json", bad.as_bytes());
    let result = load_tokenizer(&path, len, &digest, &fixture_limits());
    expect_err(result.unwrap_err(), "MalformedJson");

    // One extra merge against a policy that requires 3: LimitExceeded.
    let bad = valid_fixture().replace(
        "      [\"m2a\", \"m2b\"]\n",
        "      [\"m2a\", \"m2b\"],\n      [\"m3a\", \"m3b\"]\n",
    );
    let mut limits = fixture_limits();
    limits.max_merge_pairs = FIXTURE_MERGES; // still 3: file now has 4
    let (path, len, digest) = write_fixture(&dir.0, "tok.json", bad.as_bytes());
    let result = load_tokenizer(&path, len, &digest, &limits);
    expect_err(result.unwrap_err(), "LimitExceeded");
}

#[test]
fn out_of_range_added_token_id_rejected() {
    let dir = temp_fixture_dir("addedid");
    // An added-token id below the pinned range start.
    let bad = valid_fixture().replace("\"id\": 124893", "\"id\": 124892");
    let (path, len, digest) = write_fixture(&dir.0, "tok.json", bad.as_bytes());
    let result = load_tokenizer(&path, len, &digest, &fixture_limits());
    expect_err(result.unwrap_err(), "AddedIdOutOfRange");

    // An added-token id above the pinned range end.
    let bad = valid_fixture().replace("\"id\": 125032", "\"id\": 125033");
    let (path, len, digest) = write_fixture(&dir.0, "tok.json", bad.as_bytes());
    let result = load_tokenizer(&path, len, &digest, &fixture_limits());
    expect_err(result.unwrap_err(), "AddedIdOutOfRange");

    // Sanity: the fixture's own ids are inside the range this gate enforces.
    assert!(ADDED_ID_RANGE.contains(&124_893) && ADDED_ID_RANGE.contains(&125_032));
}

#[test]
fn duplicate_token_rejected() {
    let dir = temp_fixture_dir("dup");
    // A repeated merge *pair* is rejected as a duplicate (the pinned artifact
    // carries 293,320 distinct pairs; pair-level duplicates are malformed).
    // Self-merge halves such as `["m0a", "m0a"]` are legal — the real file
    // contains 503 of them — so they must NOT reject.
    let ok = valid_fixture().replace("[\"m1a\", \"m1b\"]", "[\"m0a\", \"m0a\"]");
    let (path, len, digest) = write_fixture(&dir.0, "selfmerge.json", ok.as_bytes());
    load_tokenizer(&path, len, &digest, &fixture_limits())
        .expect("self-merge halves are legal in the pinned artifact");

    let bad = valid_fixture().replace("[\"m2a\", \"m2b\"]", "[\"m0a\", \"m0b\"]");
    let (path, len, digest) = write_fixture(&dir.0, "tok.json", bad.as_bytes());
    let result = load_tokenizer(&path, len, &digest, &fixture_limits());
    expect_err(result.unwrap_err(), "DuplicateToken");

    // A repeated added-token content is rejected as well.
    let bad = valid_fixture().replace("\"content\": \"<|eos|>\"", "\"content\": \"<|pad|>\"");
    let (path, len, digest) = write_fixture(&dir.0, "tok.json", bad.as_bytes());
    let result = load_tokenizer(&path, len, &digest, &fixture_limits());
    expect_err(result.unwrap_err(), "DuplicateAddedToken");
}

#[test]
fn duplicate_raw_vocab_key_rejected_before_map_collapse() {
    let dir = temp_fixture_dir("dupkey");
    // BR-TOK-01C defect 1: a `model.vocab` object carrying the SAME raw key
    // twice. serde_json::Value silently collapses duplicate keys into one map
    // entry (last wins), which drops a vocabulary entry and lets the count
    // gate pass a file that is not a valid 1:1 token->id mapping. The loader
    // must detect the duplicate at the raw event level, BEFORE any map
    // collapse, and reject.
    //
    // The fixture stays fully self-consistent otherwise: 5 raw vocab keys
    // (one of them repeated), ids 0..4 with no id collision, 3 merges, 2
    // added tokens — so the ONLY defect is the repeated raw key.
    let mut s = String::from("{\n  \"version\": \"1.0\",\n");
    s.push_str("  \"added_tokens\": [\n");
    s.push_str("    {\n      \"id\": 124893,\n      \"content\": \"<|pad|>\",\n");
    s.push_str("      \"single_word\": false,\n      \"lstrip\": false,\n");
    s.push_str("      \"rstrip\": false,\n      \"normalized\": false,\n");
    s.push_str("      \"special\": true\n    },\n");
    s.push_str("    {\n      \"id\": 125032,\n      \"content\": \"<|eos|>\",\n");
    s.push_str("      \"single_word\": true,\n      \"lstrip\": true,\n");
    s.push_str("      \"rstrip\": false,\n      \"normalized\": true,\n");
    s.push_str("      \"special\": true\n    }\n  ],\n");
    s.push_str("  \"model\": {\n    \"type\": \"BPE\", \"vocab\": {");
    s.push_str("\"a\": 0, \"b\": 1, \"c\": 2, \"d\": 3, \"a\": 4");
    s.push_str(
        " }, \"merges\": [[\"m0a\", \"m0b\"], [\"m1a\", \"m1b\"], [\"m2a\", \"m2b\"]]}\n}\n",
    );
    let (path, len, digest) = write_fixture(&dir.0, "tok.json", s.as_bytes());
    let result = load_tokenizer(&path, len, &digest, &fixture_limits());
    expect_err(result.unwrap_err(), "DuplicateVocabKey");
}

#[test]
fn duplicate_vocab_id_rejected() {
    let dir = temp_fixture_dir("dupid");
    // BR-TOK-01C defect 2: two DISTINCT token strings mapped to the SAME
    // vocabulary id. Keys are unique and the entry count matches the
    // contract, so the current map-based parse accepts it — but decode would
    // become ambiguous. Every vocab id must appear at most once.
    //
    // Root-probe shape (TOKENIZER-ROOT-PROBE.rs) scaled to the fixture
    // counts: 5 unique keys, ids 0,1,2,3,2 — "tok4" collides with "tok2".
    let mut s = String::from("{\n  \"version\": \"1.0\",\n");
    s.push_str("  \"added_tokens\": [\n");
    s.push_str("    {\n      \"id\": 124893,\n      \"content\": \"<|pad|>\",\n");
    s.push_str("      \"single_word\": false,\n      \"lstrip\": false,\n");
    s.push_str("      \"rstrip\": false,\n      \"normalized\": false,\n");
    s.push_str("      \"special\": true\n    },\n");
    s.push_str("    {\n      \"id\": 125032,\n      \"content\": \"<|eos|>\",\n");
    s.push_str("      \"single_word\": true,\n      \"lstrip\": true,\n");
    s.push_str("      \"rstrip\": false,\n      \"normalized\": true,\n");
    s.push_str("      \"special\": true\n    }\n  ],\n");
    s.push_str("  \"model\": {\n    \"type\": \"BPE\", \"vocab\": {");
    s.push_str("\"tok0\": 0, \"tok1\": 1, \"tok2\": 2, \"tok3\": 3, \"tok4\": 2");
    s.push_str(
        " }, \"merges\": [[\"m0a\", \"m0b\"], [\"m1a\", \"m1b\"], [\"m2a\", \"m2b\"]]}\n}\n",
    );
    let (path, len, digest) = write_fixture(&dir.0, "tok.json", s.as_bytes());
    let result = load_tokenizer(&path, len, &digest, &fixture_limits());
    expect_err(result.unwrap_err(), "DuplicateVocabId");
}

#[test]
fn duplicate_added_token_id_rejected() {
    let dir = temp_fixture_dir("addeddup");
    // BR-TOK-01C STEP 3: two added-token entries claiming the SAME id (both
    // inside the pinned 124893..=125032 range) with distinct contents. The
    // id must be unique across added tokens as well.
    let bad = valid_fixture().replace("\"id\": 125032", "\"id\": 124893");
    let (path, len, digest) = write_fixture(&dir.0, "tok.json", bad.as_bytes());
    let result = load_tokenizer(&path, len, &digest, &fixture_limits());
    expect_err(result.unwrap_err(), "DuplicateAddedId");
}

#[test]
fn added_token_id_colliding_with_vocab_id_rejected() {
    let dir = temp_fixture_dir("addedvsvocab");
    // BR-TOK-01C STEP 3: an added-token id inside the vocabulary id space
    // (here 3, a vocab id used by "tok3"). The pinned artifact keeps the two
    // id spaces disjoint — added ids start at 124893, one past the last vocab
    // id 124892 — and the loader enforces the pinned range; this fixture
    // proves the cross-space collision rejects rather than shadowing.
    let bad = valid_fixture().replace("\"id\": 124893", "\"id\": 3");
    let (path, len, digest) = write_fixture(&dir.0, "tok.json", bad.as_bytes());
    let result = load_tokenizer(&path, len, &digest, &fixture_limits());
    expect_err(result.unwrap_err(), "AddedIdOutOfRange");
}

#[test]
fn valid_synthetic_fixture_round_trips() {
    let dir = temp_fixture_dir("roundtrip");
    let full = valid_fixture();
    let (path, len, digest) = write_fixture(&dir.0, "tok.json", full.as_bytes());
    let (report, tables) =
        load_tokenizer(&path, len, &digest, &fixture_limits()).expect("valid fixture must load");

    assert!(report.length_verified && report.digest_verified);
    assert!(!report.pinned_artifact); // synthetic fixture, not the pinned file
    assert_eq!(report.byte_length, len);
    assert_eq!(report.sha256, digest);

    // Counts round-trip.
    assert_eq!(report.vocab_count, FIXTURE_VOCAB);
    assert_eq!(report.merge_count, FIXTURE_MERGES);
    assert_eq!(report.added_count, FIXTURE_ADDED);
    assert_eq!(tables.vocab.len(), FIXTURE_VOCAB);
    assert_eq!(tables.merges.len(), FIXTURE_MERGES);
    assert_eq!(tables.added_tokens.len(), FIXTURE_ADDED);

    // Sampled vocab entry.
    assert_eq!(tables.vocab.get("tok0"), Some(&0));
    assert_eq!(tables.vocab.get("tok4"), Some(&4));

    // Sampled merge (order preserved).
    assert_eq!(tables.merges[1], ("m1a".to_owned(), "m1b".to_owned()));

    // Added-token flags preserved exactly, per token.
    assert_eq!(
        tables.added_tokens[0],
        AddedToken {
            content: "<|pad|>".to_owned(),
            id: 124_893,
            single_word: false,
            lstrip: false,
            rstrip: false,
            normalized: false,
            special: true,
        }
    );
    assert_eq!(
        tables.added_tokens[1],
        AddedToken {
            content: "<|eos|>".to_owned(),
            id: 125_032,
            single_word: true,
            lstrip: true,
            rstrip: false,
            normalized: true,
            special: true,
        }
    );
}

#[test]
fn missing_file_is_io_error_not_panic() {
    let dir = temp_fixture_dir("missing");
    let path = dir.0.join("nope.json");
    let result = load_tokenizer(&path, 1, &"0".repeat(64), &fixture_limits());
    expect_err(result.unwrap_err(), "Io");
}

// ---- BR-TOK-01D: contract-count independence (RED against the pre-fix loader)
//
// Root review 2026-10-28, finding 2: the caller-supplied `max_*` fields double
// as the EXACT contract counts, so a caller can redefine the contract by
// raising a limit. These tests pin the NEW invariant: which artifact counts are
// ACCEPTED is fixed by library constants (124,893 / 293,320 / 140 and, in
// explicit fixture mode, the pinned fixture counts) — caller `max_*` values may
// only CAP resources, never move the acceptance target. Fixture-shaped counts
// are legal ONLY through the explicit fixture constructor; the production
// constructor must reject a 5-vocab fixture no matter what limits say.

/// A synthetic fixture whose counts match the FIXTURE contract (5/3/2).
fn fixture_shaped() -> String {
    valid_fixture()
}

/// A synthetic fixture whose counts match the PRODUCTION contract (124,893
/// vocab ids `0..=124_892`, 293,320 distinct merge pairs, 140 added tokens with
/// distinct ids in the pinned `124_893..=125_032`).
fn production_shaped() -> String {
    let mut s = String::from("{\n  \"version\": \"1.0\",\n  \"added_tokens\": [\n");
    for i in 0..ADDED_COUNT {
        let comma = if i + 1 < ADDED_COUNT { "," } else { "" };
        s.push_str(&format!(
            "    {{\n      \"id\": {},\n      \"content\": \"<|added{i}|>\",\n",
            124_893 + i
        ));
        s.push_str("      \"single_word\": false,\n      \"lstrip\": false,\n");
        s.push_str("      \"rstrip\": false,\n      \"normalized\": false,\n");
        s.push_str(&format!("      \"special\": true\n    }}{comma}\n"));
    }
    s.push_str("  ],\n  \"model\": {\n    \"type\": \"BPE\",\n    \"vocab\": {\n");
    for i in 0..VOCAB_COUNT {
        let comma = if i + 1 < VOCAB_COUNT { "," } else { "" };
        s.push_str(&format!("      \"tok{i}\": {i}{comma}\n"));
    }
    s.push_str("    },\n    \"merges\": [\n");
    for i in 0..MERGE_COUNT {
        let comma = if i + 1 < MERGE_COUNT { "," } else { "" };
        s.push_str(&format!("      [\"m{i}a\", \"m{i}b\"]{comma}\n"));
    }
    s.push_str("    ]\n  }\n}\n");
    s
}

/// A fixture-shaped file (5/3/2) but with ONE extra vocab entry: 6/3/2.
fn wrong_count_fixture() -> String {
    valid_fixture().replace(
        "      \"tok4\": 4\n",
        "      \"tok4\": 4,\n      \"tok5\": 5\n",
    )
}

#[test]
fn contract_counts_are_independent_of_caller_limits() {
    let dir = temp_fixture_dir("contractfreeze");

    // (1) A 5-vocab fixture loads ONLY through the explicit fixture
    //     constructor. The production constructor's contract requires
    //     124,893 vocab entries — a 5-entry file must reject no matter what
    //     the caller's limits say.
    let (path, len, digest) = write_fixture(&dir.0, "fix.json", fixture_shaped().as_bytes());
    let limits = TokenLimits::for_fixture(len);
    load_tokenizer(&path, len, &digest, &limits).expect("fixture mode must accept fixture counts");
    let result = load_tokenizer(&path, len, &digest, &TokenLimits::production(len));
    expect_err(result.unwrap_err(), "MalformedJson");

    // (2) Raising a caller limit must NOT redefine the contract. Under the
    //     pre-fix loader the exact-count gate reads `max_vocab_entries`, so
    //     `max_vocab_entries = 6` silently accepts the 6-vocab file. Under the
    //     frozen contract the acceptance target stays at the pinned fixture
    //     count and this must reject.
    let (path, len, digest) = write_fixture(&dir.0, "wrong.json", wrong_count_fixture().as_bytes());
    let mut raised = TokenLimits::for_fixture(len);
    raised.max_vocab_entries = FIXTURE_VOCAB + 1;
    let result = load_tokenizer(&path, len, &digest, &raised);
    expect_err(result.unwrap_err(), "MalformedJson");

    // (3) Same independence on the production side: raising `max_vocab_entries`
    //     above the contract count must not make a 6-vocab file acceptable to
    //     the production constructor.
    let mut raised = TokenLimits::production(len);
    raised.max_vocab_entries = VOCAB_COUNT + 1;
    let result = load_tokenizer(&path, len, &digest, &raised);
    expect_err(result.unwrap_err(), "MalformedJson");
}

#[test]
fn caller_limits_cap_resources_without_changing_acceptance() {
    let dir = temp_fixture_dir("caponly");
    let full = production_shaped();
    let (path, len, digest) = write_fixture(&dir.0, "prod.json", full.as_bytes());

    // The production constructor accepts the production-count file with the
    // default caps.
    load_tokenizer(&path, len, &digest, &TokenLimits::production(len))
        .expect("production-count file must load under production contract");

    // A caller may CAP resources below the contract: the same file must then
    // reject `LimitExceeded` — the cap is enforced, but it changed nothing
    // about which counts the contract accepts.
    let mut capped = TokenLimits::production(len);
    capped.max_vocab_entries = VOCAB_COUNT - 1;
    let result = load_tokenizer(&path, len, &digest, &capped);
    expect_err(result.unwrap_err(), "LimitExceeded");

    let mut capped = TokenLimits::production(len);
    capped.max_total_bytes = len - 1;
    let result = load_tokenizer(&path, len, &digest, &capped);
    expect_err(result.unwrap_err(), "LimitExceeded");

    // The production constructor must reject limits below the contract counts
    // up front: caps may only tighten resources, never redefine the contract.
    let mut below = TokenLimits::production(len);
    below.max_added_tokens = ADDED_COUNT - 1;
    let result = load_tokenizer(&path, len, &digest, &below);
    expect_err(result.unwrap_err(), "LimitExceeded");
}

// ---- BR-TOK-01D: digest/parse binding (structural test of the new invariant)
//
// Root review 2026-10-28, finding 1: the pre-fix loader stat'ed, hashed, and
// parsed in SEPARATE passes that each REOPEN the path — the bytes handed to
// the parser are not the bytes that were hashed. The fix reads the file ONCE
// into a bounded buffer (cap = expected_bytes + 1, so oversize is detected),
// hashes THOSE EXACT bytes, and parses THOSE SAME bytes. A racily replaced or
// in-place-changed file cannot be simulated deterministically, so the NEW
// invariant is pinned structurally: the load core binds one in-memory buffer
// to its digest, and any buffer whose content disagrees with its claimed
// digest — or whose length disagrees with the claim — rejects before parsing.

#[test]
fn verified_buffer_bound_rejects_content_that_differs_from_its_digest() {
    // Fixture-shaped bytes (5/3/2): the buffer binding itself is
    // contract-agnostic — what matters is that the parser consumes EXACTLY
    // the hashed buffer, never a re-read of the path.
    let bytes = fixture_shaped().into_bytes();
    let claimed: [u8; 32] = Sha256::digest(&bytes).into();
    let claimed_hex = claimed
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();

    // The exact bytes with the exact digest: the binding accepts and reports
    // them.
    let (report, tables) = lfm3_brain::tokenizer::load_verified_buffer(
        &bytes,
        bytes.len() as u64,
        &claimed_hex,
        &TokenLimits::for_fixture(bytes.len() as u64),
    )
    .expect("exact bytes + exact digest must bind");
    assert!(report.length_verified && report.digest_verified);
    assert_eq!(report.byte_length, bytes.len() as u64);
    assert_eq!(report.sha256, claimed_hex);
    // The parse consumed EXACTLY the verified buffer: the parsed length
    // equals the verified length, and the counts match what THESE bytes hold.
    assert_eq!(report.vocab_count, FIXTURE_VOCAB);
    assert_eq!(tables.vocab.len(), FIXTURE_VOCAB);

    // One byte flipped AFTER hashing: the claimed digest no longer describes
    // these bytes. The binding must reject — the parser must never see a
    // buffer whose content differs from its claimed digest.
    let mut tampered = bytes.clone();
    tampered[bytes.len() / 2] ^= 0x01;
    let result = lfm3_brain::tokenizer::load_verified_buffer(
        &tampered,
        tampered.len() as u64,
        &claimed_hex,
        &TokenLimits::for_fixture(tampered.len() as u64),
    );
    expect_err(result.unwrap_err(), "HashMismatch");

    // A claimed length that differs from the buffer's actual length rejects
    // before hashing (length gate first).
    let result = lfm3_brain::tokenizer::load_verified_buffer(
        &bytes,
        (bytes.len() as u64) + 1,
        &claimed_hex,
        &TokenLimits::for_fixture(bytes.len() as u64 + 1),
    );
    expect_err(result.unwrap_err(), "SizeMismatch");
}

#[test]
fn loader_exposes_single_read_buffer_contract() {
    // The loader's own constants must state the single-read binding: one
    // bounded read at `expected_bytes + 1` (oversize detectable), and the
    // streaming-rehash path must be gone. `MAX_READ_CHUNK` remains only as the
    // chunk size of the ONE read.
    assert_eq!(
        lfm3_brain::tokenizer::READ_CAP_SLACK,
        1,
        "the single read must cap at expected_bytes + 1 to detect oversize"
    );
}
