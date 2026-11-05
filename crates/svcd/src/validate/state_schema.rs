//! The JSON Schema subset svcd enforces on service state, at install
//! (`initial_state`) and before every commit (a transaction whose state would
//! violate the schema is aborted, never persisted).
//!
//! Keywords: `type` (string or array), `required`, `properties`,
//! `additionalProperties` (bool or schema), `items`, `enum`, `const`,
//! `minimum`, `maximum`, `minLength`, `maxLength`, `maxItems`. Annotation
//! keywords (`description`, `default`, `title`, `$comment`) are ignored;
//! anything else is refused so a schema never silently means less than it says.

use serde_json::Value;

use crate::error::ptr;
use crate::expr::json_eq;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SchemaError {
    /// JSON Pointer into the instance (or into the schema for `check_schema`).
    pub path: String,
    pub message: String,
}

const KNOWN: [&str; 17] = [
    "type", "required", "properties", "additionalProperties", "items", "enum", "const", "minimum", "maximum",
    "minLength", "maxLength", "maxItems", "description", "default", "title", "$comment", "format",
];

/// Refuse schema keywords this validator does not implement.
pub fn check_schema(schema: &Value) -> Result<(), SchemaError> {
    check_at(schema, "")
}

fn check_at(schema: &Value, path: &str) -> Result<(), SchemaError> {
    let Some(obj) = schema.as_object() else {
        return Err(SchemaError { path: path.into(), message: "a schema must be an object".into() });
    };
    for (k, v) in obj {
        if !KNOWN.contains(&k.as_str()) {
            return Err(SchemaError { path: ptr(path, k), message: format!("schema keyword `{k}` is not supported by svcd") });
        }
        match k.as_str() {
            "properties" => {
                for (name, sub) in v.as_object().into_iter().flatten() {
                    check_at(sub, &ptr(&ptr(path, "properties"), name))?;
                }
            }
            "items" => check_at(v, &ptr(path, "items"))?,
            "additionalProperties" if v.is_object() => check_at(v, &ptr(path, k))?,
            _ => {}
        }
    }
    Ok(())
}

pub fn validate(schema: &Value, instance: &Value) -> Vec<SchemaError> {
    let mut out = Vec::new();
    walk(schema, instance, "", &mut out);
    out
}

fn type_ok(ty: &str, v: &Value) -> bool {
    match ty {
        "object" => v.is_object(),
        "array" => v.is_array(),
        "string" => v.is_string(),
        "boolean" => v.is_boolean(),
        "null" => v.is_null(),
        "number" => v.is_number(),
        // JSON Schema: an integer is any number with a zero fractional part.
        "integer" => v.as_i64().is_some() || v.as_u64().is_some() || v.as_f64().is_some_and(|f| f.fract() == 0.0),
        _ => false,
    }
}

fn walk(schema: &Value, v: &Value, path: &str, out: &mut Vec<SchemaError>) {
    let mut fail = |msg: String| out.push(SchemaError { path: path.to_string(), message: msg });
    if let Some(ty) = schema.get("type") {
        let types: Vec<&str> = match ty {
            Value::String(s) => vec![s.as_str()],
            Value::Array(a) => a.iter().filter_map(Value::as_str).collect(),
            _ => vec![],
        };
        if !types.iter().any(|t| type_ok(t, v)) {
            fail(format!("{v} is not of type {}", types.join(" | ")));
            return;
        }
    }
    if let Some(Value::Array(options)) = schema.get("enum") {
        if !options.iter().any(|o| json_eq(o, v)) {
            fail(format!("{v} is not one of {}", Value::Array(options.clone())));
        }
    }
    if let Some(c) = schema.get("const") {
        if !json_eq(c, v) {
            fail(format!("{v} must equal {c}"));
        }
    }
    if let (Some(n), Some(min)) = (v.as_f64(), schema.get("minimum").and_then(Value::as_f64)) {
        if n < min {
            fail(format!("{v} is below the minimum {min}"));
        }
    }
    if let (Some(n), Some(max)) = (v.as_f64(), schema.get("maximum").and_then(Value::as_f64)) {
        if n > max {
            fail(format!("{v} is above the maximum {max}"));
        }
    }
    if let Some(s) = v.as_str() {
        let n = s.chars().count() as u64;
        if schema.get("minLength").and_then(Value::as_u64).is_some_and(|m| n < m) {
            fail(format!("string of length {n} is shorter than minLength"));
        }
        if schema.get("maxLength").and_then(Value::as_u64).is_some_and(|m| n > m) {
            fail(format!("string of length {n} is longer than maxLength"));
        }
    }
    if let Some(items) = v.as_array() {
        if schema.get("maxItems").and_then(Value::as_u64).is_some_and(|m| items.len() as u64 > m) {
            fail(format!("{} items exceed maxItems", items.len()));
        }
        if let Some(sub) = schema.get("items") {
            for (i, item) in items.iter().enumerate() {
                walk(sub, item, &ptr(path, i), out);
            }
        }
    }
    if let Some(obj) = v.as_object() {
        for req in schema.get("required").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
            if !obj.contains_key(req) {
                out.push(SchemaError { path: ptr(path, req), message: format!("required property `{req}` is missing") });
            }
        }
        let props = schema.get("properties").and_then(Value::as_object);
        for (k, item) in obj {
            match (props.and_then(|p| p.get(k)), schema.get("additionalProperties")) {
                (Some(sub), _) => walk(sub, item, &ptr(path, k), out),
                (None, Some(Value::Bool(false))) => {
                    out.push(SchemaError { path: ptr(path, k), message: format!("property `{k}` is not allowed") })
                }
                (None, Some(sub @ Value::Object(_))) => walk(sub, item, &ptr(path, k), out),
                _ => {}
            }
        }
    }
}
