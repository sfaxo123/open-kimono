//! One trigger firing = one ledger run = one transaction (SERVICE-IR.md §3.2,
//! §4): guard → admission (cooldown, wakeup budget, duplicate occurrence) →
//! DAG walk against a copy-on-write state overlay with staged UI/bus outbox →
//! state-schema check → atomic commit → post-commit flush of irreversible
//! effects, each recorded in the ledger before anything else can fail.

use std::collections::{BTreeSet, VecDeque};
use std::sync::Arc;

use serde_json::{json, Map, Value};
use trace_events::AgUiEvent;

use super::effects::{Abort, Deferred, Staged};
use super::{Svcd, ServiceStatus};
use crate::expr::{self, Scope};
use crate::host::ActionHost;
use crate::ir::{ActionType, EffectClass, ServiceIr};
use crate::ledger::{self, EffectRow, ExecStatus};
use crate::validate::state_schema;

/// What fired, and with which inputs.
#[derive(Clone, Debug)]
pub(crate) struct Firing {
    pub service: String,
    pub trigger: String,
    pub payload: Value,
    /// Utterance id, scheduled instant, sample time... (stable across retries).
    pub occurrence: String,
    pub utterance_id: Option<String>,
    pub params: Map<String, Value>,
    /// User-initiated (voice, touch) — not counted as a CPU wakeup.
    pub user: bool,
}

/// Names an expression can read during one execution.
pub(crate) struct ExecScope<'a> {
    pub state: &'a Value,
    pub firing: &'a Firing,
    pub session: &'a Value,
    pub execution_id: &'a str,
    pub action_id: Option<&'a str>,
    pub now_ms: u64,
}

impl Scope for ExecScope<'_> {
    fn root(&self, name: &str) -> Option<Value> {
        Some(match name {
            "state" => self.state.clone(),
            "params" => Value::Object(self.firing.params.clone()),
            "trigger" => json!({ "id": self.firing.trigger, "payload": self.firing.payload }),
            "session" => self.session.clone(),
            "utterance_id" => Value::String(self.firing.utterance_id.clone()?),
            "execution_id" => Value::String(self.execution_id.to_string()),
            "service_id" => Value::String(self.firing.service.clone()),
            "action_id" => Value::String(self.action_id?.to_string()),
            "trigger_id" => Value::String(self.firing.trigger.clone()),
            "now_ms" => Value::from(self.now_ms),
            _ => return None,
        })
    }
}

/// Everything one attempt stages before commit.
#[derive(Default)]
pub(crate) struct Staging {
    pub overlay: Value,
    pub deltas: Vec<Value>,
    pub outbox: Vec<Staged>,
    pub deferred: Vec<Deferred>,
    pub compensations: Vec<(String, String)>,
    pub effects: Vec<EffectRow>,
}

impl<H: ActionHost> Svcd<H> {
    /// Fire a trigger, then re-derive timers and powered schemes — also when
    /// the firing was suppressed or dropped.
    pub(crate) fn fire(&mut self, f: Firing) {
        let sid = f.service.clone();
        self.fire_inner(f);
        self.after_execution(&sid);
    }

    fn fire_inner(&mut self, f: Firing) {
        let now = self.now_ms;
        let Some(rt) = self.services.get(&f.service) else { return };
        if let ServiceStatus::Stopped { reason, .. } = &rt.status {
            let v = json!({ "service": f.service, "trigger": f.trigger, "reason": format!("service stopped: {reason}") });
            self.journal.custom(now, "svc.trigger.dropped", v);
            return;
        }
        let ir = rt.ir.clone();
        let trig = ir.triggers.iter().find(|t| t.id == f.trigger).expect("trigger exists").clone();
        if let Some(c) = &trig.condition_cel {
            let scope = ExecScope { state: &rt.state, firing: &f, session: &self.session, execution_id: "", action_id: None, now_ms: now };
            let verdict = expr::parse(c).and_then(|e| e.eval_bool(&scope));
            if verdict != Ok(true) {
                let why = match verdict {
                    Err(e) => format!("guard error: {e}"),
                    _ => format!("guard `{c}` is false"),
                };
                self.journal.custom(now, "svc.trigger.suppressed", json!({ "service": f.service, "trigger": f.trigger, "reason": why }));
                return;
            }
        }
        if let Some(&last) = rt.last_fire.get(&f.trigger) {
            if trig.cooldown_ms > 0 && now < last + trig.cooldown_ms {
                let why = format!("cooldown {} ms since {last}", trig.cooldown_ms);
                self.journal.custom(now, "svc.trigger.suppressed", json!({ "service": f.service, "trigger": f.trigger, "reason": why }));
                return;
            }
        }
        if !f.user {
            if let Some(v) = self.admit_wakeup(&f.service) {
                self.journal.custom(now, "svc.trigger.dropped", json!({ "service": f.service, "trigger": f.trigger, "reason": "wakeup budget" }));
                self.stop_service(&f.service, "power budget exceeded", Some(v));
                return;
            }
        }
        self.services.get_mut(&f.service).unwrap().last_fire.insert(f.trigger.clone(), now);

        let exec_id = ledger::execution_id(&f.service, &f.trigger, &f.occurrence);
        if self.ledger.completed(&exec_id) {
            self.journal.custom(now, "svc.execution.duplicate", json!({ "service": f.service, "execution_id": exec_id }));
            return;
        }
        let run_id = self.ledger.claim(&exec_id, &f.service, &f.trigger, now);
        self.journal.begin_run(&run_id);
        let ctx = json!({ "service": f.service, "trigger": f.trigger, "execution_id": exec_id });
        self.emit_bus("ovos.intent.handler.start", ctx.clone());
        self.journal.agui(now, AgUiEvent::RunStarted {
            thread_id: f.service.clone(),
            run_id: run_id.clone(),
            protocol_version: None,
            parent_run_id: None,
            input: Some(json!({ "trigger": f.trigger, "payload": f.payload, "params": f.params })),
        });
        self.ledger.transition(&run_id, ExecStatus::Running, now, None).expect("claimed -> running");

        let attempts = 1 + ir.retry.as_ref().map_or(0, |r| r.max_retries);
        let mut result = Err(Abort::Failed("not run".into()));
        for n in 1..=attempts {
            self.ledger.bump_attempts(&run_id);
            result = self.attempt(&ir, &f, &exec_id, &run_id, &trig.entry_action);
            match &result {
                Err(Abort::Failed(msg)) if n < attempts => {
                    let v = json!({ "service": f.service, "run_id": run_id, "attempt": n, "error": msg });
                    self.journal.custom(self.now_ms, "svc.execution.retry", v);
                }
                _ => break,
            }
        }
        let now = self.now_ms;
        match result {
            Ok(()) => {
                self.ledger.transition(&run_id, ExecStatus::Completed, now, None).expect("running -> completed");
                self.emit_bus("ovos.intent.handler.complete", ctx);
                let result = json!({ "status": "completed", "llm_tokens": 0 });
                self.journal.agui(now, AgUiEvent::RunFinished { thread_id: f.service.clone(), run_id, result: Some(result), outcome: None, usage: vec![] });
            }
            Err(abort) => {
                let (code, msg) = abort.describe();
                self.ledger.transition(&run_id, ExecStatus::Failed, now, Some(msg.clone())).expect("running -> failed");
                let mut data = ctx;
                data["error"] = Value::String(msg.clone());
                self.emit_bus("ovos.intent.handler.error", data);
                self.journal.agui(now, AgUiEvent::RunError { message: msg, code: Some(code.into()), usage: vec![] });
                if let Abort::Budget(v) = abort {
                    self.stop_service(&f.service, "power budget exceeded", Some(v));
                }
            }
        }
        self.journal.end_run();
    }

    fn attempt(&mut self, ir: &Arc<ServiceIr>, f: &Firing, exec_id: &str, run_id: &str, entry: &str) -> Result<(), Abort> {
        let mut st = Staging { overlay: self.services[&ir.id].state.clone(), ..Default::default() };
        let walked = self.walk(ir, f, exec_id, run_id, entry, &mut st).and_then(|()| check_state(ir, &st.overlay));
        if let Err(abort) = walked {
            self.compensate(ir, exec_id, &st.compensations);
            return Err(abort);
        }
        let deferred = std::mem::take(&mut st.deferred);
        self.commit(&ir.id, st);
        self.flush_deferred(ir, exec_id, run_id, deferred)
    }

    fn walk(&mut self, ir: &ServiceIr, f: &Firing, exec_id: &str, run_id: &str, entry: &str, st: &mut Staging) -> Result<(), Abort> {
        let mut queue = VecDeque::from([entry.to_string()]);
        let mut visited = BTreeSet::new();
        while let Some(aid) = queue.pop_front() {
            if !visited.insert(aid.clone()) {
                continue;
            }
            let a = &ir.actions[&aid];
            self.journal.step(run_id, &aid);
            self.journal.agui(self.now_ms, AgUiEvent::StepStarted { step_name: aid.clone() });
            let key = if a.effect_class == EffectClass::Pure { None } else { Some(self.key_for(ir, f, exec_id, &aid, a, st)?) };
            let done = key.as_ref().and_then(|(k, _)| self.ledger.effect(&ir.id, k)).cloned();
            if let Some(row) = done {
                let v = json!({ "service": ir.id, "action": aid, "key": row.key, "material": row.material, "output": row.output });
                self.journal.custom(self.now_ms, "svc.action.replayed", v);
            } else if a.ty == ActionType::Dialog || (a.ty == ActionType::Logic && a.effect_class == EffectClass::CommittedIrreversible) {
                let kind = self.action_kind(ir, f, exec_id, &aid, a, &st.overlay)?;
                st.deferred.push(Deferred { action_id: aid.clone(), key, kind });
            } else {
                self.stage(ir, f, exec_id, &aid, a, key, st)?;
            }
            self.check_memory(&ir.id, st)?;
            self.journal.agui(self.now_ms, AgUiEvent::StepFinished { step_name: aid.clone() });
            for e in &a.next {
                let go = match &e.when_cel {
                    None => true,
                    Some(c) => {
                        let scope = ExecScope { state: &st.overlay, firing: f, session: &self.session, execution_id: exec_id, action_id: Some(&aid), now_ms: self.now_ms };
                        expr::parse(c).and_then(|x| x.eval_bool(&scope)).map_err(|e| Abort::Failed(format!("{aid}: edge guard: {e}")))?
                    }
                };
                if go {
                    queue.push_back(e.target.clone());
                }
            }
        }
        self.journal.span = None;
        Ok(())
    }

    /// Atomic commit: state, ledger rows for staged effects, then the outbox.
    fn commit(&mut self, sid: &str, st: Staging) {
        let now = self.now_ms;
        let rt = self.services.get_mut(sid).unwrap();
        if rt.state != st.overlay {
            rt.state = st.overlay;
            rt.generation += 1;
            rt.dedup.clear();
        }
        if !st.deltas.is_empty() {
            let delta = json!({ "service": sid, "generation": rt.generation, "patch": st.deltas });
            self.journal.agui(now, AgUiEvent::StateDelta { delta });
        }
        for row in st.effects {
            if let Err(e) = self.ledger.record_effect(row) {
                self.journal.custom(now, "svc.ledger.conflict", json!({ "service": sid, "error": e.to_string() }));
            }
        }
        for item in st.outbox {
            match item {
                Staged::Ui { patch, value } => self.apply_ui(sid, &patch, value),
                Staged::Bus { msg_type, data } => self.emit_bus(&msg_type, data),
            }
        }
    }

    /// Irreversible effects run after commit, in DAG order. Each success is
    /// recorded before the next starts, so a retry never repeats it.
    fn flush_deferred(&mut self, ir: &ServiceIr, exec_id: &str, run_id: &str, deferred: Vec<Deferred>) -> Result<(), Abort> {
        for d in deferred {
            if let Some((k, _)) = &d.key {
                if let Some(row) = self.ledger.effect(&ir.id, k).cloned() {
                    let v = json!({ "service": ir.id, "action": d.action_id, "key": row.key, "output": row.output });
                    self.journal.custom(self.now_ms, "svc.action.replayed", v);
                    continue;
                }
            }
            self.journal.step(run_id, &d.action_id);
            let a = &ir.actions[&d.action_id];
            let outcome = self.call_host(ir, exec_id, &d.action_id, d.kind.clone(), d.key.as_ref().map(|k| k.0.clone()))?;
            if let Some((key, material)) = d.key {
                let row = EffectRow {
                    service_id: ir.id.clone(),
                    key,
                    material,
                    action_id: d.action_id.clone(),
                    execution_id: exec_id.to_string(),
                    effect_class: a.effect_class,
                    output: outcome.output.clone(),
                    at_ms: self.now_ms,
                };
                self.ledger.record_effect(row).expect("checked absent above");
            }
            if let crate::host::ActionKind::Dialog { speech, text, priority } = &d.kind {
                let lang = self.session.get("lang").cloned().unwrap_or(json!("en-US"));
                let data = json!({ "utterance": speech, "text": text, "priority": priority.as_str(), "service": ir.id, "lang": lang });
                self.emit_bus("speak", data);
            }
            self.check_outcome(ir, &d.action_id, a, &outcome)?;
        }
        self.journal.span = None;
        Ok(())
    }

    pub(crate) fn emit_bus(&mut self, msg_type: &str, data: Value) {
        let context = json!({ "source": "svcd", "session": self.session });
        let msg = super::BusMessage { msg_type: msg_type.to_string(), data, context, at_ms: self.now_ms };
        self.journal.custom(self.now_ms, "ovos.bus", serde_json::to_value(&msg).expect("serializes"));
        self.bus.push(msg);
    }
}

fn check_state(ir: &ServiceIr, overlay: &Value) -> Result<(), Abort> {
    let schema = Value::Object(ir.state_schema.schema.clone());
    match state_schema::validate(&schema, overlay).first() {
        None => Ok(()),
        Some(e) => Err(Abort::Failed(format!("state would violate state_schema at {}: {}", if e.path.is_empty() { "/" } else { &e.path }, e.message))),
    }
}
