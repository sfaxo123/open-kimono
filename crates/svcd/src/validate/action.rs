//! Per-action meaning checks: payload matches type, effect classes are
//! coherent (SERVICE-IR.md §4.1), expressions read known names, the action
//! is covered by capabilities and scheme deps, and bus emissions cannot stop
//! svcd or loop back into the same service (§6.2 rule 5).

use super::graph::{has_cap, has_dep, Names};
use super::Report;
use crate::error::{ptr, Code};
use crate::ir::*;

/// Bus topics no service may emit (svcd, the broker, the journal, init/watchdog).
const GUARDED_TOPICS: [&str; 7] = ["svcd.", "svc.stop", "caps.", "trace.", "kernel.", "init.", "watchdog."];

pub(super) fn action(ir: &ServiceIr, id: &str, a: &ActionNode, r: &mut Report, names: &Names) {
    let base = ptr("/actions", id);
    let payloads = [
        ("wasm_call", a.wasm_call.is_some(), ActionType::Logic),
        ("state_patch", a.state_patch.is_some(), ActionType::StateMutate),
        ("dialog", a.dialog.is_some(), ActionType::Dialog),
        ("ui_patch", a.ui_patch.is_some(), ActionType::UiPatch),
        ("bus_emit", a.bus_emit.is_some(), ActionType::BusEmit),
    ];
    for (field, is_set, ty) in payloads {
        if ty == a.ty && !is_set {
            r.err(ptr(&base, field), Code::MissingField, format!("`{}` action needs `{field}`", a.ty.as_str()));
        } else if ty != a.ty && is_set {
            r.err(ptr(&base, field), Code::Contradiction, format!("`{field}` given on a `{}` action", a.ty.as_str()));
        }
    }
    let contradiction = |r: &mut Report, field: &str, msg: &str| r.err(ptr(&base, field), Code::Contradiction, msg);
    match (a.ty, a.effect_class) {
        (ActionType::SubService, _) => r.err(ptr(&base, "type"), Code::Unsupported, "sub_service actions are not implemented by svcd MVP"),
        (ActionType::StateMutate, _) if a.read_only => contradiction(r, "read_only", "a state_mutate action cannot be read_only"),
        (t, EffectClass::Pure) if t != ActionType::Logic => contradiction(r, "effect_class", "only logic actions can be `pure`"),
        (ActionType::Dialog, e) if e != EffectClass::CommittedIrreversible => contradiction(r, "effect_class", "speech cannot be undone: dialog must be `committed_irreversible`"),
        _ => {}
    }
    if a.effect_class == EffectClass::Pure && !a.read_only {
        contradiction(r, "read_only", "a `pure` action must be read_only");
    }
    if a.timeout.idle_ms > a.timeout.total_ms {
        contradiction(r, "timeout", "idle_ms exceeds total_ms");
    }
    match (&a.compensating_action, a.effect_class) {
        (None, EffectClass::ReversibleCompensated) => r.err(ptr(&base, "compensating_action"), Code::MissingField, "`reversible_compensated` needs a compensating_action"),
        (Some(_), e) if e != EffectClass::ReversibleCompensated => contradiction(r, "compensating_action", "compensating_action is only meaningful on `reversible_compensated`"),
        (Some(c), _) => match ir.actions.get(c) {
            None => r.err(ptr(&base, "compensating_action"), Code::UnknownReference, format!("no action `{c}`")),
            Some(ca) if ca.effect_class == EffectClass::CommittedIrreversible => contradiction(r, "compensating_action", "a compensation cannot itself be irreversible"),
            _ => {}
        },
        _ => {}
    }
    match (&a.idempotency_key_template, a.effect_class) {
        (None, EffectClass::CommittedIrreversible) => r.err(ptr(&base, "idempotency_key_template"), Code::MissingField, "a `committed_irreversible` action needs an idempotency_key_template so a retry never repeats it"),
        (Some(k), _) => names.value(r, ptr(&base, "idempotency_key_template"), &serde_json::Value::String(k.clone())),
        _ => {}
    }
    for (j, e) in a.next.iter().enumerate() {
        let eb = ptr(&ptr(&base, "next"), j);
        if !ir.actions.contains_key(&e.target) {
            r.err(ptr(&eb, "target"), Code::UnknownReference, format!("no action `{}`", e.target));
        }
        if let Some(w) = &e.when_cel {
            names.cel(r, ptr(&eb, "when_cel"), w, true);
        }
    }
    if let Some(ops) = &a.state_patch {
        if ops.is_empty() {
            r.err(ptr(&base, "state_patch"), Code::Length, "state_patch needs at least one operation");
        }
        for (j, op) in ops.iter().enumerate() {
            let ob = ptr(&ptr(&base, "state_patch"), j);
            let first = op.path.trim_start_matches('/').split('/').next().unwrap_or("");
            if names.state_props.is_some_and(|s| !s.contains(first)) {
                r.err(ptr(&ob, "path"), Code::StateSchema, format!("`/{first}` is not a property of state_schema"));
            }
            match (op.op, &op.value) {
                (PatchOp::Remove, Some(_)) => r.err(ptr(&ob, "value"), Code::Contradiction, "`remove` takes no value"),
                (PatchOp::Remove, None) => {}
                (_, None) => r.err(ptr(&ob, "value"), Code::MissingField, format!("`{}` needs a value", op.op.as_str())),
                (_, Some(v)) => names.value(r, ptr(&ob, "value"), v),
            }
        }
    }
    if let Some(d) = &a.dialog {
        names.value(r, ptr(&ptr(&base, "dialog"), "speech"), &serde_json::Value::String(d.speech.clone()));
        names.value(r, ptr(&ptr(&base, "dialog"), "text"), &serde_json::Value::String(d.text.clone()));
        if !has_cap(ir, "audio.playback.tts") && !has_cap(ir, "audio.playback.alarm") {
            r.err(base.clone(), Code::Capability, "dialog needs capability `audio.playback.tts` or `audio.playback.alarm`");
        }
        if !has_dep(ir, |d| d.scheme.starts_with("audio:") && d.access != SchemeAccess::Read) {
            r.err(base.clone(), Code::Dependency, "dialog needs a writable `audio:` scheme dependency");
        }
    }
    if let Some(u) = &a.ui_patch {
        if let Some(v) = &u.data_model_value {
            names.value(r, ptr(&ptr(&base, "ui_patch"), "data_model_value"), v);
        }
        let covered = ir.capabilities.iter().any(|c| {
            c.capability == "ui.surface.render" && c.scope.get("surface_id").is_none_or(|s| s.as_str() == Some(u.surface_id.as_str()))
        });
        if !covered {
            r.err(ptr(&ptr(&base, "ui_patch"), "surface_id"), Code::Capability, format!("no `ui.surface.render` grant covers surface `{}`", u.surface_id));
        }
        if u.operation == UiOperation::OpenuiStatements && u.openui_statements.is_none() {
            r.err(ptr(&ptr(&base, "ui_patch"), "openui_statements"), Code::MissingField, "operation `openui_statements` needs statements");
        }
    }
    if let Some(b) = &a.bus_emit {
        let bb = ptr(&base, "bus_emit");
        names.value(r, ptr(&bb, "data"), &serde_json::Value::Object(b.data.clone()));
        if GUARDED_TOPICS.iter().any(|g| b.msg_type.starts_with(g)) {
            r.err(ptr(&bb, "msg_type"), Code::LifecycleGuard, format!("`{}` targets svcd, caps:, trace: or init and may not be emitted by a service", b.msg_type));
        }
        if ir.triggers.iter().filter_map(|t| t.bus_message.as_ref()).any(|m| m.msg_type == b.msg_type) {
            r.err(ptr(&bb, "msg_type"), Code::LifecycleGuard, format!("emitting `{}` would re-trigger this service (bus loop)", b.msg_type));
        }
    }
    if let Some(w) = &a.wasm_call {
        let wb = ptr(&base, "wasm_call");
        if !ir.deps.wasm_components.iter().any(|c| c.id == w.component_ref) {
            r.err(ptr(&wb, "component_ref"), Code::UnknownReference, format!("no deps.wasm_components entry `{}`", w.component_ref));
        }
        names.value(r, ptr(&wb, "args_mapping"), &serde_json::Value::Object(w.args_mapping.clone()));
        if w.post_hook_jq.is_some() {
            r.err(ptr(&wb, "post_hook_jq"), Code::Unsupported, "post_hook_jq is not implemented by svcd MVP");
        }
        if a.effect_class == EffectClass::CommittedIrreversible && w.output_state_pointer.is_some() {
            r.err(ptr(&wb, "output_state_pointer"), Code::Contradiction, "irreversible actions run after commit, so their output cannot be written to state");
        }
    }
}
