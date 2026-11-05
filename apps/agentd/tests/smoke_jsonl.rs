//! A0 tests: the smoke line is a real journal record, and the journal parser
//! rejects the ways it could be wrong.

use agentd::smoke::{self, HostInfo, EVENT_NAME, SERVICE};

fn host() -> HostInfo {
    HostInfo { hostname: "oneplus-enchilada".into(), os_release: "7.1.0-rc1-sdm845".into(), arch: "qcom,sdm845".into() }
}

#[test]
fn smoke_line_round_trips_through_the_journal_schema() {
    let line = smoke::smoke_line(1_730_000_000_000_000, &host()).expect("serialize");
    assert!(!line.contains('\n'), "one journal record must be one line");

    let event = trace_events::from_json_line(&line).expect("the smoke line must parse as an Event");
    assert_eq!(event.seq, 0, "the standalone smoke binary has no journal to assign a seq");
    assert_eq!(event.ts_us, 1_730_000_000_000_000);
    assert_eq!(event.service.as_deref(), Some(SERVICE));

    let json: serde_json::Value = serde_json::from_str(&line).unwrap();
    // The payload tag is internal: it sits inside the `payload` object.
    assert_eq!(json["payload"]["kind"], "ag_ui", "payload tag must be a copied vocabulary, not an invented one");
    assert_eq!(json["payload"]["type"], "CUSTOM");
    assert_eq!(json["payload"]["name"], EVENT_NAME);
    assert_eq!(json["payload"]["value"]["hostname"], "oneplus-enchilada");
    assert_eq!(json["payload"]["value"]["os_release"], "7.1.0-rc1-sdm845");
}

#[test]
fn target_is_reported_as_musl_only_for_musl_builds() {
    // The phone is musl; a glibc binary there fails with "not found", so the
    // event must state which libc it statically linked against.
    if smoke::target_is_musl() {
        assert_eq!(smoke::target_triple(), format!("{}-unknown-linux-musl", std::env::consts::ARCH));
    } else {
        assert!(!smoke::target_triple().ends_with("musl"));
    }
}

// — negative cases: the parser must reject a broken line with a named error —

#[test]
fn truncated_line_is_rejected_as_incomplete_json() {
    let line = smoke::smoke_line(1_730_000_000_000_000, &host()).unwrap();
    let broken = &line[..line.len() - 7];
    let err = trace_events::from_json_line(broken).expect_err("a truncated record must not parse");
    assert!(err.is_eof() || err.to_string().contains("EOF while parsing"), "got: {err}");
}

#[test]
fn unknown_payload_kind_is_rejected() {
    let err = trace_events::from_json_line(r#"{"seq":0,"ts_us":1,"payload":{"kind":"not_a_vocabulary"}}"#)
        .expect_err("invented payload kinds must be rejected");
    assert!(err.to_string().contains("unknown variant"), "got: {err}");
}

#[test]
fn missing_payload_field_is_rejected() {
    let err = trace_events::from_json_line(r#"{"seq":0,"ts_us":1}"#)
        .expect_err("an envelope without a payload is not an event");
    assert!(err.to_string().contains("missing field `payload`"), "got: {err}");
}

#[test]
fn custom_event_without_a_name_is_rejected() {
    let err = trace_events::from_json_line(
        r#"{"seq":0,"ts_us":1,"payload":{"kind":"ag_ui","type":"CUSTOM","value":{}}}"#,
    )
    .expect_err("CUSTOM requires a name");
    assert!(err.to_string().contains("missing field `name`"), "got: {err}");
}
