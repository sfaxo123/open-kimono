//! Virtual A2UI / OpenUI surfaces (SERVICE-IR.md §5.1 "SurfaceSnapshot").
//! A surface exists only once its service first patches it (minimal: no UI
//! until there is something to show); every change is journaled as the A2UI
//! message a renderer would receive.

use serde::Serialize;
use serde_json::{json, Map, Value};

use super::Svcd;
use crate::host::ActionHost;
use crate::ir::{UiMode, UiOperation, UiPatch};
use crate::patch;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Surface {
    #[serde(rename = "surfaceId")]
    pub surface_id: String,
    pub owner: String,
    #[serde(rename = "catalogId", skip_serializing_if = "Option::is_none")]
    pub catalog_id: Option<String>,
    pub components: Vec<Value>,
    #[serde(rename = "dataModel")]
    pub data_model: Value,
    #[serde(rename = "openuiStatements")]
    pub openui_statements: Vec<String>,
    pub revision: u64,
}

/// `$name = ...` statements upsert by their left-hand side.
fn upsert_statements(list: &mut Vec<String>, new: &[String]) {
    for s in new {
        let lhs = s.split('=').next().unwrap_or("").trim().to_string();
        match list.iter_mut().find(|e| e.split('=').next().unwrap_or("").trim() == lhs) {
            Some(slot) => *slot = s.clone(),
            None => list.push(s.clone()),
        }
    }
}

fn upsert_components(list: &mut Vec<Value>, new: &[Value]) {
    for c in new {
        match list.iter_mut().find(|e| e.get("id").is_some() && e.get("id") == c.get("id")) {
            Some(slot) => *slot = c.clone(),
            None => list.push(c.clone()),
        }
    }
}

impl<H: ActionHost> Svcd<H> {
    fn ensure_surface(&mut self, sid: &str, surface_id: &str) {
        if self.surfaces.contains_key(surface_id) {
            return;
        }
        let ir = self.services[sid].ir.clone();
        let mut s = Surface {
            surface_id: surface_id.to_string(),
            owner: sid.to_string(),
            catalog_id: ir.ui.catalog_id.clone(),
            components: vec![],
            data_model: json!({}),
            openui_statements: vec![],
            revision: 0,
        };
        if ir.ui.surface_id.as_deref() == Some(surface_id) {
            for m in &ir.ui.initial_messages {
                apply_message(&mut s, m);
            }
            if ir.ui.mode == UiMode::Openui {
                upsert_statements(&mut s.openui_statements, &ir.ui.openui_statements);
            }
        }
        let msg = json!({ "version": "v1.0", "createSurface": { "surfaceId": surface_id, "catalogId": s.catalog_id } });
        self.journal.custom(self.now_ms, "a2ui", msg);
        self.surfaces.insert(surface_id.to_string(), s);
    }

    pub(crate) fn apply_ui(&mut self, sid: &str, p: &UiPatch, value: Option<Value>) {
        if p.operation == UiOperation::DeleteSurface {
            if self.surfaces.remove(&p.surface_id).is_some() {
                self.journal.custom(self.now_ms, "a2ui", json!({ "version": "v1.0", "deleteSurface": { "surfaceId": p.surface_id } }));
            }
            return;
        }
        self.ensure_surface(sid, &p.surface_id);
        let s = self.surfaces.get_mut(&p.surface_id).unwrap();
        let path = p.data_model_path.clone().unwrap_or_else(|| "/".into());
        let msg = match p.operation {
            UiOperation::CreateSurface => None,
            UiOperation::UpdateComponents => {
                let comps = value.as_ref().and_then(Value::as_array).cloned().unwrap_or_default();
                upsert_components(&mut s.components, &comps);
                Some(json!({ "updateComponents": { "surfaceId": p.surface_id, "components": comps } }))
            }
            UiOperation::UpdateDataModel => {
                let v = value.unwrap_or(Value::Null);
                if let Err(e) = patch::set(&mut s.data_model, &path, v.clone()) {
                    let why = json!({ "surfaceId": p.surface_id, "error": e });
                    self.journal.custom(self.now_ms, "svc.ui.error", why);
                    return;
                }
                Some(json!({ "updateDataModel": { "surfaceId": p.surface_id, "path": path, "value": v } }))
            }
            UiOperation::OpenuiStatements => {
                let stmts = p.openui_statements.clone().unwrap_or_default();
                upsert_statements(&mut s.openui_statements, &stmts);
                Some(json!({ "openuiStatements": { "surfaceId": p.surface_id, "statements": stmts } }))
            }
            UiOperation::DeleteSurface => unreachable!(),
        };
        s.revision += 1;
        if let Some(mut m) = msg {
            m["version"] = json!("v1.0");
            self.journal.custom(self.now_ms, "a2ui", m);
        }
    }

    /// Remove every surface a service owns (when it stops).
    pub(crate) fn drop_surfaces(&mut self, sid: &str) {
        let owned: Vec<String> = self.surfaces.values().filter(|s| s.owner == sid).map(|s| s.surface_id.clone()).collect();
        for id in owned {
            self.surfaces.remove(&id);
            self.journal.custom(self.now_ms, "a2ui", json!({ "version": "v1.0", "deleteSurface": { "surfaceId": id } }));
        }
    }

    pub(crate) fn has_surface(&self, sid: &str) -> bool {
        self.surfaces.values().any(|s| s.owner == sid)
    }
}

/// Apply one declarative A2UI message from `ui.initial_messages`.
fn apply_message(s: &mut Surface, m: &Map<String, Value>) {
    if let Some(c) = m.get("createSurface") {
        if let Some(cat) = c.get("catalogId").and_then(Value::as_str) {
            s.catalog_id = Some(cat.to_string());
        }
    }
    if let Some(comps) = m.get("updateComponents").and_then(|u| u.get("components")).and_then(Value::as_array) {
        upsert_components(&mut s.components, comps);
    }
    if let Some(u) = m.get("updateDataModel") {
        let path = u.get("path").and_then(Value::as_str).unwrap_or("/");
        let _ = patch::set(&mut s.data_model, path, u.get("value").cloned().unwrap_or(Value::Null));
    }
}
