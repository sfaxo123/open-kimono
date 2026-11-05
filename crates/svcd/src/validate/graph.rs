//! Cross-reference and meaning checks the D1 schema cannot express:
//! references resolve, the action graph is a DAG, every expression parses and
//! reads only known names, effect classes are coherent, actions are covered
//! by requested capabilities and scheme deps, and nothing can stop svcd or
//! loop on its own bus emissions (SERVICE-IR.md §4.1, §6.2 rule 5).

use std::collections::{BTreeMap, BTreeSet};

use super::Report;
use crate::error::{ptr, Code};
use crate::intent::Sentence;
use crate::ir::*;
use crate::{expr, template};

/// Roots an expression may read (`session` = OVOS session: `location`, `lang`).
pub const ROOTS: [&str; 10] =
    ["state", "params", "trigger", "session", "utterance_id", "execution_id", "service_id", "action_id", "trigger_id", "now_ms"];

pub fn check(ir: &ServiceIr, r: &mut Report) {
    let state_props: Option<BTreeSet<String>> = ir
        .state_schema
        .schema
        .get("properties")
        .and_then(|p| p.as_object())
        .map(|p| p.keys().cloned().collect());
    let params: BTreeSet<&str> = ir.parameters.iter().map(|p| p.key.as_str()).collect();
    let names = Names { state_props: state_props.as_ref(), params: &params };

    duplicates(r, "/parameters", ir.parameters.iter().map(|p| p.key.as_str()), "key");
    duplicates(r, "/triggers", ir.triggers.iter().map(|t| t.id.as_str()), "id");
    duplicates(r, "/capabilities", ir.capabilities.iter().map(|c| c.cap_id.as_str()), "cap_id");
    duplicates(r, "/deps/wasm_components", ir.deps.wasm_components.iter().map(|c| c.id.as_str()), "id");

    for (i, w) in ir.workflow.iter().flatten().enumerate() {
        if !ir.actions.contains_key(w) {
            r.err(ptr("/workflow", i), Code::UnknownReference, format!("no action `{w}`"));
        }
    }
    triggers(ir, r, &names);
    for (id, a) in &ir.actions {
        super::action::action(ir, id, a, r, &names);
    }
    cycles(ir, r);
    budget(ir, r);
    if !ir.deps.sub_services.is_empty() {
        r.err("/deps/sub_services", Code::Unsupported, "sub-services are not implemented by svcd MVP");
    }
    if ir.ui.mode != UiMode::None && ir.ui.surface_id.is_none() {
        r.err("/ui/surface_id", Code::MissingField, format!("ui mode `{}` needs a surface_id", ir.ui.mode.as_str()));
    }
}

pub(super) struct Names<'a> {
    pub(super) state_props: Option<&'a BTreeSet<String>>,
    params: &'a BTreeSet<&'a str>,
}

impl Names<'_> {
    fn check_paths(&self, r: &mut Report, path: &str, src: &str, paths: &[Vec<String>], allow_trigger: bool) {
        for p in paths {
            let root = p[0].as_str();
            let bad = if !ROOTS.contains(&root) || (root == "trigger" && !allow_trigger) {
                Some(format!("`{root}` is not readable here (known: {})", ROOTS.join(", ")))
            } else if root == "params" && p.get(1).is_some_and(|k| !self.params.contains(k.as_str())) {
                Some(format!("`params.{}` is not a declared parameter", p[1]))
            } else if root == "state" && p.get(1).is_some_and(|k| self.state_props.is_some_and(|s| !s.contains(k))) {
                Some(format!("`state.{}` is not a property of state_schema", p[1]))
            } else {
                None
            };
            if let Some(msg) = bad {
                r.err(path, Code::UnknownReference, format!("{msg} in `{src}`"));
            }
        }
    }

    pub(super) fn cel(&self, r: &mut Report, path: String, src: &str, allow_trigger: bool) {
        match expr::parse(src) {
            Ok(e) => {
                let mut paths = Vec::new();
                e.paths(&mut paths);
                self.check_paths(r, &path, src, &paths, allow_trigger);
            }
            Err(e) => r.err(path, Code::Expression, e.0),
        }
    }

    pub(super) fn value(&self, r: &mut Report, path: String, v: &serde_json::Value) {
        let mut paths = Vec::new();
        match template::value_paths(v, &mut paths) {
            Ok(()) => self.check_paths(r, &path, &v.to_string(), &paths, true),
            Err(e) => r.err(path, Code::Expression, e.0),
        }
    }
}

fn duplicates<'a>(r: &mut Report, base: &str, items: impl Iterator<Item = &'a str>, field: &str) {
    let mut seen = BTreeSet::new();
    for (i, id) in items.enumerate() {
        if !seen.insert(id) {
            r.err(ptr(&ptr(base, i), field), Code::Duplicate, format!("duplicate {field} `{id}`"));
        }
    }
}

pub(super) fn has_cap(ir: &ServiceIr, cap: &str) -> bool {
    ir.capabilities.iter().any(|c| c.capability == cap)
}

pub(super) fn has_dep(ir: &ServiceIr, pred: impl Fn(&SchemeDep) -> bool) -> bool {
    ir.deps.schemes.iter().any(pred)
}

fn triggers(ir: &ServiceIr, r: &mut Report, names: &Names) {
    for (i, t) in ir.triggers.iter().enumerate() {
        let base = ptr("/triggers", i);
        if !ir.actions.contains_key(&t.entry_action) {
            r.err(ptr(&base, "entry_action"), Code::UnknownReference, format!("no action `{}`", t.entry_action));
        }
        if let Some(c) = &t.condition_cel {
            names.cel(r, ptr(&base, "condition_cel"), c, true);
        }
        let present = [
            ("voice_intent", t.voice_intent.is_some(), TriggerKind::Voice),
            ("bus_message", t.bus_message.is_some(), TriggerKind::Bus),
            ("timer_schedule", t.timer_schedule.is_some(), TriggerKind::Timer),
            ("sensor_threshold", t.sensor_threshold.is_some(), TriggerKind::Sensor),
            ("ui_action", t.ui_action.is_some(), TriggerKind::Ui),
        ];
        for (field, is_set, kind) in present {
            if kind == t.kind && !is_set {
                r.err(ptr(&base, field), Code::MissingField, format!("trigger kind `{}` needs `{field}`", t.kind.as_str()));
            } else if kind != t.kind && is_set {
                r.err(ptr(&base, field), Code::Contradiction, format!("`{field}` given on a `{}` trigger", t.kind.as_str()));
            }
        }
        if let Some(v) = &t.voice_intent {
            let vb = ptr(&base, "voice_intent");
            if v.method != IntentMethod::Template {
                r.err(ptr(&vb, "method"), Code::Unsupported, format!("intent method `{}` is not implemented (template only)", v.method.as_str()));
            }
            for k in v.slots.keys() {
                if !names.params.contains(k.as_str()) {
                    r.err(ptr(&ptr(&vb, "slots"), k), Code::UnknownReference, format!("slot `{k}` has no parameter of the same key"));
                }
            }
            for (j, s) in v.sentences.iter().enumerate() {
                let sp = ptr(&ptr(&vb, "sentences"), j);
                match Sentence::parse(s) {
                    Ok(sent) => {
                        for slot in sent.slots() {
                            if !v.slots.contains_key(&slot) {
                                r.err(sp.clone(), Code::UnknownReference, format!("sentence uses undeclared slot `{{{slot}}}`"));
                            }
                        }
                    }
                    Err(e) => r.err(sp, Code::Expression, e),
                }
            }
        }
        if let Some(ts) = &t.timer_schedule {
            let tb = ptr(&base, "timer_schedule");
            match ts.mode {
                TimerMode::Countdown | TimerMode::Interval => match &ts.duration_sec_expr {
                    None => r.err(ptr(&tb, "duration_sec_expr"), Code::MissingField, format!("`{}` needs duration_sec_expr", ts.mode.as_str())),
                    Some(d) => names.value(r, ptr(&tb, "duration_sec_expr"), &serde_json::Value::String(d.clone())),
                },
                m => r.err(ptr(&tb, "mode"), Code::Unsupported, format!("timer mode `{}` is not implemented by svcd MVP", m.as_str())),
            }
            if !has_cap(ir, "timer.monotonic.schedule") {
                r.err(base.clone(), Code::Capability, "timer trigger needs capability `timer.monotonic.schedule`");
            }
            if !has_dep(ir, |d| d.scheme.starts_with("time:")) {
                r.err(base.clone(), Code::Dependency, "timer trigger needs a `time:` scheme dependency");
            }
        }
        if let Some(s) = &t.sensor_threshold {
            if !has_dep(ir, |d| s.scheme_path.starts_with(d.scheme.as_str())) {
                r.err(ptr(&ptr(&base, "sensor_threshold"), "scheme_path"), Code::Dependency, format!("no deps.schemes entry covers `{}`", s.scheme_path));
            }
            if (s.scheme_path.starts_with("sys:power") || s.scheme_path.starts_with("power:")) && !has_cap(ir, "power.battery.read") {
                r.err(base.clone(), Code::Capability, "battery threshold needs capability `power.battery.read`");
            }
        }
    }
}

/// `next` edges must form a DAG; report the first cycle as a path.
fn cycles(ir: &ServiceIr, r: &mut Report) {
    #[derive(Clone, Copy, PartialEq)]
    enum Mark {
        Visiting,
        Done,
    }
    fn visit<'a>(ir: &'a ServiceIr, id: &'a str, marks: &mut BTreeMap<&'a str, Mark>, stack: &mut Vec<&'a str>) -> Option<Vec<&'a str>> {
        match marks.get(id) {
            Some(Mark::Done) => return None,
            Some(Mark::Visiting) => {
                let start = stack.iter().position(|s| *s == id).unwrap_or(0);
                let mut cyc = stack[start..].to_vec();
                cyc.push(id);
                return Some(cyc);
            }
            None => {}
        }
        marks.insert(id, Mark::Visiting);
        stack.push(id);
        for e in ir.actions.get(id).map(|a| a.next.as_slice()).unwrap_or(&[]) {
            if ir.actions.contains_key(&e.target) {
                if let Some(c) = visit(ir, &e.target, marks, stack) {
                    return Some(c);
                }
            }
        }
        stack.pop();
        marks.insert(id, Mark::Done);
        None
    }
    let mut marks = BTreeMap::new();
    for id in ir.actions.keys() {
        if let Some(c) = visit(ir, id, &mut marks, &mut Vec::new()) {
            r.err(ptr(&ptr("/actions", c[0]), "next"), Code::Cycle, format!("action graph has a cycle: {}", c.join(" -> ")));
            return;
        }
    }
}

fn budget(ir: &ServiceIr, r: &mut Report) {
    let pb = &ir.power_budget;
    let background = ir.triggers.iter().any(|t| matches!(t.kind, TriggerKind::Timer | TriggerKind::Sensor | TriggerKind::Bus));
    if background && pb.max_wakeups_per_minute == 0.0 {
        r.err("/power_budget/max_wakeups_per_minute", Code::Contradiction, "timer/sensor/bus triggers need max_wakeups_per_minute > 0");
    }
    let wants_net = pb.network_egress_bytes_max > 0 || !pb.allowed_hosts.is_empty();
    if wants_net && !has_cap(ir, "network.connect") {
        r.err("/power_budget/network_egress_bytes_max", Code::Capability, "network egress needs capability `network.connect`");
    }
}
