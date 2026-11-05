//! JSON Pointer (RFC 6901) access into an A2UI data model.
//!
//! A2UI binds component properties to the data model with JSON Pointer paths
//! (`common_types.json#/$defs/DataBinding`), and `formatString` interpolates
//! the same paths. Absolute paths start at the surface model; inside a list
//! template the spec also allows relative ones (`{"path": "title"}`), which
//! [`crate::value`] resolves against the current item before calling here.

use serde_json::{Map, Value};

/// Split a pointer into unescaped segments. `""` and `"/"` mean the root.
pub fn segments(path: &str) -> Vec<String> {
    let p = path.strip_prefix('/').unwrap_or(path);
    if p.is_empty() {
        return Vec::new();
    }
    p.split('/')
        .map(|seg| seg.replace("~1", "/").replace("~0", "~"))
        .collect()
}

/// Look a pointer up in `root`; `None` when any step is missing or the shape
/// does not match (a pointer through a string is simply not found).
pub fn lookup<'a>(root: &'a Value, path: &str) -> Option<&'a Value> {
    let mut cur = root;
    for seg in segments(path) {
        match cur {
            Value::Object(map) => cur = map.get(&seg)?,
            Value::Array(items) => {
                let idx: usize = seg.parse().ok()?;
                cur = items.get(idx)?;
            }
            _ => return None,
        }
    }
    Some(cur)
}

/// Replace (or create) the value at `path`. Missing object keys are created
/// as empty objects on the way down, which is what `updateDataModel` means by
/// "replaced (or created)". Writing through a non-container, or past the end
/// of an array, is an error rather than a silent mutation.
pub fn set(root: &mut Value, path: &str, value: Value) -> Result<(), String> {
    let segs = segments(path);
    if segs.is_empty() {
        *root = value;
        return Ok(());
    }
    let mut cur = root;
    for (i, seg) in segs.iter().enumerate() {
        let last = i + 1 == segs.len();
        match cur {
            Value::Object(map) => {
                if last {
                    map.insert(seg.clone(), value);
                    return Ok(());
                }
                cur = map.entry(seg.clone()).or_insert_with(|| Value::Object(Map::new()));
            }
            Value::Array(items) => {
                let idx: usize = seg.parse().map_err(|_| format!("`{seg}` is not an array index"))?;
                if last {
                    if idx == items.len() {
                        items.push(value);
                        return Ok(());
                    }
                    if idx > items.len() {
                        return Err(format!("index {idx} is past the end of a {}-item array", items.len()));
                    }
                    items[idx] = value;
                    return Ok(());
                }
                let next = items.get_mut(idx).ok_or(format!("index {idx} is past the end of the array"))?;
                cur = next;
            }
            _ => return Err(format!("cannot descend into a value at `{seg}`")),
        }
    }
    Ok(())
}

/// Remove the value at `path` (an `updateDataModel` with no `value`).
pub fn remove(root: &mut Value, path: &str) -> Result<(), String> {
    let segs = segments(path);
    if segs.is_empty() {
        *root = Value::Object(Map::new());
        return Ok(());
    }
    let mut cur = root;
    for (i, seg) in segs.iter().enumerate() {
        let last = i + 1 == segs.len();
        match cur {
            Value::Object(map) => {
                if last {
                    return match map.remove(seg) {
                        Some(_) => Ok(()),
                        None => Err(format!("no key `{seg}` to remove")),
                    };
                }
                cur = map.get_mut(seg).ok_or(format!("no key `{seg}`"))?;
            }
            Value::Array(items) => {
                let idx: usize = seg.parse().map_err(|_| format!("`{seg}` is not an array index"))?;
                if idx >= items.len() {
                    return Err(format!("index {idx} is past the end of the array"));
                }
                if last {
                    items.remove(idx);
                    return Ok(());
                }
                cur = &mut items[idx];
            }
            _ => return Err(format!("cannot descend into a value at `{seg}`")),
        }
    }
    Ok(())
}
