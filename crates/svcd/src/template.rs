//! Value templates used throughout a ServiceIR:
//!
//! - `"{{ params.hours * 3600 }}"` — a string that is exactly one `{{ }}`
//!   block evaluates to a **typed** value (number stays a number).
//! - `"Time is up for {{state.eventName}}."` — mixed text interpolates.
//! - `"$.state"`, `"$.trigger.payload.capacity_pct"` — a whole-string
//!   JSONPath-style reference evaluates to the typed value at that path.
//!
//! Anything else is a literal. Rendering is strict: an undefined name fails.

use serde_json::Value;

use crate::expr::{self, Expr, ExprError, Scope};

#[derive(Clone, Debug, PartialEq)]
pub enum Segment {
    Text(String),
    Expr(Expr),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Template {
    Literal(String),
    /// A whole-string `{{expr}}` or `$.path` — renders to a typed value.
    Typed(Expr),
    Mixed(Vec<Segment>),
}

pub fn parse(src: &str) -> Result<Template, ExprError> {
    if let Some(path) = src.strip_prefix("$.") {
        let segs: Vec<String> = path.split('.').map(str::to_string).collect();
        if segs.iter().any(|s| s.is_empty() || !s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')) {
            return Err(ExprError(format!("bad `$.` reference `{src}`")));
        }
        return Ok(Template::Typed(Expr::Path(segs)));
    }
    let mut segments = Vec::new();
    let mut rest = src;
    while let Some(start) = rest.find("{{") {
        if start > 0 {
            segments.push(Segment::Text(rest[..start].to_string()));
        }
        let after = &rest[start + 2..];
        let end = after.find("}}").ok_or_else(|| ExprError(format!("unclosed `{{{{` in `{src}`")))?;
        segments.push(Segment::Expr(expr::parse(&after[..end])?));
        rest = &after[end + 2..];
    }
    if rest.contains("}}") {
        return Err(ExprError(format!("stray `}}}}` in `{src}`")));
    }
    if !rest.is_empty() {
        segments.push(Segment::Text(rest.to_string()));
    }
    Ok(match segments.as_slice() {
        [] => Template::Literal(String::new()),
        [Segment::Expr(e)] => Template::Typed(e.clone()),
        [Segment::Text(t)] => Template::Literal(t.clone()),
        _ => Template::Mixed(segments),
    })
}

impl Template {
    pub fn render(&self, scope: &dyn Scope) -> Result<Value, ExprError> {
        match self {
            Template::Literal(s) => Ok(Value::String(s.clone())),
            Template::Typed(e) => e.eval(scope),
            Template::Mixed(segs) => {
                let mut out = String::new();
                for seg in segs {
                    match seg {
                        Segment::Text(t) => out.push_str(t),
                        Segment::Expr(e) => out.push_str(&display(&e.eval(scope)?)),
                    }
                }
                Ok(Value::String(out))
            }
        }
    }

    pub fn paths(&self, out: &mut Vec<Vec<String>>) {
        match self {
            Template::Literal(_) => {}
            Template::Typed(e) => e.paths(out),
            Template::Mixed(segs) => segs.iter().for_each(|s| {
                if let Segment::Expr(e) = s {
                    e.paths(out)
                }
            }),
        }
    }
}

/// Human text for a value: integral numbers print without `.0`
/// ("Battery is down to 19 percent", not "19.0").
pub fn display(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => match (n.as_i64(), n.as_f64()) {
            (Some(i), _) => i.to_string(),
            (None, Some(f)) if f.fract() == 0.0 && f.abs() < 9.0e15 => (f as i64).to_string(),
            _ => n.to_string(),
        },
        other => other.to_string(),
    }
}

/// Render a string as a text template (always a string result).
pub fn render_text(src: &str, scope: &dyn Scope) -> Result<String, ExprError> {
    Ok(display(&parse(src)?.render(scope)?))
}

/// Recursively render every string inside a JSON value as a template.
pub fn render_value(v: &Value, scope: &dyn Scope) -> Result<Value, ExprError> {
    Ok(match v {
        Value::String(s) => parse(s)?.render(scope)?,
        Value::Array(items) => Value::Array(items.iter().map(|i| render_value(i, scope)).collect::<Result<_, _>>()?),
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (k, item) in map {
                out.insert(k.clone(), render_value(item, scope)?);
            }
            Value::Object(out)
        }
        other => other.clone(),
    })
}

/// Every path read by templates anywhere inside `v` (static validation).
pub fn value_paths(v: &Value, out: &mut Vec<Vec<String>>) -> Result<(), ExprError> {
    match v {
        Value::String(s) => parse(s)?.paths(out),
        Value::Array(items) => items.iter().try_for_each(|i| value_paths(i, out))?,
        Value::Object(map) => map.values().try_for_each(|i| value_paths(i, out))?,
        _ => {}
    }
    Ok(())
}
