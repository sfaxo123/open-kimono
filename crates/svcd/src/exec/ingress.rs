//! Ingress: everything that can wake a service. Each entry point matches
//! triggers deterministically (zero LLM tokens) and fires them; an utterance
//! nothing matches goes to `ovos.intent.fallback` — the only path that may
//! wake the agent harness (SERVICE-IR.md §3.1).

use std::collections::BTreeMap;

use serde_json::{json, Map, Value};

use super::power::default_params;
use super::run::Firing;
use super::{ServiceStatus, Svcd, TimerEntry};
use crate::expr::json_subset;
use crate::host::ActionHost;
use crate::intent::Sentence;
use crate::ir::{Requirement, TriggerKind};

/// Template matching is bounded work: longer input goes to the fallback.
pub const MAX_UTTERANCE_CHARS: usize = 1024;

impl<H: ActionHost> Svcd<H> {
    fn running_in_priority_order(&self) -> Vec<String> {
        let mut ids: Vec<&String> = self.order.iter().filter(|id| self.services[*id].status == ServiceStatus::Running).collect();
        ids.sort_by_key(|id| self.services[*id].ir.priority_class);
        ids.into_iter().cloned().collect()
    }

    /// A final transcript. Returns the matched `(service, trigger)`, if any.
    pub fn utterance(&mut self, text: &str, utterance_id: Option<&str>) -> Option<(String, String)> {
        let uid = match utterance_id {
            Some(u) => u.to_string(),
            None => {
                let n = self.next_seq();
                format!("utt-{}-{n}", self.now_ms)
            }
        };
        self.journal.custom(self.now_ms, "svc.intent.utterance", json!({ "text": text, "utterance_id": uid }));
        if text.chars().count() > MAX_UTTERANCE_CHARS {
            let why = format!("utterance longer than {MAX_UTTERANCE_CHARS} characters is not template-matched");
            self.emit_bus("ovos.intent.fallback", json!({ "utterance_id": uid, "reason": why }));
            return None;
        }
        for sid in self.running_in_priority_order() {
            let ir = self.services[&sid].ir.clone();
            for t in ir.triggers.iter().filter(|t| t.kind == TriggerKind::Voice) {
                let v = t.voice_intent.as_ref().expect("validated");
                let types = v.slots.iter().map(|(k, s)| (k.clone(), s.ty)).collect::<BTreeMap<_, _>>();
                let hit = v.sentences.iter().find_map(|s| Sentence::parse(s).ok()?.matches(text, &types).map(|m| (s.clone(), m)));
                let Some((sentence, slots)) = hit else { continue };
                let mut params = default_params(&ir);
                for (k, spec) in &v.slots {
                    match (slots.get(k), &spec.default) {
                        (Some(val), _) => {
                            params.insert(k.clone(), val.clone());
                        }
                        (None, Some(d)) => {
                            params.entry(k.clone()).or_insert_with(|| d.clone());
                        }
                        _ => {}
                    }
                }
                let missing: Vec<&str> = ir.parameters.iter().filter(|p| p.requirement == Requirement::Required && !params.contains_key(&p.key)).map(|p| p.key.as_str()).collect();
                if !missing.is_empty() {
                    let v = json!({ "service": sid, "trigger": t.id, "missing_parameters": missing });
                    self.journal.custom(self.now_ms, "svc.intent.incomplete", v);
                    continue;
                }
                let info = json!({ "service": sid, "trigger": t.id, "intent": v.intent_name, "sentence": sentence, "slots": slots, "llm_tokens": 0 });
                self.journal.custom(self.now_ms, "svc.intent.matched", info);
                let payload = json!({ "text": text, "slots": slots });
                self.fire(Firing { service: sid.clone(), trigger: t.id.clone(), payload, occurrence: uid.clone(), utterance_id: Some(uid), params, user: true });
                return Some((sid, t.id.clone()));
            }
        }
        self.emit_bus("ovos.intent.fallback", json!({ "utterance": text, "utterance_id": uid }));
        None
    }

    /// A scheme reading (e.g. `sys:power/battery` with `capacity_pct`).
    pub fn sensor_sample(&mut self, scheme: &str, payload: Map<String, Value>) {
        let now = self.now_ms;
        self.sensors.insert(scheme.to_string(), payload.clone());
        self.journal.custom(now, "svc.sensor.sample", json!({ "scheme": scheme, "payload": payload }));
        let mut fire_now = Vec::new();
        let mut debounce = Vec::new();
        for sid in self.running_in_priority_order() {
            let ir = self.services[&sid].ir.clone();
            for t in ir.triggers.iter().filter(|t| t.kind == TriggerKind::Sensor) {
                let st = t.sensor_threshold.as_ref().expect("validated");
                let Some(v) = (st.scheme_path == scheme).then(|| payload.get(&st.metric).and_then(Value::as_f64)).flatten() else { continue };
                let track = self.services.get_mut(&sid).unwrap().sensors.get_mut(&t.id).expect("tracked");
                if track.sample(st, v) {
                    if t.debounce_ms > 0 {
                        track.pending = true;
                        debounce.push((sid.clone(), t.id.clone(), now + t.debounce_ms));
                    } else {
                        fire_now.push((sid.clone(), t.id.clone(), default_params(&ir)));
                    }
                }
            }
        }
        for (service, trigger, due) in debounce {
            self.journal.custom(now, "svc.trigger.debouncing", json!({ "service": service, "trigger": trigger, "until_ms": due }));
            self.add_timer(due, TimerEntry::Debounce { service, trigger });
        }
        for (service, trigger, params) in fire_now {
            let payload = Value::Object(payload.clone());
            self.fire(Firing { service, trigger, payload, occurrence: format!("s{now}"), utterance_id: None, params, user: false });
        }
    }

    /// An A2UI client action. Only the surface's owner can receive it, and
    /// only while the surface is visible.
    pub fn ui_action(&mut self, surface_id: &str, name: &str, source: Option<&str>) -> bool {
        let Some(owner) = self.surfaces.get(surface_id).map(|s| s.owner.clone()) else {
            let v = json!({ "surfaceId": surface_id, "name": name, "reason": "surface is not visible" });
            self.journal.custom(self.now_ms, "svc.ui.dropped", v);
            return false;
        };
        let ir = self.services[&owner].ir.clone();
        let hit = ir.triggers.iter().find(|t| {
            t.ui_action.as_ref().is_some_and(|u| {
                u.surface_id == surface_id && u.action_name == name && u.source_component_id.as_deref().is_none_or(|s| Some(s) == source)
            })
        });
        let Some(t) = hit else {
            self.journal.custom(self.now_ms, "svc.ui.unhandled", json!({ "surfaceId": surface_id, "name": name }));
            return false;
        };
        let payload = json!({ "surfaceId": surface_id, "name": name, "sourceComponentId": source });
        let occurrence = format!("ui{}", self.next_seq());
        self.fire(Firing { service: owner, trigger: t.id.clone(), payload, occurrence, utterance_id: None, params: default_params(&ir), user: true });
        true
    }

    /// An inbound OVOS bus message (counts as a wakeup).
    pub fn bus_message(&mut self, msg_type: &str, data: Value) {
        for sid in self.running_in_priority_order() {
            let ir = self.services[&sid].ir.clone();
            for t in &ir.triggers {
                let Some(m) = &t.bus_message else { continue };
                if m.msg_type != msg_type || !m.data_filter.as_ref().is_none_or(|f| json_subset(&Value::Object(f.clone()), &data)) {
                    continue;
                }
                let occurrence = format!("bus{}", self.next_seq());
                let f = Firing { service: sid.clone(), trigger: t.id.clone(), payload: data.clone(), occurrence, utterance_id: None, params: default_params(&ir), user: false };
                self.fire(f);
            }
        }
    }

    /// `svc:stop` — targeted (one service) or global. `lifecycle_stop`
    /// triggers run first so a service can clean up.
    pub fn stop(&mut self, target: Option<&str>, reason: &str) {
        let targets: Vec<String> = match target {
            Some(t) => vec![t.to_string()],
            None => self.order.clone(),
        };
        for sid in targets {
            let Some(rt) = self.services.get(&sid) else { continue };
            let ir = rt.ir.clone();
            for t in ir.triggers.iter().filter(|t| t.kind == TriggerKind::LifecycleStop) {
                let occurrence = format!("stop{}", self.next_seq());
                let f = Firing { service: sid.clone(), trigger: t.id.clone(), payload: json!({ "reason": reason }), occurrence, utterance_id: None, params: default_params(&ir), user: true };
                self.fire(f);
            }
            self.stop_service(&sid, reason, None);
        }
    }
}
