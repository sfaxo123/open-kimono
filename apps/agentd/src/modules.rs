//! Agent-emitted module manifests (MODULE-PROTOCOL v1).
//!
//! agentd builds declarative manifests from real state — a successful
//! `app_launch` tool result, the live `SessionStore` — and journals
//! them as `module_manifest` CUSTOM events. kimono renders them only
//! from the journal (the CV1 follow_up_window pattern): no manifest
//! reaches the page except through a journal record agentd emitted.
//!
//! A manifest is data, never code. Every builder here sets a non-empty
//! `user_purpose`, uses only the closed block vocabulary and real
//! tool names, and is checked by `validate_manifest` — a Rust replica
//! of the exact rules `apps/kimono-ui/web/modules.mjs` enforces —
//! before it is journaled. The node suite also validates captured
//! emitted manifests with the real JS validator (see
//! `apps/kimono-ui/test/modules_test.mjs`); the replica here is the
//! emission-side guard, not a substitute for that proof.

use serde_json::{json, Value};

pub const DESTINATION_CARD_ID: &str = "destination-card";
pub const SESSION_CARD_ID: &str = "session-card";

const BLOCK_TYPES: [&str; 4] = ["title", "big_number", "rows", "action_button"];
const TOOL_NAMES: [&str; 9] = [
    "phone_clock", "phone_status", "power_status", "power_plan",
    "service_create", "service_describe", "service_list", "service_stop",
    "app_launch",
];
const INPUT_TYPES: [&str; 5] = ["string", "number", "minutes", "time", "boolean"];
const INPUT_SOURCES: [&str; 3] = ["agent", "context", "tool"];

fn is_non_empty_str(v: Option<&Value>) -> bool {
    matches!(v, Some(Value::String(s)) if !s.trim().is_empty())
}

/// Build the destination card from an actual `app_launch` result.
///
/// `destination`/`app` come from the tool call arguments, `uri` from
/// the tool result content (`launch.uri`), and `run_id` links the card
/// back to the run that produced it. Returns `(manifest, inputs)` —
/// the manifest is the protocol-v1 definition, `inputs` the
/// agent-filled values kimono displays (production values ride beside
/// the manifest, never in `sample_inputs`).
pub fn destination_card(destination: &str, app: &str, uri: &str, run_id: &str) -> (Value, Value) {
    let manifest = json!({
        "id": DESTINATION_CARD_ID,
        "protocol": "1",
        "user_purpose": "Show the user the destination just opened so they can confirm it and reopen it in one tap.",
        "trigger": { "when": "app_launch_succeeded", "context": ["last_destination", "run"] },
        "inputs": [
            { "name": "destination", "type": "string", "required": true, "source": "context" },
            { "name": "app", "type": "string", "required": true, "source": "agent" },
            { "name": "uri", "type": "string", "required": true, "source": "tool" },
            { "name": "run_id", "type": "string", "required": true, "source": "agent" }
        ],
        "layout": [
            { "block": "title", "text": "destination" },
            { "block": "rows", "items": [
                { "label": "App", "value": "app" },
                { "label": "Link", "value": "uri" },
                { "label": "Run", "value": "run_id" }
            ]},
            { "block": "action_button", "label": "Open in Maps", "action": "open_maps" }
        ],
        "actions": [
            { "id": "open_maps", "tool": "app_launch", "args": { "app": "maps", "destination": "destination" } }
        ],
        "lifetime": {
            "appears_when": "a destination is opened in Maps",
            "expires_when": "the destination is opened again, changed, or the conversation moves on",
            "max_ttl_minutes": 30
        }
    });
    let inputs = json!({
        "destination": destination,
        "app": app,
        "uri": uri,
        "run_id": run_id
    });
    (manifest, inputs)
}

/// Build the session card from a `SessionStore` snapshot. Display-only:
/// it declares no actions, so it can offer nothing the agent cannot do.
pub fn session_card(session_id: &str, turn_count: usize, follow_up_open: bool, pending: &[String]) -> (Value, Value) {
    let pending_text = if pending.is_empty() { "none".to_string() } else { pending.join(", ") };
    let follow_up_text = if follow_up_open { "open" } else { "closed" };
    let manifest = json!({
        "id": SESSION_CARD_ID,
        "protocol": "1",
        "user_purpose": "Show the user the current conversation session state at a glance.",
        "trigger": { "when": "session_active", "context": ["session"] },
        "inputs": [
            { "name": "session_id", "type": "string", "required": true, "source": "context" },
            { "name": "turn_count", "type": "number", "required": true, "source": "agent" },
            { "name": "follow_up_window", "type": "string", "required": true, "source": "agent" },
            { "name": "pending", "type": "string", "required": true, "source": "agent" }
        ],
        "layout": [
            { "block": "title", "text": "session_id" },
            { "block": "big_number", "value": "turn_count", "unit": "turns", "caption": "turns" },
            { "block": "rows", "items": [
                { "label": "Follow-up window", "value": "follow_up_window" },
                { "label": "Pending", "value": "pending" }
            ]}
        ],
        "actions": [],
        "lifetime": {
            "appears_when": "a conversation session starts",
            "expires_when": "the session expires",
            "max_ttl_minutes": 120
        }
    });
    let inputs = json!({
        "session_id": session_id,
        "turn_count": turn_count,
        "follow_up_window": follow_up_text,
        "pending": pending_text
    });
    (manifest, inputs)
}

/// Replica of `validateModule` in `apps/kimono-ui/web/modules.mjs`,
/// check for check, so a builder bug is caught in Rust tests and at
/// emission time instead of only in the browser. Returns every error
/// in plain words, mirroring the JS messages.
pub fn validate_manifest(m: &Value) -> Vec<String> {
    let mut errors: Vec<String> = Vec::new();
    let Some(obj) = m.as_object() else {
        return vec!["manifest is not a JSON object".to_string()];
    };
    if !is_non_empty_str(obj.get("id")) {
        errors.push("id is missing or empty".to_string());
    }
    if obj.get("protocol").and_then(Value::as_str) != Some("1") {
        errors.push("protocol must be \"1\"".to_string());
    }
    if !is_non_empty_str(obj.get("user_purpose")) {
        errors.push("user_purpose is missing or empty (required by the minimalism rule)".to_string());
    }
    let trigger_ok = obj
        .get("trigger")
        .and_then(Value::as_object)
        .map(|t| is_non_empty_str(t.get("when")))
        .unwrap_or(false);
    if !trigger_ok {
        errors.push("trigger.when is missing or empty".to_string());
    }

    let mut input_names: Vec<String> = Vec::new();
    match obj.get("inputs") {
        Some(Value::Array(inputs)) => {
            for inp in inputs {
                let Some(name) = inp.get("name").and_then(Value::as_str).filter(|s| !s.trim().is_empty()) else {
                    errors.push("an input has no name".to_string());
                    continue;
                };
                input_names.push(name.to_string());
                let ty = inp.get("type");
                if !ty.and_then(Value::as_str).map(|t| INPUT_TYPES.contains(&t)).unwrap_or(false) {
                    errors.push(format!("input {name}: type {} is not in the protocol set", ty.map(|v| v.to_string()).unwrap_or_else(|| "undefined".into())));
                }
                let src = inp.get("source");
                if !src.and_then(Value::as_str).map(|s| INPUT_SOURCES.contains(&s)).unwrap_or(false) {
                    errors.push(format!("input {name}: source {} is not in the protocol set", src.map(|v| v.to_string()).unwrap_or_else(|| "undefined".into())));
                }
            }
        }
        _ => errors.push("inputs must be an array".to_string()),
    }

    let mut action_ids: Vec<String> = Vec::new();
    match obj.get("actions") {
        Some(Value::Array(actions)) => {
            for a in actions {
                let Some(id) = a.get("id").and_then(Value::as_str).filter(|s| !s.trim().is_empty()) else {
                    errors.push("an action has no id".to_string());
                    continue;
                };
                action_ids.push(id.to_string());
                let tool = a.get("tool");
                if !tool.and_then(Value::as_str).map(|t| TOOL_NAMES.contains(&t)).unwrap_or(false) {
                    errors.push(format!("action {id}: tool {} is not a real agentd tool", tool.map(|v| v.to_string()).unwrap_or_else(|| "undefined".into())));
                }
                if tool.and_then(Value::as_str) == Some("app_launch") {
                    if let Some(app) = a.get("args").and_then(|args| args.get("app")) {
                        if app.as_str() != Some("maps") {
                            errors.push(format!("action {id}: app_launch args.app may only be \"maps\""));
                        }
                    }
                }
            }
        }
        _ => errors.push("actions must be an array (it may be empty)".to_string()),
    }

    match obj.get("layout") {
        Some(Value::Array(blocks)) if !blocks.is_empty() => {
            for (i, b) in blocks.iter().enumerate() {
                let block = b.get("block").and_then(Value::as_str);
                if !block.map(|t| BLOCK_TYPES.contains(&t)).unwrap_or(false) {
                    errors.push(format!(
                        "layout[{i}]: block {} is not in the closed vocabulary ({})",
                        block.map(|t| format!("\"{t}\"")).unwrap_or_else(|| b.to_string()),
                        BLOCK_TYPES.join(", ")
                    ));
                    continue;
                }
                match block {
                    Some("title") if !is_non_empty_str(b.get("text")) => {
                        errors.push(format!("layout[{i}]: title needs a text"))
                    }
                    Some("big_number") if !is_non_empty_str(b.get("value")) => {
                        errors.push(format!("layout[{i}]: big_number needs a value"))
                    }
                    Some("rows") if !matches!(b.get("items"), Some(Value::Array(_))) => {
                        errors.push(format!("layout[{i}]: rows needs an items array"))
                    }
                    Some("action_button") => {
                        if !is_non_empty_str(b.get("label")) {
                            errors.push(format!("layout[{i}]: action_button needs a label"));
                        }
                        let act = b.get("action").and_then(Value::as_str).unwrap_or("");
                        if !action_ids.iter().any(|id| id == act) {
                            errors.push(format!("layout[{i}]: action_button references undeclared action \"{act}\""));
                        }
                    }
                    _ => {}
                }
            }
        }
        _ => errors.push("layout must be a non-empty array of blocks".to_string()),
    }

    let ttl_ok = obj
        .get("lifetime")
        .and_then(|l| l.get("max_ttl_minutes"))
        .and_then(Value::as_f64)
        .map(|n| n > 0.0)
        .unwrap_or(false);
    if !ttl_ok {
        errors.push("lifetime.max_ttl_minutes must be a positive number (every module expires)".to_string());
    }
    errors
}

/// True when every required input has a present, non-empty value —
/// the same missing-value check `renderModuleCard` applies.
pub fn missing_required_inputs(manifest: &Value, inputs: &Value) -> Vec<String> {
    let mut missing = Vec::new();
    if let Some(arr) = manifest.get("inputs").and_then(Value::as_array) {
        for inp in arr {
            let required = inp.get("required").and_then(Value::as_bool).unwrap_or(false);
            let Some(name) = inp.get("name").and_then(Value::as_str) else { continue };
            if !required {
                continue;
            }
            match inputs.get(name) {
                None | Some(Value::Null) => missing.push(name.to_string()),
                Some(Value::String(s)) if s.is_empty() => missing.push(name.to_string()),
                _ => {}
            }
        }
    }
    missing
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn destination_card_validates_and_has_values() {
        let (m, inputs) = destination_card("San Francisco", "maps", "geo:0,0?q=San%20Francisco", "run-test-1");
        assert_eq!(validate_manifest(&m), Vec::<String>::new());
        assert_eq!(missing_required_inputs(&m, &inputs), Vec::<String>::new());
        assert_eq!(m["id"], json!(DESTINATION_CARD_ID));
        assert!(is_non_empty_str(m.get("user_purpose")));
    }

    #[test]
    fn session_card_validates_and_has_values() {
        let (m, inputs) = session_card("sess-1", 2, true, &["answer_question".to_string()]);
        assert_eq!(validate_manifest(&m), Vec::<String>::new());
        assert_eq!(missing_required_inputs(&m, &inputs), Vec::<String>::new());
        assert_eq!(inputs["turn_count"], json!(2));
        assert_eq!(inputs["follow_up_window"], json!("open"));
        assert_eq!(inputs["pending"], json!("answer_question"));
        let (m2, inputs2) = session_card("sess-1", 0, false, &[]);
        assert_eq!(validate_manifest(&m2), Vec::<String>::new());
        assert_eq!(inputs2["pending"], json!("none"));
    }

    #[test]
    fn validator_refuses_missing_purpose_unknown_block_and_tool() {
        let (m, _) = destination_card("X", "maps", "geo:0,0?q=X", "run-1");
        let mut bad = m.clone();
        bad.as_object_mut().unwrap().remove("user_purpose");
        assert!(validate_manifest(&bad).iter().any(|e| e.contains("user_purpose")));
        let mut bad2 = m.clone();
        bad2["layout"] = json!([{"block": "video", "url": "https://example.invalid/x"}]);
        assert!(validate_manifest(&bad2).iter().any(|e| e.contains("video")));
        let mut bad3 = m.clone();
        bad3["actions"] = json!([{"id": "run_it", "tool": "shell_exec", "args": {}}]);
        assert!(validate_manifest(&bad3).iter().any(|e| e.contains("shell_exec")));
    }
}
