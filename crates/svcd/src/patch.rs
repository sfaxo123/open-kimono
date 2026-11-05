//! RFC 6902 JSON Patch subset (`add`, `replace`, `remove`, `test`) applied
//! to the copy-on-write state overlay, plus `set` for data-model writes.

use serde_json::Value;

use crate::expr::json_eq;
use crate::ir::PatchOp;

fn unescape(seg: &str) -> String {
    seg.replace("~1", "/").replace("~0", "~")
}

fn split(path: &str) -> Result<(Vec<String>, String), String> {
    let rest = path.strip_prefix('/').ok_or_else(|| format!("`{path}` is not a JSON Pointer"))?;
    let mut segs: Vec<String> = rest.split('/').map(unescape).collect();
    let last = segs.pop().unwrap_or_default();
    Ok((segs, last))
}

fn parent_mut<'a>(root: &'a mut Value, segs: &[String], path: &str) -> Result<&'a mut Value, String> {
    let mut cur = root;
    for s in segs {
        cur = match cur {
            Value::Object(m) => m.get_mut(s),
            Value::Array(a) => s.parse::<usize>().ok().and_then(|i| a.get_mut(i)),
            _ => None,
        }
        .ok_or_else(|| format!("path `{path}`: `{s}` does not exist"))?;
    }
    Ok(cur)
}

pub fn apply(root: &mut Value, op: PatchOp, path: &str, value: Option<Value>) -> Result<(), String> {
    let (segs, last) = split(path)?;
    let parent = parent_mut(root, &segs, path)?;
    let need = |v: Option<Value>| v.ok_or_else(|| format!("`{}` on `{path}` needs a value", op.as_str()));
    match (parent, op) {
        (Value::Object(m), PatchOp::Add) => {
            m.insert(last, need(value)?);
        }
        (Value::Object(m), PatchOp::Replace) => match m.get_mut(&last) {
            Some(slot) => *slot = need(value)?,
            None => return Err(format!("replace: `{path}` does not exist")),
        },
        (Value::Object(m), PatchOp::Remove) => {
            m.remove(&last).ok_or_else(|| format!("remove: `{path}` does not exist"))?;
        }
        (Value::Object(m), PatchOp::Test) => {
            let want = need(value)?;
            if !m.get(&last).is_some_and(|v| json_eq(v, &want)) {
                return Err(format!("test failed at `{path}`: expected {want}"));
            }
        }
        (Value::Array(a), op) => {
            let idx = if last == "-" { a.len() } else { last.parse::<usize>().map_err(|_| format!("`{last}` is not an array index"))? };
            match op {
                PatchOp::Add if idx <= a.len() => a.insert(idx, need(value)?),
                PatchOp::Replace if idx < a.len() => a[idx] = need(value)?,
                PatchOp::Remove if idx < a.len() => {
                    a.remove(idx);
                }
                PatchOp::Test if idx < a.len() && json_eq(&a[idx], &need(value.clone())?) => {}
                _ => return Err(format!("{}: index {idx} out of range at `{path}`", op.as_str())),
            }
        }
        _ => return Err(format!("path `{path}`: parent is not a container")),
    }
    Ok(())
}

/// Write `value` at `path`, creating intermediate objects; `/` replaces the root.
pub fn set(root: &mut Value, path: &str, value: Value) -> Result<(), String> {
    if path == "/" || path.is_empty() {
        *root = value;
        return Ok(());
    }
    let (segs, last) = split(path)?;
    let mut cur = root;
    for s in segs {
        if !cur.is_object() {
            *cur = Value::Object(Default::default());
        }
        cur = cur.as_object_mut().unwrap().entry(s).or_insert_with(|| Value::Object(Default::default()));
    }
    match cur {
        Value::Object(m) => {
            m.insert(last, value);
            Ok(())
        }
        _ => Err(format!("path `{path}`: parent is not an object")),
    }
}
