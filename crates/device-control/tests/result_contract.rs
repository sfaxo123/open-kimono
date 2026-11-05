//! DC-302B result-contract freeze tests. Four focused checks: the frozen
//! bounds, distinct error names, fixture-table internal consistency, and
//! UnknownOutcome being a discriminant distinct from Failure. No codec is
//! exercised — these pin the contract a later codec packet must honour.

use device_control::command_record::common::MAX_COMMAND_BODY;
use device_control::command_record::result_contract::{
    ActionCommandRef, ActionResult, FixtureRow, ResultContractError, ResultStatus, StreamBody,
    FIXTURES, MAX_STREAM_BODY,
};

/// Test 1: constants equal the expected values and the combined rule holds.
#[test]
fn bounds_match_shared_bound_and_combined_rule_holds() {
    assert_eq!(MAX_COMMAND_BODY, 65_536);
    assert_eq!(MAX_STREAM_BODY, MAX_COMMAND_BODY / 2);
    assert_eq!(MAX_STREAM_BODY, 32_768);
    // Combined rule: both per-stream caps fit inside the whole-body bound.
    assert!(MAX_STREAM_BODY * 2 <= MAX_COMMAND_BODY);
}

/// Test 2: every ResultContractError variant has a distinct Debug name —
/// no two variants may stringify alike (the DC-302R conflation objection).
#[test]
fn error_variants_have_distinct_debug_names() {
    let variants = [
        ResultContractError::CorrelationMismatch,
        ResultContractError::BodyLengthMismatch,
        ResultContractError::TruncatedFlagMismatch,
        ResultContractError::InvalidDetail,
        ResultContractError::BudgetExceeded,
    ];
    let names: Vec<String> = variants.iter().map(|e| format!("{e:?}")).collect();
    for (i, a) in names.iter().enumerate() {
        assert!(!a.is_empty(), "variant {i} has an empty Debug name");
        for (j, b) in names.iter().enumerate() {
            if i != j {
                assert_ne!(a, b, "variants {i} and {j} share Debug name {a}");
            }
        }
    }
    // The five frozen names are pinned verbatim.
    assert_eq!(
        names,
        [
            "CorrelationMismatch",
            "BodyLengthMismatch",
            "TruncatedFlagMismatch",
            "InvalidDetail",
            "BudgetExceeded",
        ]
    );
}

/// Test 3: the fixture table is internally consistent — every truncated
/// flag matches actual cap occupancy, every length is within the cap, and
/// every expected error name resolves to a distinct frozen variant.
#[test]
fn fixture_table_is_internally_consistent() {
    assert!(
        FIXTURES.len() >= 4,
        "fixture table must have at least 4 rows"
    );
    let mut seen_errors: Vec<&'static str> = Vec::new();
    for row in FIXTURES.iter() {
        // Flag matches cap occupancy exactly (constructed-value rule).
        assert_eq!(
            row.stdout_truncated,
            row.stdout_len == MAX_STREAM_BODY,
            "row {} stdout flag/occupancy mismatch",
            row.name
        );
        assert_eq!(
            row.stderr_truncated,
            row.stderr_len == MAX_STREAM_BODY,
            "row {} stderr flag/occupancy mismatch",
            row.name
        );
        // Lengths within the per-stream cap.
        assert!(row.stdout_len <= MAX_STREAM_BODY);
        assert!(row.stderr_len <= MAX_STREAM_BODY);
        // Error names resolve and stay distinct across error rows.
        if let Some(e) = row.expected_error {
            let name = format!("{e:?}");
            assert!(!seen_errors.contains(&name.as_str()), "dup error {name}");
            seen_errors.push(match e {
                ResultContractError::CorrelationMismatch => "CorrelationMismatch",
                ResultContractError::BodyLengthMismatch => "BodyLengthMismatch",
                ResultContractError::TruncatedFlagMismatch => "TruncatedFlagMismatch",
                ResultContractError::InvalidDetail => "InvalidDetail",
                ResultContractError::BudgetExceeded => "BudgetExceeded",
            });
        }
    }
    // Required coverage rows exist by name.
    let names: Vec<&str> = FIXTURES.iter().map(|r: &FixtureRow| r.name).collect();
    for required in [
        "both_streams_at_cap_truncated",
        "oversize_input_yields_flagged_prefix",
        "correlation_mismatch",
        "unknown_outcome_distinct_from_failure",
    ] {
        assert!(names.contains(&required), "missing fixture row {required}");
    }
    // Oversize-input scenario is documented as prefix+flag: accepted, no error.
    let oversize = FIXTURES
        .iter()
        .find(|r| r.name == "oversize_input_yields_flagged_prefix")
        .unwrap();
    assert!(oversize.expected_error.is_none());
    assert!(oversize.stdout_truncated && oversize.stderr_truncated);
    assert_eq!(oversize.stdout_len, MAX_STREAM_BODY);
    assert_eq!(oversize.stderr_len, MAX_STREAM_BODY);
    // Correlation row rejects with exactly CorrelationMismatch.
    let corr = FIXTURES
        .iter()
        .find(|r| r.name == "correlation_mismatch")
        .unwrap();
    assert_eq!(
        corr.expected_error,
        Some(ResultContractError::CorrelationMismatch)
    );
}

/// Test 4: UnknownOutcome and Failure are distinct discriminants — never
/// conflated as values, as Debug names, or in a constructed result.
#[test]
fn unknown_outcome_is_distinct_from_failure() {
    assert_ne!(ResultStatus::UnknownOutcome, ResultStatus::Failure);
    assert_ne!(
        format!("{:?}", ResultStatus::UnknownOutcome),
        format!("{:?}", ResultStatus::Failure)
    );

    // A constructed result carrying UnknownOutcome is a different value
    // from the same result carrying Failure.
    let empty = StreamBody {
        bytes: [0u8; MAX_STREAM_BODY],
        len: 0,
        truncated: false,
    };
    let corr = ActionCommandRef {
        session_id: 7,
        request_seq: 3,
    };
    let unknown = ActionResult {
        status: ResultStatus::UnknownOutcome,
        stdout: empty,
        stderr: empty,
        correlation: corr,
    };
    let failed = ActionResult {
        status: ResultStatus::Failure,
        ..unknown
    };
    assert_ne!(unknown, failed);
    // A constructed flagged-short prefix is invalid per the frozen rule:
    // at_cap() reports the occupancy the flag must match.
    let short_flagged = StreamBody {
        bytes: [0u8; MAX_STREAM_BODY],
        len: 10,
        truncated: true,
    };
    assert!(!short_flagged.at_cap());
}
