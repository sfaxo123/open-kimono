//! Surface state tests: the protocol's open/close rules, upsert-by-id,
//! data-model patching, and text-field two-way binding.

use serde_json::{json, Value};

use a2ui_render::doc::{parse_document, Message};
use a2ui_render::surface::Surface;

fn msgs(text: &str) -> Vec<Message> {
    let d = parse_document(text);
    assert!(d.errors.is_empty(), "{:?}", d.errors);
    d.messages
}

#[test]
fn open_surface_holds_components_and_model() {
    let mut s = Surface::default();
    assert!(!s.is_open());
    for m in msgs(
        r#"{"version":"v0.9","createSurface":{"surfaceId":"s","catalogId":"basic"}}"#,
    ) {
        s.apply(&m).unwrap();
    }
    assert!(s.is_open());
    assert_eq!(s.catalog_id, "basic");

    for m in msgs(
        r#"{"version":"v0.9","updateComponents":{"surfaceId":"s","components":[
            {"id":"root","component":"Column","children":["a"]},
            {"id":"a","component":"Text","text":"hello"}]}}"#,
    ) {
        s.apply(&m).unwrap();
    }
    assert_eq!(s.components.len(), 2);
    assert_eq!(s.root().unwrap()["component"], "Column");
    assert_eq!(s.component("a").unwrap()["text"], "hello");
}

#[test]
fn second_create_surface_is_an_error_not_a_reset() {
    let mut s = Surface::default();
    s.apply(&msgs(r#"{"version":"v0.9","createSurface":{"surfaceId":"s","catalogId":"c"}}"#)[0]).unwrap();
    let err = s
        .apply(&msgs(r#"{"version":"v0.9","createSurface":{"surfaceId":"s2","catalogId":"c"}}"#)[0])
        .unwrap_err();
    assert!(err.contains("already open"), "{err}");
    assert_eq!(s.id, "s", "the live surface is untouched");
}

#[test]
fn messages_for_the_wrong_surface_are_refused() {
    let mut s = Surface::default();
    s.apply(&msgs(r#"{"version":"v0.9","createSurface":{"surfaceId":"s","catalogId":"c"}}"#)[0]).unwrap();
    let err = s
        .apply(&msgs(r#"{"version":"v0.9","updateComponents":{"surfaceId":"other","components":[{"id":"root","component":"Text","text":"x"}]}}"#)[0])
        .unwrap_err();
    assert!(err.contains("open surface is `s`"), "{err}");

    let err = s
        .apply(&msgs(r#"{"version":"v0.9","deleteSurface":{"surfaceId":"nope"}}"#)[0])
        .unwrap_err();
    assert!(err.contains("open surface"), "{err}");
}

#[test]
fn update_before_create_is_refused() {
    let mut s = Surface::default();
    let err = s
        .apply(&msgs(r#"{"version":"v0.9","updateDataModel":{"surfaceId":"s","value":{}}}"#)[0])
        .unwrap_err();
    assert!(err.contains("no surface is open"), "{err}");
}

#[test]
fn components_are_upserted_by_id_so_updates_patch_the_tree() {
    let mut s = Surface::default();
    for m in msgs(
        r#"{"version":"v0.9","createSurface":{"surfaceId":"s","catalogId":"c"}}"#,
    ) {
        s.apply(&m).unwrap();
    }
    for m in msgs(r#"{"version":"v0.9","updateComponents":{"surfaceId":"s","components":[{"id":"root","component":"Text","text":"one"}]}}"#) {
        s.apply(&m).unwrap();
    }
    for m in msgs(r#"{"version":"v0.9","updateComponents":{"surfaceId":"s","components":[{"id":"root","component":"Text","text":"two"}]}}"#) {
        s.apply(&m).unwrap();
    }
    assert_eq!(s.components.len(), 1);
    assert_eq!(s.root().unwrap()["text"], "two");
}

#[test]
fn data_model_patch_create_and_remove() {
    let mut s = Surface::default();
    let script = r#"{"messages":[
        {"version":"v0.9","createSurface":{"surfaceId":"s","catalogId":"c"}},
        {"version":"v0.9","updateDataModel":{"surfaceId":"s","path":"/","value":{"a":1}}},
        {"version":"v0.9","updateDataModel":{"surfaceId":"s","path":"/b/c","value":7}},
        {"version":"v0.9","updateDataModel":{"surfaceId":"s","path":"/a","value":2}},
        {"version":"v0.9","updateDataModel":{"surfaceId":"s","path":"/b/c"}}
    ]}"#;
    for m in msgs(script) {
        s.apply(&m).unwrap();
    }
    assert_eq!(s.data, json!({"a": 2, "b": {}}), "created parents survive; /b/c removed");
}

#[test]
fn data_model_write_through_a_scalar_is_reported() {
    let mut s = Surface::default();
    for m in msgs(r#"{"version":"v0.9","createSurface":{"surfaceId":"s","catalogId":"c"}}"#) {
        s.apply(&m).unwrap();
    }
    for m in msgs(r#"{"version":"v0.9","updateDataModel":{"surfaceId":"s","value":{"a":"str"}}}"#) {
        s.apply(&m).unwrap();
    }
    let err = s
        .apply(&msgs(r#"{"version":"v0.9","updateDataModel":{"surfaceId":"s","path":"/a/deep","value":1}}"#)[0])
        .unwrap_err();
    assert!(err.contains("cannot descend"), "{err}");
}

#[test]
fn text_field_edit_writes_local_value_and_the_bound_model_path() {
    let mut s = Surface::default();
    let script = r#"{"messages":[
        {"version":"v0.9","createSurface":{"surfaceId":"s","catalogId":"c"}},
        {"version":"v0.9","updateComponents":{"surfaceId":"s","components":[
            {"id":"root","component":"TextField","label":"City","value":{"path":"/city"}}]}},
        {"version":"v0.9","updateDataModel":{"surfaceId":"s","value":{"city":"Oslo"}}}
    ]}"#;
    for m in msgs(script) {
        s.apply(&m).unwrap();
    }
    let path = s.set_field_value("root", "Bergen");
    assert_eq!(path.as_deref(), Some("/city"));
    assert_eq!(s.local_values["root"], "Bergen");
    assert_eq!(s.data["city"], json!("Bergen"), "two-way binding updates the model");
}

#[test]
fn unbound_text_field_keeps_only_the_local_value() {
    let mut s = Surface::default();
    for m in msgs(r#"{"version":"v0.9","createSurface":{"surfaceId":"s","catalogId":"c"}}"#) {
        s.apply(&m).unwrap();
    }
    for m in msgs(r#"{"version":"v0.9","updateComponents":{"surfaceId":"s","components":[
        {"id":"f","component":"TextField","label":"Note","value":"literal"}]}}"#)
    {
        s.apply(&m).unwrap();
    }
    assert_eq!(s.set_field_value("f", "typed"), None);
    assert_eq!(s.local_values["f"], "typed");
    assert_eq!(s.data, Value::Object(Default::default()));
}

#[test]
fn delete_surface_closes_everything() {
    let mut s = Surface::default();
    let script = r#"{"messages":[
        {"version":"v0.9","createSurface":{"surfaceId":"s","catalogId":"c"}},
        {"version":"v0.9","updateComponents":{"surfaceId":"s","components":[{"id":"root","component":"Text","text":"x"}]}},
        {"version":"v0.9","deleteSurface":{"surfaceId":"s"}}
    ]}"#;
    for m in msgs(script) {
        s.apply(&m).unwrap();
    }
    assert!(!s.is_open());
    assert!(s.components.is_empty());
}
