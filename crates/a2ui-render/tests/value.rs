//! Dynamic-value tests: literals, JSON-Pointer bindings (absolute and
//! relative to a list item), and the catalog functions a real document uses.

use serde_json::{json, Value};

use a2ui_render::value::{self, Ctx, format_number};

fn ctx<'a>(data: &'a Value) -> Ctx<'a> {
    Ctx { data, item: None }
}

fn one(v: &Value, data: &Value) -> String {
    let mut d = Vec::new();
    let s = value::text(v, &ctx(data), &mut d);
    assert!(d.is_empty(), "unexpected diagnostics: {d:?}");
    s
}

fn with_diag(v: &Value, data: &Value) -> (String, Vec<String>) {
    let mut d = Vec::new();
    let s = value::text(v, &ctx(data), &mut d);
    (s, d)
}

const MODEL: &str = r#"{
    "city": "Oslo",
    "count": 3,
    "price": 1234.5,
    "on": true,
    "empty": "",
    "items": [{"title": "Map", "mins": 12}, {"title": "Charger", "mins": 3}],
    "deep": {"nested": {"value": "found"}}
}"#;

fn model() -> Value {
    serde_json::from_str(MODEL).unwrap()
}

#[test]
fn literals_pass_through() {
    let m = model();
    assert_eq!(one(&json!("plain"), &m), "plain");
    assert_eq!(one(&json!(42), &m), "42");
    assert_eq!(one(&json!({"literalString": "spelled out"}), &m), "spelled out");
}

#[test]
fn absolute_binding_walks_the_model() {
    let m = model();
    assert_eq!(one(&json!({"path": "/city"}), &m), "Oslo");
    assert_eq!(one(&json!({"path": "/deep/nested/value"}), &m), "found");
    assert_eq!(one(&json!({"path": "/items/0/title"}), &m), "Map");
}

#[test]
fn missing_binding_is_blank_not_fatal() {
    let m = model();
    let (s, d) = with_diag(&json!({"path": "/not/there"}), &m);
    assert_eq!(s, "", "progressive rendering: no data yet, no text");
    assert!(d.is_empty(), "a missing path is expected, not a complaint: {d:?}");
}

#[test]
fn relative_binding_prefers_the_template_item_then_the_model() {
    let m = model();
    let item = &m["items"][1];
    let c = Ctx { data: &m, item: Some(item) };
    let mut d = Vec::new();
    assert_eq!(value::text(&json!({"path": "title"}), &c, &mut d), "Charger");
    assert_eq!(value::text(&json!({"path": "/city"}), &c, &mut d), "Oslo", "absolute still hits the model");
    // A relative path the item does not have falls back to the model root.
    assert_eq!(value::text(&json!({"path": "city"}), &c, &mut d), "Oslo");
    assert!(d.is_empty(), "{d:?}");
}

#[test]
fn numbers_and_flags_read_through_bindings() {
    let m = model();
    let mut d = Vec::new();
    assert_eq!(value::number(&json!({"path": "/count"}), &ctx(&m), &mut d), Some(3.0));
    assert_eq!(value::number(&json!({"path": "/city"}), &ctx(&m), &mut d), None);
    assert_eq!(value::number(&json!("17.5"), &ctx(&m), &mut d), Some(17.5), "agents send numeric strings");
    assert!(value::flag(&json!({"path": "/on"}), &ctx(&m), &mut d));
    assert!(!value::flag(&json!({"path": "/empty"}), &ctx(&m), &mut d));
    assert!(!value::flag(&json!({"path": "/gone"}), &ctx(&m), &mut d));
}

#[test]
fn stringify_is_predictable() {
    assert_eq!(value::stringify(&json!(null)), "");
    assert_eq!(value::stringify(&json!(7.0)), "7");
    assert_eq!(value::stringify(&json!(2.5)), "2.5");
    assert_eq!(value::stringify(&json!(true)), "true");
    assert_eq!(value::stringify(&json!(["a"])), "[\"a\"]");
}

#[test]
fn format_number_groups_and_rounds() {
    assert_eq!(format_number(1234.5, None, true), "1,234.50");
    assert_eq!(format_number(1234.0, None, true), "1,234");
    assert_eq!(format_number(1234.567, Some(1), true), "1,234.6");
    assert_eq!(format_number(1234.5, Some(2), false), "1234.50");
    assert_eq!(format_number(-9876.5, Some(0), true), "-9,877");
    assert_eq!(format_number(0.0, Some(0), true), "0");
    assert_eq!(format_number(f64::NAN, None, true), "", "no NaN on the screen");
}

#[test]
fn format_number_call_reads_dynamic_args() {
    let m = model();
    let call = json!({"call": "formatNumber", "args": {"value": {"path": "/price"}, "decimals": 2}});
    assert_eq!(one(&call, &m), "1,234.50");
    let call = json!({"call": "formatNumber", "args": {"value": {"path": "/price"}, "grouping": false}});
    assert_eq!(one(&call, &m), "1234.50");
}

#[test]
fn format_string_interpolates_paths_and_escapes() {
    let m = model();
    let call = json!({"call": "formatString", "args": {"value": "Heading to ${/city} in ${/count} min"}});
    assert_eq!(one(&call, &m), "Heading to Oslo in 3 min");

    let esc = json!({"call": "formatString", "args": {"value": "price \\${/count} stays literal"}});
    assert_eq!(one(&esc, &m), "price ${/count} stays literal");

    let miss = json!({"call": "formatString", "args": {"value": "x ${/nope} y"}});
    let (s, d) = with_diag(&miss, &m);
    assert_eq!(s, "x  y", "a missing path leaves a hole, it does not blank the line");
    assert_eq!(d.len(), 1, "{d:?}");
}

#[test]
fn format_string_can_nest_function_calls() {
    let m = model();
    let call = json!({"call": "formatString", "args": {
        "value": "Total: ${formatNumber(value:/price, decimals:0)} kr"
    }});
    assert_eq!(one(&call, &m), "Total: 1,235 kr");

    let quoted = json!({"call": "formatString", "args": {
        "value": "${pluralize(value:/count, one:\"stop\", other:\"stops\")}"
    }});
    assert_eq!(one(&quoted, &m), "stops");
    let one_item = json!({"call": "formatString", "args": {
        "value": "${pluralize(value:/n, one:\"stop\", other:\"stops\")}"
    }});
    let m1 = json!({"n": 1});
    assert_eq!(one(&one_item, &m1), "stop");
}

#[test]
fn format_string_relative_paths_use_the_item() {
    let m = model();
    let item = &m["items"][0];
    let c = Ctx { data: &m, item: Some(item) };
    let mut d = Vec::new();
    let call = json!({"call": "formatString", "args": {"value": "${title}: ${mins} min"}});
    assert_eq!(value::text(&call, &c, &mut d), "Map: 12 min");
    assert!(d.is_empty(), "{d:?}");
}

#[test]
fn unknown_function_is_named_and_blank() {
    let m = model();
    let call = json!({"call": "quantumSpin", "args": {"value": "x"}});
    let (s, d) = with_diag(&call, &m);
    assert_eq!(s, "");
    assert!(d.iter().any(|x| x.contains("quantumSpin")), "{d:?}");
}

#[test]
fn unterminated_interpolation_is_reported() {
    let m = model();
    let call = json!({"call": "formatString", "args": {"value": "oops ${/city"}});
    let (s, d) = with_diag(&call, &m);
    assert!(s.contains("oops"), "{s}");
    assert!(d.iter().any(|x| x.contains("unterminated")), "{d:?}");
}

#[test]
fn boolean_functions_work() {
    let m = model();
    let mut d = Vec::new();
    let mut t = |v: Value| value::flag(&v, &ctx(&m), &mut d);
    assert!(t(json!({"call": "and", "args": {"values": [{"path": "/on"}, true]}})));
    assert!(!t(json!({"call": "and", "args": {"values": [{"path": "/on"}, false]}})));
    assert!(t(json!({"call": "or", "args": {"values": [{"path": "/empty"}, {"path": "/on"}]}})));
    assert!(!t(json!({"call": "not", "args": {"value": {"path": "/on"}}})));
    assert!(t(json!({"call": "required", "args": {"value": {"path": "/city"}}})));
    assert!(!t(json!({"call": "required", "args": {"value": {"path": "/empty"}}})));
    assert!(d.is_empty(), "{d:?}");
}

#[test]
fn open_url_is_recorded_not_silent() {
    let m = model();
    let call = json!({"call": "openUrl", "args": {"url": "https://example.org"}});
    let (_, d) = with_diag(&call, &m);
    assert!(d.iter().any(|x| x.contains("no browser") && x.contains("example.org")), "{d:?}");
}

#[test]
fn absurd_nesting_is_refused_not_hung() {
    let m = model();
    let mut deep = json!({"path": "/on"});
    for _ in 0..60 {
        deep = json!({"call": "not", "args": {"value": deep}});
    }
    let mut d = Vec::new();
    let _ = value::resolve(&deep, &ctx(&m), &mut d);
    assert_eq!(d.len(), 1, "evaluation is cut once, at the depth limit: {d:?}");
    assert!(d[0].contains("nests too deeply"), "{d:?}");
}

#[test]
fn a_property_that_is_none_of_the_three_shapes_is_reported() {
    let m = model();
    let (s, d) = with_diag(&json!({"nonsense": {"a": 1}}), &m);
    assert!(!s.is_empty(), "the value is handed back, not dropped");
    assert!(d.iter().any(|x| x.contains("not a literal/binding/call")), "{d:?}");
}
