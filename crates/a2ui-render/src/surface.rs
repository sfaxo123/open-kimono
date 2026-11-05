//! The renderer's state for one A2UI surface: the component table, the data
//! model, and the client-side values typed into text inputs.
//!
//! Rules taken from `specification/v0_9_1/json/server_to_client.json`:
//! `createSurface` opens a surface (a second one for a live id is an error),
//! `updateComponents`/`updateDataModel` only apply to an open surface, and
//! `deleteSurface` closes it. Components are upserted by `id`, so a document
//! may define the tree in any order and patch it later — the renderer reads
//! the table at layout time, which is what makes progressive rendering work.

use std::collections::BTreeMap;

use serde_json::{Map, Value};

use crate::doc::Message;
use crate::pointer;

/// One surface's mutable state.
#[derive(Clone, Debug)]
pub struct Surface {
    /// `""` until `createSurface` opens it.
    pub id: String,
    pub catalog_id: String,
    /// The surface theme is read but not honoured: this renderer owns the
    /// black+orange palette (VISION §8), so a foreign `primaryColor` is
    /// recorded, not applied.
    pub theme: Option<Value>,
    /// id → the component object exactly as it arrived.
    pub components: BTreeMap<String, Value>,
    /// The surface data model (always an object at the root).
    pub data: Value,
    /// Two-way binding state for text inputs the user typed into
    /// (component id → current text), layered over the model's value.
    pub local_values: BTreeMap<String, String>,
    /// Non-fatal protocol complaints (unknown message, stale surface id, …).
    pub diagnostics: Vec<String>,
}

impl Default for Surface {
    fn default() -> Self {
        Surface {
            id: String::new(),
            catalog_id: String::new(),
            theme: None,
            components: BTreeMap::new(),
            data: Value::Object(Map::new()),
            local_values: BTreeMap::new(),
            diagnostics: Vec::new(),
        }
    }
}

impl Surface {
    pub fn is_open(&self) -> bool {
        !self.id.is_empty()
    }

    /// Apply one message. Protocol violations are returned, never panicked on;
    /// the caller decides whether to also record them as diagnostics.
    pub fn apply(&mut self, msg: &Message) -> Result<(), String> {
        match msg {
            Message::CreateSurface { surface_id, catalog_id, theme, .. } => {
                if self.is_open() {
                    return Err(format!(
                        "createSurface `{surface_id}`: surface `{}` is already open (delete it first)",
                        self.id
                    ));
                }
                self.id = surface_id.clone();
                self.catalog_id = catalog_id.clone();
                self.theme = theme.clone();
                self.components.clear();
                self.data = Value::Object(Map::new());
                self.local_values.clear();
                Ok(())
            }
            Message::UpdateComponents { surface_id, components } => {
                self.require_open(surface_id)?;
                for c in components {
                    let id = c.get("id").and_then(Value::as_str).ok_or("component without an `id`")?;
                    self.components.insert(id.to_string(), c.clone());
                }
                Ok(())
            }
            Message::UpdateDataModel { surface_id, path, value } => {
                self.require_open(surface_id)?;
                // `path` omitted or "/" refers to the whole model; a `value`
                // omitted removes what is at `path`.
                let p = path.as_deref();
                let whole = p.is_none_or(|q| q == "/" || q.is_empty());
                match (whole, value) {
                    (true, Some(v)) => {
                        self.data = v.clone();
                        Ok(())
                    }
                    (true, None) => {
                        self.data = Value::Object(Map::new());
                        Ok(())
                    }
                    (false, Some(v)) => pointer::set(&mut self.data, p.unwrap(), v.clone())
                        .map_err(|e| format!("updateDataModel: {e}")),
                    (false, None) => pointer::remove(&mut self.data, p.unwrap())
                        .map_err(|e| format!("updateDataModel: {e}")),
                }
            }
            Message::DeleteSurface { surface_id } => {
                self.require_open(surface_id)?;
                *self = Surface::default();
                Ok(())
            }
        }
    }

    fn require_open(&self, surface_id: &str) -> Result<(), String> {
        if !self.is_open() {
            return Err(format!("message for surface `{surface_id}` but no surface is open"));
        }
        if self.id != surface_id {
            return Err(format!(
                "message for surface `{surface_id}` but the open surface is `{}`",
                self.id
            ));
        }
        Ok(())
    }

    /// The component with `id`, if the document has defined it yet.
    pub fn component(&self, id: &str) -> Option<&Value> {
        self.components.get(id)
    }

    /// The tree's entry point. The spec requires a component with `id: "root"`.
    pub fn root(&self) -> Option<&Value> {
        self.components.get("root")
    }

    /// Feed a typed value into a text input (client-side two-way binding).
    /// If the field's `value` is bound to a model path, the model is updated
    /// too, so anything else bound to that path re-renders.
    pub fn set_field_value(&mut self, component_id: &str, value: &str) -> Option<String> {
        self.local_values.insert(component_id.to_string(), value.to_string());
        let path = self
            .component(component_id)
            .and_then(|c| c.get("value"))
            .and_then(|v| v.get("path"))
            .and_then(Value::as_str)
            .map(|p| p.to_string());
        if let Some(p) = &path {
            if pointer::set(&mut self.data, p, Value::String(value.to_string())).is_err() {
                self.diagnostics
                    .push(format!("text field `{component_id}`: cannot write model path `{p}`"));
            }
        }
        path
    }
}
