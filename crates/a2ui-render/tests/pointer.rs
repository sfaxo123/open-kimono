//! JSON Pointer tests, including the RFC 6901 escape rules and the failure
//! modes a generated document can hit.

use serde_json::json;

use a2ui_render::pointer::{lookup, remove, segments, set};

#[test]
fn segments_unescape_per_rfc6901() {
    assert_eq!(segments("/a/b"), ["a", "b"]);
    assert_eq!(segments("/m~1n/~0x"), ["m/n", "~x"]);
    assert!(segments("").is_empty());
    assert!(segments("/").is_empty());
}

#[test]
fn lookup_walks_objects_and_arrays() {
    let v = json!({"a": {"b": [10, 20, {"c": "deep"}]}, "m/n": "slash key"});
    assert_eq!(lookup(&v, "/a/b/1"), Some(&json!(20)));
    assert_eq!(lookup(&v, "/a/b/2/c"), Some(&json!("deep")));
    assert_eq!(lookup(&v, "/m~1n"), Some(&json!("slash key")));
    assert_eq!(lookup(&v, "/"), Some(&v), "the root pointer is the whole model");
}

#[test]
fn lookup_misses_are_none_not_panics() {
    let v = json!({"a": "scalar"});
    assert_eq!(lookup(&v, "/a/b"), None, "cannot walk through a string");
    assert_eq!(lookup(&v, "/nope"), None);
    assert_eq!(lookup(&v, "/a/0"), None, "cannot index a string");
    assert_eq!(lookup(&json!([1, 2]), "/5"), None, "index past the end");
    assert_eq!(lookup(&json!([1, 2]), "/x"), None, "non-numeric index into an array");
}

#[test]
fn set_creates_missing_object_parents() {
    let mut v = json!({});
    set(&mut v, "/a/b/c", json!(1)).unwrap();
    assert_eq!(v, json!({"a": {"b": {"c": 1}}}));
}

#[test]
fn set_replaces_in_place_and_appends_at_array_end() {
    let mut v = json!({"xs": [1, 2]});
    set(&mut v, "/xs/0", json!(9)).unwrap();
    set(&mut v, "/xs/2", json!(3)).unwrap();
    assert_eq!(v, json!({"xs": [9, 2, 3]}));
    let err = set(&mut v, "/xs/9", json!(0)).unwrap_err();
    assert!(err.contains("past the end"), "{err}");
}

#[test]
fn set_at_root_replaces_the_model() {
    let mut v = json!({"a": 1});
    set(&mut v, "/", json!({"b": 2})).unwrap();
    assert_eq!(v, json!({"b": 2}));
}

#[test]
fn remove_drops_keys_and_array_slots() {
    let mut v = json!({"a": 1, "xs": [1, 2, 3]});
    remove(&mut v, "/a").unwrap();
    remove(&mut v, "/xs/1").unwrap();
    assert_eq!(v, json!({"xs": [1, 3]}));
    assert!(remove(&mut v, "/gone").is_err());
}
