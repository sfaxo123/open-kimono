use lfm3_brain::artifacts::{parse_artifact, ArtifactError, ArtifactLimits};

fn file(header: &str, payload: &[u8]) -> Vec<u8> {
    let mut bytes = (header.len() as u64).to_le_bytes().to_vec();
    bytes.extend_from_slice(header.as_bytes());
    bytes.extend_from_slice(payload);
    bytes
}

#[test]
fn duplicate_diagnostics_preserve_adversarial_names() {
    // Neither colons nor serde's diagnostic suffix are delimiters in JSON keys.
    let key = "weight at line 3 column 1:metadata";
    let encoded = serde_json::to_string(key).unwrap();
    let descriptor = r#"{"dtype":"U8","shape":[1],"data_offsets":[0,1]}"#;
    let header = format!("{{{encoded}:{descriptor},{encoded}:{descriptor}}}");
    assert_eq!(
        parse_artifact(&file(&header, &[7]), &ArtifactLimits::default()).unwrap_err(),
        ArtifactError::DuplicateTensorName(key.into())
    );
}

#[test]
fn hostile_declared_lengths_and_deep_json_never_panic() {
    let limits = ArtifactLimits {
        max_header_bytes: u64::MAX,
        max_total_bytes: u64::MAX,
        ..ArtifactLimits::default()
    };
    for length in [u64::MAX, u64::MAX - 8, 1_u64 << 32] {
        let bytes = length.to_le_bytes();
        assert!(parse_artifact(&bytes, &limits).is_err());
    }
    let header = format!("{{\"x\":{}0{}}}", "[".repeat(150), "]".repeat(150));
    assert!(parse_artifact(&file(&header, &[]), &limits).is_err());
}

#[test]
fn every_truncation_of_valid_file_is_rejected() {
    let bytes = file(
        r#"{"w":{"dtype":"F32","shape":[2],"data_offsets":[0,8]}}"#,
        &[0; 8],
    );
    let limits = ArtifactLimits::default();
    assert!(parse_artifact(&bytes, &limits).is_ok());
    for end in 0..bytes.len() {
        assert!(
            parse_artifact(&bytes[..end], &limits).is_err(),
            "accepted prefix {end}"
        );
    }
}

#[test]
fn unsigned_dimensions_and_explicit_dimension_policy() {
    let header =
        r#"{"empty":{"dtype":"U8","shape":[18446744073709551615,0],"data_offsets":[0,0]}}"#;
    let bytes = file(header, &[]);
    assert_eq!(
        parse_artifact(&bytes, &ArtifactLimits::default())
            .unwrap()
            .tensor("empty")
            .unwrap()
            .element_count(),
        0
    );
    let limited = ArtifactLimits {
        max_dimension: 1024,
        ..ArtifactLimits::default()
    };
    assert_eq!(
        parse_artifact(&bytes, &limited).unwrap_err(),
        ArtifactError::DimensionTooLarge
    );
}

#[test]
fn zero_dimension_cancels_other_dimensions_before_overflow() {
    for shape in ["[18446744073709551615,2,0]", "[0,18446744073709551615,2]"] {
        let header =
            format!("{{\"e\":{{\"dtype\":\"F64\",\"shape\":{shape},\"data_offsets\":[0,0]}}}}");
        let bytes = file(&header, &[]);
        let artifact = parse_artifact(&bytes, &ArtifactLimits::default()).unwrap();
        assert_eq!(artifact.tensor("e").unwrap().element_count(), 0);
    }
}

#[test]
fn repeated_workload_and_header_mutations_remain_bounded() {
    let bytes = file(
        r#"{"w":{"dtype":"U8","shape":[2],"data_offsets":[0,2]}}"#,
        &[7, 9],
    );
    let limits = ArtifactLimits {
        max_header_bytes: 1024,
        max_total_bytes: 2048,
        ..ArtifactLimits::default()
    };
    // Each view is dropped every iteration; the caller retains only one input.
    // Process RSS is measured separately. This is not a leak-freedom proof.
    for _ in 0..10_000 {
        let artifact = parse_artifact(&bytes, &limits).unwrap();
        assert_eq!(artifact.tensor("w").unwrap().bytes, &[7, 9]);
    }
    // Exercise every possible byte value at each header position. A mutation
    // may be another valid header; acceptance must preserve all payload bounds.
    let mut mutated = bytes.clone();
    for index in 0..bytes.len() - 2 {
        for value in 0..=255_u8 {
            mutated[index] = value;
            if let Ok(artifact) = parse_artifact(&mutated, &limits) {
                for tensor in &artifact.tensors {
                    assert!(tensor.data_offsets[0] <= tensor.data_offsets[1]);
                    assert!(tensor.data_offsets[1] <= artifact.payload.len() as u64);
                    assert_eq!(
                        tensor.bytes.len() as u64,
                        tensor.data_offsets[1] - tensor.data_offsets[0]
                    );
                }
            }
        }
        mutated[index] = bytes[index];
    }
}
