//! Integration tests for the bounded safetensors structural reader.
//! Tests build real safetensors byte buffers and assert structural acceptance
//! or precise rejection — they do not echo implementation internals.

use lfm3_brain::artifacts::{parse_artifact, ArtifactError, ArtifactLimits};

/// Build a complete safetensors file: 8-byte LE header length + header JSON + payload.
fn st(header_json: &str, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&(header_json.len() as u64).to_le_bytes());
    out.extend_from_slice(header_json.as_bytes());
    out.extend_from_slice(payload);
    out
}

fn limits() -> ArtifactLimits {
    ArtifactLimits {
        max_header_bytes: 4096,
        max_tensors: 16,
        max_rank: 4,
        max_dimension: u64::MAX,
        max_total_bytes: 1 << 20,
    }
}

/// Assert the parse failed with exactly this error (Ok and other Errs both fail).
macro_rules! assert_err {
    ($expr:expr, $pat:pat $(if $guard:expr)?) => {
        match $expr {
            Err($pat) $(if $guard)? => {}
            Err(other) => panic!("expected {}, got Err({other:?})", stringify!($pat)),
            Ok(_) => panic!("expected {}, got Ok", stringify!($pat)),
        }
    };
}

// ---------- valid cases ----------

#[test]
fn reserved_key_at_root_is_a_tensor_name_not_metadata() {
    // A bare "metadata" key is an ordinary tensor name upstream; only
    // "__metadata__" is reserved. So a descriptor under "metadata" parses.
    let header = r#"{"metadata":{"dtype":"U8","shape":[1],"data_offsets":[0,1]}}"#;
    let file = st(header, &[5]);
    let art = parse_artifact(&file, &limits()).expect("metadata is a normal name");
    assert_eq!(art.metadata, None);
    assert_eq!(art.tensor("metadata").unwrap().bytes, &[5]);
}

#[test]
fn accepts_empty_header_object_with_empty_payload() {
    let file = st("{}", &[]);
    let art = parse_artifact(&file, &limits()).expect("empty file structurally valid");
    assert!(art.tensors.is_empty());
    assert!(art.metadata.is_none());
}

// ---------- malformed / rejection cases ----------

#[test]
fn rejects_truncated_prefix_and_truncated_header() {
    assert_err!(
        parse_artifact(&[0, 0, 0], &limits()),
        ArtifactError::TruncatedPrefix
    );
    // Declares a 100-byte header but only provides 10.
    let mut file = (100u64).to_le_bytes().to_vec();
    file.extend_from_slice(b"{\"a\":1}");
    assert_err!(
        parse_artifact(&file, &limits()),
        ArtifactError::TruncatedHeader
    );
}

#[test]
fn rejects_header_length_policy_and_overflow() {
    // Header length above max_header_bytes.
    let mut file = (limits().max_header_bytes + 1).to_le_bytes().to_vec();
    file.extend_from_slice(b"{}");
    assert_err!(
        parse_artifact(&file, &limits()),
        ArtifactError::HeaderTooLarge
    );
    // u64::MAX length: 8 + len must not overflow or index anything.
    let mut file = u64::MAX.to_le_bytes().to_vec();
    file.extend_from_slice(b"{}");
    assert_err!(
        parse_artifact(&file, &limits()),
        ArtifactError::HeaderTooLarge
    );
    // Length within header cap but total input beyond max_total_bytes.
    let tiny = ArtifactLimits {
        max_total_bytes: 12,
        ..limits()
    };
    let header = "{\"a\":{\"dtype\":\"U8\",\"shape\":[1],\"data_offsets\":[0,1]}}";
    let mut file = (header.len() as u64).to_le_bytes().to_vec();
    file.extend_from_slice(header.as_bytes());
    file.extend_from_slice(&[7]);
    assert_err!(parse_artifact(&file, &tiny), ArtifactError::InputTooLarge);
}

#[test]
fn rejects_non_object_and_non_utf8_header() {
    assert_err!(
        parse_artifact(&st("[1,2]", &[]), &limits()),
        ArtifactError::HeaderNotObject
    );
    // Header declares exactly the invalid-UTF-8 bytes it provides.
    let bad: [u8; 4] = [b'{', 0xFF, 0xFE, b'}'];
    let mut file = (bad.len() as u64).to_le_bytes().to_vec();
    file.extend_from_slice(&bad);
    assert_err!(
        parse_artifact(&file, &limits()),
        ArtifactError::HeaderNotUtf8
    );
}

#[test]
fn rejects_malformed_or_truncated_header_json() {
    assert_err!(
        parse_artifact(&st("{\"a\":", &[]), &limits()),
        ArtifactError::HeaderJson(_)
    );
    assert_err!(
        parse_artifact(&st("{\"a\": {\"dtype\": \"U8\"", &[]), &limits()),
        ArtifactError::HeaderJson(_)
    );
}

#[test]
fn rejects_duplicate_root_tensor_names() {
    // serde_json would silently keep the last; the reader must catch it.
    let header = r#"{"a":{"dtype":"U8","shape":[1],"data_offsets":[0,1]},"a":{"dtype":"U8","shape":[1],"data_offsets":[0,1]}}"#;
    assert_err!(parse_artifact(&st(header, &[0]), &limits()), ArtifactError::DuplicateTensorName(s) if s == "a");
}

#[test]
fn rejects_duplicate_descriptor_fields_and_unknown_fields() {
    let dup = r#"{"a":{"dtype":"U8","dtype":"U8","shape":[1],"data_offsets":[0,1]}}"#;
    assert_err!(parse_artifact(&st(dup, &[0]), &limits()), ArtifactError::DuplicateDescriptorField(s) if s == "dtype");
    let unknown = r#"{"a":{"dtype":"U8","shape":[1],"data_offsets":[0,1],"compression":"none"}}"#;
    assert_err!(parse_artifact(&st(unknown, &[0]), &limits()), ArtifactError::UnknownDescriptorField(s) if s == "compression");
    let missing = r#"{"a":{"dtype":"U8","shape":[1]}}"#;
    assert_err!(parse_artifact(&st(missing, &[]), &limits()), ArtifactError::MissingDescriptorField(s) if s == "data_offsets");
}

#[test]
fn rejects_duplicate_metadata_keys() {
    let header =
        r#"{"a":{"dtype":"U8","shape":[1],"data_offsets":[0,1]},"__metadata__":{"k":"1","k":"2"}}"#;
    assert_err!(parse_artifact(&st(header, &[0]), &limits()), ArtifactError::DuplicateMetadataKey(s) if s == "k");
    let nonstring =
        r#"{"a":{"dtype":"U8","shape":[1],"data_offsets":[0,1]},"__metadata__":{"k":42}}"#;
    assert_err!(
        parse_artifact(&st(nonstring, &[0]), &limits()),
        ArtifactError::MetadataNotStringMap
    );
}

#[test]
fn rejects_unsupported_and_quantized_dtypes() {
    for d in ["F8_E4M3", "F8_E5M2", "I4", "U4", "F4", "BF8", "string", ""] {
        let header =
            format!("{{\"a\":{{\"dtype\":\"{d}\",\"shape\":[2],\"data_offsets\":[0,2]}}}}");
        let file = st(&header, &[0, 0]);
        let got = parse_artifact(&file, &limits());
        match got {
            Err(ArtifactError::UnsupportedDtype(s)) => assert_eq!(s, d),
            other => panic!("dtype {d:?} must be explicitly rejected, got {other:?}"),
        }
    }
}

#[test]
fn rejects_negative_reversed_and_out_of_range_offsets() {
    let neg = r#"{"a":{"dtype":"U8","shape":[1],"data_offsets":[-1,1]}}"#;
    assert_err!(parse_artifact(&st(neg, &[0]), &limits()), ArtifactError::OffsetNotUnsigned(s) if s == "-1");
    let rev = r#"{"a":{"dtype":"U8","shape":[1],"data_offsets":[1,0]}}"#;
    assert_err!(
        parse_artifact(&st(rev, &[0, 0]), &limits()),
        ArtifactError::ReversedOffsets
    );
    let oor = r#"{"a":{"dtype":"U8","shape":[2],"data_offsets":[0,9]}}"#;
    assert_err!(
        parse_artifact(&st(oor, &[0, 0]), &limits()),
        ArtifactError::OffsetOutOfRange
    );
}

#[test]
fn rejects_shape_product_and_byte_mismatches() {
    // product overflows u64 (2^32 * 2^32), payload far smaller
    let huge = r#"{"a":{"dtype":"U8","shape":[4294967296,4294967296],"data_offsets":[0,1]}}"#;
    assert_err!(
        parse_artifact(&st(huge, &[0]), &limits()),
        ArtifactError::ShapeProductOverflow
    );
    // element*width != range length: F32 shape[3] needs 12 bytes, range is 8
    let mismatch = r#"{"a":{"dtype":"F32","shape":[3],"data_offsets":[0,8]}}"#;
    assert_err!(
        parse_artifact(&st(mismatch, &[0; 8]), &limits()),
        ArtifactError::ShapeByteMismatch
    );
    // negative shape element
    let negdim = r#"{"a":{"dtype":"U8","shape":[-2],"data_offsets":[0,0]}}"#;
    assert_err!(parse_artifact(&st(negdim, &[]), &limits()), ArtifactError::ShapeElementNotDimension(s) if s == "-2");
    // non-integer shape element
    let frac = r#"{"a":{"dtype":"U8","shape":[1.5],"data_offsets":[0,0]}}"#;
    match parse_artifact(&st(frac, &[]), &limits()) {
        Err(ArtifactError::ShapeElementNotDimension(_)) => {}
        other => panic!("fractional dim rejected precisely, got {other:?}"),
    }
}

#[test]
fn rejects_overlap_hole_and_unindexed_trailing_bytes() {
    // overlap: [0,4) and [2,6)
    let overlap = r#"{"a":{"dtype":"U8","shape":[4],"data_offsets":[0,4]},"b":{"dtype":"U8","shape":[4],"data_offsets":[2,6]}}"#;
    assert_err!(
        parse_artifact(&st(overlap, &[0; 6]), &limits()),
        ArtifactError::OverlappingRanges
    );
    // hole: [0,2) then [4,6)
    let hole = r#"{"a":{"dtype":"U8","shape":[2],"data_offsets":[0,2]},"b":{"dtype":"U8","shape":[2],"data_offsets":[4,6]}}"#;
    assert_err!(
        parse_artifact(&st(hole, &[0; 6]), &limits()),
        ArtifactError::RangeHole
    );
    // first tensor not starting at 0 is also a hole
    let late = r#"{"a":{"dtype":"U8","shape":[2],"data_offsets":[2,4]}}"#;
    assert_err!(
        parse_artifact(&st(late, &[0; 4]), &limits()),
        ArtifactError::RangeHole
    );
    // trailing bytes not referenced by any tensor
    let trailing = r#"{"a":{"dtype":"U8","shape":[2],"data_offsets":[0,2]}}"#;
    assert_err!(
        parse_artifact(&st(trailing, &[0; 5]), &limits()),
        ArtifactError::UnindexedTrailingBytes
    );
}

#[test]
fn rejects_when_limits_exceeded() {
    let few = ArtifactLimits {
        max_tensors: 1,
        ..limits()
    };
    let header = r#"{"a":{"dtype":"U8","shape":[1],"data_offsets":[0,1]},"b":{"dtype":"U8","shape":[1],"data_offsets":[1,2]}}"#;
    assert_err!(
        parse_artifact(&st(header, &[0, 0]), &few),
        ArtifactError::LimitExceeded
    );
    let shallow = ArtifactLimits {
        max_rank: 2,
        ..limits()
    };
    let deep = r#"{"a":{"dtype":"U8","shape":[1,1,1],"data_offsets":[0,1]}}"#;
    assert_err!(
        parse_artifact(&st(deep, &[0]), &shallow),
        ArtifactError::RankTooLarge
    );
}

#[test]
fn accepts_trailing_spaces_after_header_json() {
    // The header may end with whitespace after the closing brace; the JSON
    // itself is still valid UTF-8 JSON.
    let header = "{\"a\":{\"dtype\":\"U8\",\"shape\":[1],\"data_offsets\":[0,1]}}  ";
    let file = st(header, &[9]);
    let art = parse_artifact(&file, &limits()).expect("trailing header spaces accepted");
    assert_eq!(art.tensor("a").unwrap().bytes, &[9]);
}

#[test]
fn rejects_escaped_duplicate_decoded_names() {
    // "a" and "a\u0062" decode to the same key "ab"; serde_json would keep the
    // last, but duplicate decoded names must be rejected before insertion.
    let header = r#"{"ab":{"dtype":"U8","shape":[1],"data_offsets":[0,1]},"a\u0062":{"dtype":"U8","shape":[1],"data_offsets":[0,1]}}"#;
    assert_err!(parse_artifact(&st(header, &[0]), &limits()), ArtifactError::DuplicateTensorName(s) if s == "ab");
    // Escaped duplicate descriptor field: "d\u0074ype" decodes to "dtype".
    let dup = r#"{"a":{"dtype":"U8","d\u0074ype":"U8","shape":[1],"data_offsets":[0,1]}}"#;
    assert_err!(parse_artifact(&st(dup, &[0]), &limits()), ArtifactError::DuplicateDescriptorField(s) if s == "dtype");
}

#[test]
fn finds_tensor_regardless_of_header_or_offset_order() {
    // Names declared in one order; byte ranges deliberately reverse-sorted so
    // the offset-ordered output differs from declaration order. Lookup by name
    // must still find both, and the vec must be (begin,end)-ordered.
    let header = r#"{"first":{"dtype":"U8","shape":[2],"data_offsets":[0,2]},"second":{"dtype":"U8","shape":[2],"data_offsets":[2,4]}}"#;
    let file = st(header, &[1, 2, 3, 4]);
    let art = parse_artifact(&file, &limits()).expect("reverse-declared order valid");
    assert_eq!(art.tensors[0].name, "first");
    assert_eq!(art.tensors[1].name, "second");
    // Empty range sharing a boundary must sort by end after the filled one.
    // (begin,end) tie-break: [0,4] sorts before [0,0], so the filled range
    // tiles first and the empty one shares its end boundary.
    let header2 = r#"{"emp":{"dtype":"U8","shape":[0],"data_offsets":[4,4]},"full":{"dtype":"U8","shape":[4],"data_offsets":[0,4]}}"#;
    let file2 = st(header2, &[9, 9, 9, 9]);
    let art2 = parse_artifact(&file2, &limits())
        .expect("empty range sharing boundary with filled range is valid");
    assert_eq!(art2.tensors[0].name, "full");
    assert_eq!(art2.tensors[1].name, "emp");
}

#[test]
fn rejects_non_string_root_key_and_descriptor_not_object() {
    // Root value that is not an object at all.
    let header = r#"{"a":42}"#;
    assert_err!(
        parse_artifact(&st(header, &[]), &limits()),
        ArtifactError::DescriptorNotObject
    );
}
