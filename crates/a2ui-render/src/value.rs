//! Dynamic values — the `common_types.json` half of A2UI.
//!
//! Every interesting property in the format (`text`, `value`, `label`, a
//! progress `value`, …) is a *dynamic* value, and the spec gives it exactly
//! three shapes:
//!
//! - a literal (`"hello"`, `42`, `true`, or `{"literalString": "…"}`),
//! - a data binding — `{"path": "/city"}`, a JSON Pointer into the surface
//!   data model; inside a list template a path without a leading `/` is
//!   relative to the current item,
//! - a function call — `{"call": "formatNumber", "args": {…}, "returnType": "string"}`.
//!
//! A missing path is **not** an error: the spec expects progressive
//! rendering, so a component whose data has not arrived yet renders as an
//! empty string and the caller decides what placeholder to show. Unknown
//! functions are named in the diagnostics and resolve to nothing — a
//! generated document that calls `quantumSpin()` shows up in the log rather
//! than silently rendering nonsense.
//!
//! Implemented catalog functions (from `catalogs/basic/catalog.json`):
//! `formatString`, `formatNumber`, `pluralize`, `and`, `or`, `not`,
//! `required`, `openUrl` (recorded — this client has no browser).

use serde_json::{Map, Value};

use crate::pointer;

/// How deep an expression may nest before we call it a bug in the document.
const MAX_DEPTH: u32 = 24;

/// What a property can see: the surface model, plus the current item when the
/// component is an instance of a list template.
#[derive(Clone, Copy, Debug)]
pub struct Ctx<'a> {
    pub data: &'a Value,
    pub item: Option<&'a Value>,
}

impl<'a> Ctx<'a> {
    /// The context a list-template instance evaluates in.
    pub fn with_item(&self, item: &'a Value) -> Ctx<'a> {
        Ctx { data: self.data, item: Some(item) }
    }
}

/// Resolve any dynamic value to a plain JSON value (`Null` when absent).
pub fn resolve(value: &Value, ctx: &Ctx, diag: &mut Vec<String>) -> Value {
    resolve_depth(value, ctx, diag, 0)
}

fn resolve_depth(value: &Value, ctx: &Ctx, diag: &mut Vec<String>, depth: u32) -> Value {
    if depth > MAX_DEPTH {
        diag.push("expression nests too deeply (malformed document)".into());
        return Value::Null;
    }
    let obj = match value {
        Value::Object(obj) => obj,
        // A literal string/number/bool passes through.
        other => return other.clone(),
    };

    // Literals spelled out (the spec allows both forms).
    for key in ["literalString", "literalNumber", "literalBoolean", "literal"] {
        if let Some(v) = obj.get(key) {
            return v.clone();
        }
    }

    if let Some(p) = obj.get("path").and_then(Value::as_str) {
        return bind(p, ctx).cloned().unwrap_or(Value::Null);
    }

    if let Some(name) = obj.get("call").and_then(Value::as_str) {
        return call(name, obj.get("args"), ctx, diag, depth);
    }

    // Neither a literal, a binding, nor a call: hand it back untouched and
    // say so — the caller renders something visible either way.
    diag.push(format!("property is not a literal/binding/call (object with {} keys)", obj.len()));
    value.clone()
}

/// Resolve a *container* of dynamic values — an action's `context` object, for
/// instance — by resolving each member, while an object that is itself a
/// binding/call/literal resolves as one unit.
pub fn resolve_tree(value: &Value, ctx: &Ctx, diag: &mut Vec<String>) -> Value {
    match value {
        Value::Object(obj) => {
            let is_dynamic = obj.contains_key("path")
                || obj.contains_key("call")
                || ["literalString", "literalNumber", "literalBoolean", "literal"]
                    .iter()
                    .any(|k| obj.contains_key(*k));
            if is_dynamic {
                resolve_depth(value, ctx, diag, 0)
            } else {
                Value::Object(obj.iter().map(|(k, v)| (k.clone(), resolve_tree(v, ctx, diag))).collect())
            }
        }
        Value::Array(items) => Value::Array(items.iter().map(|v| resolve_tree(v, ctx, diag)).collect()),
        other => resolve_depth(other, ctx, diag, 0),
    }
}

/// A `{"path": …}` binding. Absolute paths hit the model; relative ones try
/// the current template item first, then the model root.
pub fn bind<'a>(path: &str, ctx: &Ctx<'a>) -> Option<&'a Value> {
    if path.starts_with('/') {
        return pointer::lookup(ctx.data, path);
    }
    if let Some(item) = ctx.item {
        if let Some(v) = pointer::lookup(item, path) {
            return Some(v);
        }
    }
    pointer::lookup(ctx.data, path)
}

/// Resolve and render as text (the `DynamicString` case).
pub fn text(value: &Value, ctx: &Ctx, diag: &mut Vec<String>) -> String {
    stringify(&resolve(value, ctx, diag))
}

/// Resolve and read as a number (the `DynamicNumber` case). Numbers stored as
/// strings are accepted — agents do that — anything else is `None`.
pub fn number(value: &Value, ctx: &Ctx, diag: &mut Vec<String>) -> Option<f64> {
    match resolve(value, ctx, diag) {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse::<f64>().ok(),
        Value::Bool(b) => Some(if b { 1.0 } else { 0.0 }),
        _ => None,
    }
}

/// Resolve and read as a boolean (the `DynamicBoolean` case).
pub fn flag(value: &Value, ctx: &Ctx, diag: &mut Vec<String>) -> bool {
    truthy(&resolve(value, ctx, diag))
}

/// Truthiness of an already-resolved value.
fn truthy(v: &Value) -> bool {
    match v {
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().map(|f| f != 0.0).unwrap_or(false),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Null | Value::Object(_) => false,
    }
}

/// JSON value → display text. Missing is the empty string, so a component
/// waiting on data renders blank rather than "null".
pub fn stringify(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => match n.as_f64() {
            Some(f) if f.fract() == 0.0 && f.abs() < 1e15 => format!("{}", f as i64),
            Some(f) => format!("{f}"),
            None => n.to_string(),
        },
        other => other.to_string(),
    }
}

/// One argument, resolved.
fn arg_of(args: Option<&Value>, key: &str, ctx: &Ctx, diag: &mut Vec<String>, depth: u32) -> Value {
    let slot = args.and_then(Value::as_object).and_then(|m| m.get(key));
    match slot {
        Some(v) => resolve_depth(v, ctx, diag, depth + 1),
        None => Value::Null,
    }
}

fn call(name: &str, args: Option<&Value>, ctx: &Ctx, diag: &mut Vec<String>, depth: u32) -> Value {
    match name {
        "formatString" => {
            let tmpl = arg_of(args, "value", ctx, diag, depth);
            Value::String(interpolate(&stringify(&tmpl), ctx, diag, depth))
        }
        "formatNumber" => {
            let v = arg_of(args, "value", ctx, diag, depth);
            let Some(f) = v.as_f64().or_else(|| v.as_str().and_then(|s| s.trim().parse::<f64>().ok())) else {
                if v != Value::Null {
                    diag.push(format!("formatNumber: `{}` is not a number", stringify(&v)));
                }
                return Value::String(String::new());
            };
            let decimals = arg_of(args, "decimals", ctx, diag, depth).as_f64().map(|d| d.clamp(0.0, 12.0) as usize);
            let grouping = args.and_then(Value::as_object).and_then(|m| m.get("grouping")).and_then(Value::as_bool).unwrap_or(true);
            Value::String(format_number(f, decimals, grouping))
        }
        "pluralize" => {
            let n = arg_of(args, "value", ctx, diag, depth).as_f64().unwrap_or(0.0);
            // English plural categories: `one` for 1, everything else `other`.
            let chosen = arg_of(args, if n == 1.0 { "one" } else { "other" }, ctx, diag, depth);
            if chosen == Value::Null {
                arg_of(args, "other", ctx, diag, depth)
            } else {
                chosen
            }
        }
        "and" | "or" => {
            let list = args.and_then(Value::as_object).and_then(|m| m.get("values")).cloned().unwrap_or(Value::Null);
            let Some(items) = list.as_array() else {
                diag.push(format!("`{name}` needs a `values` array"));
                return Value::Bool(false);
            };
            let all = items.iter().map(|v| flag(v, ctx, diag)).collect::<Vec<bool>>();
            Value::Bool(if name == "and" { !all.is_empty() && all.iter().all(|b| *b) } else { all.iter().any(|b| *b) })
        }
        "not" => {
            let v = arg_of(args, "value", ctx, diag, depth);
            Value::Bool(!truthy(&v))
        }
        "required" => {
            let v = arg_of(args, "value", ctx, diag, depth);
            Value::Bool(match &v {
                Value::Null => false,
                Value::String(s) => !s.trim().is_empty(),
                Value::Array(a) => !a.is_empty(),
                _ => true,
            })
        }
        "openUrl" => {
            // No browser on the phone: say so instead of a silent no-op.
            let url = arg_of(args, "url", ctx, diag, depth);
            diag.push(format!("openUrl `{}` ignored: this client has no browser", stringify(&url)));
            Value::Null
        }
        other => {
            diag.push(format!("unknown function `{other}`"));
            Value::Null
        }
    }
}

/// `${…}` interpolation: data paths, nested function calls, `\${` escapes.
pub fn interpolate(template: &str, ctx: &Ctx, diag: &mut Vec<String>, depth: u32) -> String {
    let mut out = String::new();
    let mut i = 0;
    while i < template.len() {
        let rest = &template[i..];
        if rest.starts_with("\\${") {
            out.push_str("${");
            i += 3;
            continue;
        }
        if rest.starts_with("${") {
            match find_close(template, i + 1) {
                Some(end) => {
                    out.push_str(&eval_expr(&template[i + 2..end], ctx, diag, depth));
                    i = end + 1;
                }
                None => {
                    diag.push("unterminated `${` in a formatString".into());
                    out.push_str(rest);
                    break;
                }
            }
            continue;
        }
        let len = rest.chars().next().map(char::len_utf8).unwrap_or(1);
        out.push_str(&rest[..len]);
        i += len;
    }
    out
}

/// Index of the `}` closing the `${` whose `{` sits at `open`. Nested `${`
/// and quoted sections are counted so `${a(x: ${/b})}` closes correctly.
fn find_close(s: &str, open: usize) -> Option<usize> {
    let b = s.as_bytes();
    let mut depth = 0usize;
    let mut quote: Option<u8> = None;
    let mut i = open;
    while i < b.len() {
        let c = b[i];
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if c == b'\'' || c == b'"' => quote = Some(c),
            None if c == b'{' => depth += 1,
            None if c == b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            None => {}
        }
        i += 1;
    }
    None
}

/// One interpolated expression: a call, a path, or nothing sensible.
fn eval_expr(expr: &str, ctx: &Ctx, diag: &mut Vec<String>, depth: u32) -> String {
    if depth > MAX_DEPTH {
        diag.push("interpolation nests too deeply".into());
        return String::new();
    }
    let e = expr.trim();
    if e.is_empty() {
        diag.push("empty `${}` expression".into());
        return String::new();
    }
    if let Some(open) = e.find('(') {
        if e.ends_with(')') {
            let name = e[..open].trim();
            if is_ident(name) {
                let inner = &e[open + 1..e.len() - 1];
                let args = parse_args(inner, ctx, diag, depth);
                return stringify(&call(name, Some(&args), ctx, diag, depth + 1));
            }
        }
    }
    match bind(e, ctx) {
        Some(v) => stringify(v),
        None => {
            diag.push(format!("binding `${e}` not found in the data model"));
            String::new()
        }
    }
}

fn is_ident(s: &str) -> bool {
    !s.is_empty()
        && s.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_')
        && s.chars().all(|c| c.is_alphanumeric() || c == '_')
}

/// `name:value, name:value` → an object the [`call`] evaluator can read.
fn parse_args(inner: &str, ctx: &Ctx, diag: &mut Vec<String>, depth: u32) -> Value {
    let mut map = Map::new();
    for part in split_args(inner) {
        let part = part.trim().to_string();
        if part.is_empty() {
            continue;
        }
        match part.split_once(':') {
            Some((name, raw)) => {
                map.insert(name.trim().to_string(), arg_literal(raw.trim(), ctx, diag, depth));
            }
            None => diag.push(format!("argument `{part}` is not named (`name:value`)")),
        }
    }
    Value::Object(map)
}

/// Split on top-level commas, ignoring commas inside quotes and `()`/`${}`.
fn split_args(inner: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut cur = String::new();
    let mut depth = 0usize;
    let mut quote: Option<char> = None;
    for c in inner.chars() {
        match quote {
            Some(q) => {
                cur.push(c);
                if c == q {
                    quote = None;
                }
            }
            None => match c {
                '\'' | '"' => {
                    quote = Some(c);
                    cur.push(c);
                }
                '(' | '{' => {
                    depth += 1;
                    cur.push(c);
                }
                ')' | '}' => {
                    depth = depth.saturating_sub(1);
                    cur.push(c);
                }
                ',' if depth == 0 => parts.push(std::mem::take(&mut cur)),
                _ => cur.push(c),
            },
        }
    }
    if !cur.trim().is_empty() {
        parts.push(cur);
    }
    parts
}

/// An argument's raw text inside `${…}`: quoted string, number, bool, nested
/// `${…}`, or a data path.
fn arg_literal(raw: &str, ctx: &Ctx, diag: &mut Vec<String>, depth: u32) -> Value {
    let r = raw.trim();
    if r.len() >= 2 && (r.starts_with('\'') || r.starts_with('"')) && r.ends_with(&r[..1]) {
        return Value::String(r[1..r.len() - 1].to_string());
    }
    if r == "true" || r == "false" {
        return Value::Bool(r == "true");
    }
    if let Ok(n) = r.parse::<f64>() {
        return Value::from(n);
    }
    if let Some(inner) = r.strip_prefix("${").and_then(|s| s.strip_suffix('}')) {
        return Value::String(eval_expr(inner.trim(), ctx, diag, depth + 1));
    }
    match bind(r, ctx) {
        Some(v) => v.clone(),
        None => {
            diag.push(format!("argument path `{r}` not found in the data model"));
            Value::Null
        }
    }
}

/// Grouped decimal formatting. With `decimals` omitted, whole numbers print
/// whole and everything else gets two places — the spec leaves the default to
/// the locale; this client is deterministic instead.
pub fn format_number(f: f64, decimals: Option<usize>, grouping: bool) -> String {
    if !f.is_finite() {
        return String::new();
    }
    let places = decimals.unwrap_or(if f.fract() == 0.0 { 0 } else { 2 });
    let neg = f < 0.0;
    // Round half away from zero: `{:.n}` rounds half to even, which makes
    // 1234.5 print as 1234 and reads as a bug on a progress readout.
    let scale = 10f64.powi(places as i32);
    let s = format!("{:.places$}", (f.abs() * scale).round() / scale, places = places);
    let (int, frac) = s.split_once('.').map_or((s.as_str(), ""), |(a, b)| (a, b));
    let digits = if grouping {
        let mut grouped = String::new();
        for (i, c) in int.chars().enumerate() {
            if i > 0 && (int.len() - i) % 3 == 0 {
                grouped.push(',');
            }
            grouped.push(c);
        }
        grouped
    } else {
        int.to_string()
    };
    let body = if frac.is_empty() { digits } else { format!("{digits}.{frac}") };
    if neg {
        format!("-{body}")
    } else {
        body
    }
}
