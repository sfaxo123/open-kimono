//! Minimal JSON support, zero-dependency: a pretty-printer matching
//! CPython `json.dumps(..., indent=2)` for the manifest's value shapes
//! (strings, integers, booleans, string arrays, one nested object), and
//! a tolerant string-field extractor for reading a prior manifest's
//! payload receipt (the freshness guard).

pub enum Json {
    Str(String),
    Int(i64),
    Bool(bool),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c if (c as u32) > 0x7e => {
                // CPython ensure_ascii=True (the json.dumps default)
                let mut buf = [0u16; 2];
                for u in c.encode_utf16(&mut buf) {
                    out.push_str(&format!("\\u{:04x}", u));
                }
            }
            c => out.push(c),
        }
    }
    out
}

fn write_json(v: &Json, indent: usize, out: &mut String) {
    match v {
        Json::Str(s) => {
            out.push('"');
            out.push_str(&escape(s));
            out.push('"');
        }
        Json::Int(n) => out.push_str(&n.to_string()),
        Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Json::Arr(items) => {
            if items.is_empty() {
                out.push_str("[]");
                return;
            }
            out.push_str("[\n");
            for (i, item) in items.iter().enumerate() {
                out.push_str(&" ".repeat(indent + 2));
                write_json(item, indent + 2, out);
                if i + 1 < items.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            out.push_str(&" ".repeat(indent));
            out.push(']');
        }
        Json::Obj(pairs) => {
            if pairs.is_empty() {
                out.push_str("{}");
                return;
            }
            out.push_str("{\n");
            for (i, (k, val)) in pairs.iter().enumerate() {
                out.push_str(&" ".repeat(indent + 2));
                out.push('"');
                out.push_str(&escape(k));
                out.push_str("\": ");
                write_json(val, indent + 2, out);
                if i + 1 < pairs.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            out.push_str(&" ".repeat(indent));
            out.push('}');
        }
    }
}

/// Serialize like `json.dumps(value, indent=2)` (no trailing newline;
/// callers add the file's trailing "\n" exactly as the Python did).
pub fn dumps_indent2(v: &Json) -> String {
    let mut s = String::new();
    write_json(v, 0, &mut s);
    s
}

/// Extract a top-level string field's value from a JSON document.
/// Tolerant scanner (handles whitespace + standard string escapes);
/// returns None when the key or a string value is absent.
pub fn extract_string_field(json: &str, key: &str) -> Option<String> {
    let needle = format!("\"{}\"", key);
    let start = json.find(&needle)? + needle.len();
    let rest = json[start..].trim_start();
    let rest = rest.strip_prefix(':')?.trim_start();
    let rest = rest.strip_prefix('"')?;
    let mut out = String::new();
    let mut chars = rest.chars();
    loop {
        match chars.next()? {
            '"' => return Some(out),
            '\\' => match chars.next()? {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                'r' => out.push('\r'),
                '"' => out.push('"'),
                '\\' => out.push('\\'),
                '/' => out.push('/'),
                'u' => {
                    let h: String = chars.by_ref().take(4).collect();
                    let n = u32::from_str_radix(&h, 16).ok()?;
                    out.push(char::from_u32(n)?);
                }
                _ => return None,
            },
            c => out.push(c),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dumps_matches_python_shape() {
        let v = Json::Obj(vec![
            ("a".into(), Json::Str("x".into())),
            ("n".into(), Json::Int(42)),
            ("b".into(), Json::Bool(false)),
            (
                "range".into(),
                Json::Arr(vec![Json::Str("0x0".into()), Json::Str("0x10".into())]),
            ),
        ]);
        assert_eq!(
            dumps_indent2(&v),
            "{\n  \"a\": \"x\",\n  \"n\": 42,\n  \"b\": false,\n  \"range\": [\n    \"0x0\",\n    \"0x10\"\n  ]\n}"
        );
    }

    #[test]
    fn extract_receipt_fields() {
        let doc = "{\n  \"payload_kind\": \"redox-v4-reset\",\n  \"payload_sha256\": \"ab12\"\n}\n";
        assert_eq!(
            extract_string_field(doc, "payload_kind").as_deref(),
            Some("redox-v4-reset")
        );
        assert_eq!(
            extract_string_field(doc, "payload_sha256").as_deref(),
            Some("ab12")
        );
        assert_eq!(extract_string_field(doc, "missing"), None);
        assert_eq!(extract_string_field("{\"n\": 5}", "n"), None);
    }
}
