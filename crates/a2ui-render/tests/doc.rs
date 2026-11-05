//! Message parsing tests: the four v0.9 messages, the three document shapes,
//! and precise errors for the ways a generated document goes wrong.

use serde_json::json;

use a2ui_render::doc::{parse_document, parse_jsonl, parse_message, Message};

const CREATE: &str = r#"{"version":"v0.9","createSurface":{"surfaceId":"s1","catalogId":"basic"}}"#;

#[test]
fn create_surface_message_parses() {
    let d = parse_document(CREATE);
    assert!(d.errors.is_empty(), "{:?}", d.errors);
    assert_eq!(
        d.messages,
        vec![Message::CreateSurface {
            surface_id: "s1".into(),
            catalog_id: "basic".into(),
            theme: None,
            send_data_model: false,
        }]
    );
}

#[test]
fn gallery_wrapper_array_of_messages_parses() {
    // The shape every spec example file uses.
    let doc = json!({
        "name": "Simple Text",
        "messages": [
            {"version": "v0.9", "createSurface": {"surfaceId": "g", "catalogId": "basic"}},
            {"version": "v0.9", "updateComponents": {"surfaceId": "g", "components": [
                {"id": "root", "component": "Text", "text": "hi", "variant": "h1"}
            ]}},
            {"version": "v0.9", "updateDataModel": {"surfaceId": "g", "path": "/x", "value": 3}},
            {"version": "v0.9.1", "deleteSurface": {"surfaceId": "g"}}
        ]
    });
    let d = parse_document(&doc.to_string());
    assert!(d.errors.is_empty(), "{:?}", d.errors);
    assert_eq!(d.messages.len(), 4);
    assert!(matches!(d.messages[1], Message::UpdateComponents { ref components, .. } if components.len() == 1));
    match &d.messages[2] {
        Message::UpdateDataModel { path, value, .. } => {
            assert_eq!(path.as_deref(), Some("/x"));
            assert_eq!(value, &Some(json!(3)));
        }
        other => panic!("wrong message: {other:?}"),
    }
    assert!(matches!(d.messages[3], Message::DeleteSurface { .. }));
}

#[test]
fn bare_array_document_parses() {
    let d = parse_document(&format!("[{CREATE}]"));
    assert_eq!(d.messages.len(), 1, "{:?}", d.errors);
}

#[test]
fn jsonl_stream_parses_and_counts_bad_lines() {
    let text = format!("{CREATE}\n\nnot json\n{{\"version\":\"v0.9\",\"deleteSurface\":{{\"surfaceId\":\"s1\"}}}}\n");
    let d = parse_jsonl(&text);
    assert_eq!(d.messages.len(), 2, "{:?}", d.errors);
    assert_eq!(d.errors.len(), 1);
    assert_eq!(d.errors[0].index, Some(2), "the bad line is reported with its line number");
}

#[test]
fn missing_version_is_rejected_with_a_named_reason() {
    let e = parse_message(&json!({"createSurface": {"surfaceId": "s", "catalogId": "c"}})).unwrap_err();
    assert!(e.contains("version"), "{e}");
}

#[test]
fn other_protocol_versions_are_refused_not_guessed() {
    let e = parse_message(&json!({"version": "v0.8", "createSurface": {"surfaceId": "s", "catalogId": "c"}})).unwrap_err();
    assert!(e.contains("v0.8"), "{e}");
}

#[test]
fn two_payload_keys_is_an_error() {
    let e = parse_message(&json!({
        "version": "v0.9",
        "createSurface": {"surfaceId": "s", "catalogId": "c"},
        "deleteSurface": {"surfaceId": "s"}
    }))
    .unwrap_err();
    assert!(e.contains("exactly one payload key"), "{e}");
}

#[test]
fn unknown_message_type_is_named() {
    let e = parse_message(&json!({"version": "v0.9", "teleportSurface": {"surfaceId": "s"}})).unwrap_err();
    assert!(e.contains("teleportSurface"), "{e}");
}

#[test]
fn components_without_id_or_component_are_rejected_by_index() {
    let e = parse_message(&json!({
        "version": "v0.9",
        "updateComponents": {"surfaceId": "s", "components": [{"component": "Text", "text": "x"}]}
    }))
    .unwrap_err();
    assert!(e.contains("components[0]") && e.contains("`id`"), "{e}");

    let e = parse_message(&json!({
        "version": "v0.9",
        "updateComponents": {"surfaceId": "s", "components": [{"id": "a"}]}
    }))
    .unwrap_err();
    assert!(e.contains("components[0]") && e.contains("`component`"), "{e}");
}

#[test]
fn empty_components_array_is_rejected() {
    let e = parse_message(&json!({
        "version": "v0.9", "updateComponents": {"surfaceId": "s", "components": []}
    }))
    .unwrap_err();
    assert!(e.contains("non-empty"), "{e}");
}

#[test]
fn a_bad_message_does_not_sink_the_document() {
    let text = format!(
        "[{CREATE}, {}, {{\"version\":\"v0.9\",\"deleteSurface\":{{\"surfaceId\":\"s1\"}}}}]",
        json!({"version": "v0.9", "updateComponents": {"surfaceId": "s1"}})
    );
    let d = parse_document(&text);
    assert_eq!(d.messages.len(), 2, "the good messages still render");
    assert_eq!(d.errors.len(), 1);
    assert_eq!(d.errors[0].index, Some(1));
}

#[test]
fn invalid_json_document_reports_once() {
    let d = parse_document("{\"version\": \"v0.9\", ");
    assert_eq!(d.errors.len(), 1);
    assert!(d.errors[0].message.contains("invalid JSON"), "{}", d.errors[0].message);
    assert!(d.messages.is_empty());
}

#[test]
fn empty_document_is_reported_not_silent() {
    let d = parse_document("   ");
    assert_eq!(d.errors.len(), 1);
    assert!(d.errors[0].message.contains("empty"));
}
