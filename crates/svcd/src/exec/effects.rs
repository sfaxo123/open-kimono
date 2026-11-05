//! Effects inside a transaction: idempotency keys, staging of reversible
//! actions, host calls (with demand-powered schemes and per-invocation
//! grants), power-budget checks on what a call cost, and compensation.

use serde_json::{json, Value};
use trace_events::{AgUiEvent, Op6Event};

use super::run::{ExecScope, Firing, Staging};
use super::{BudgetViolation, Svcd};
use crate::host::{ActionHost, ActionKind, ActionOutcome, ActionRequest};
use crate::ir::{ActionNode, ActionType, CapLifetime, CapabilityRequest, DialogPriority, EffectClass, Lifecycle, ServiceIr, UiPatch};
use crate::journal::inline;
use crate::ledger::{self, EffectRow};
use crate::{patch, template};

pub(crate) enum Abort {
    /// Transient or logical failure: the run may be retried.
    Failed(String),
    /// The broker refused a capability: not retried.
    Denied(String),
    /// A power budget was exceeded: the service is stopped.
    Budget(BudgetViolation),
}

impl Abort {
    pub fn describe(&self) -> (&'static str, String) {
        match self {
            Abort::Failed(m) => ("action_failed", m.clone()),
            Abort::Denied(m) => ("capability_denied", m.clone()),
            Abort::Budget(v) => ("budget_exceeded", format!("{} exceeded: limit {}, observed {}", v.budget, v.limit, v.observed)),
        }
    }
}

pub(crate) enum Staged {
    Ui { patch: UiPatch, value: Option<Value> },
    Bus { msg_type: String, data: Value },
}

pub(crate) struct Deferred {
    pub action_id: String,
    /// `(blake3 key, readable material)`.
    pub key: Option<(String, String)>,
    pub kind: ActionKind,
}

fn size(v: &impl serde::Serialize) -> u64 {
    serde_json::to_vec(v).map_or(0, |b| b.len() as u64)
}

const MIB: u64 = 1024 * 1024;

impl<H: ActionHost> Svcd<H> {
    fn scope<'a>(&'a self, f: &'a Firing, overlay: &'a Value, exec_id: &'a str, aid: &'a str) -> ExecScope<'a> {
        ExecScope { state: overlay, firing: f, session: &self.session, execution_id: exec_id, action_id: Some(aid), now_ms: self.now_ms }
    }

    pub(crate) fn key_for(&self, ir: &ServiceIr, f: &Firing, exec_id: &str, aid: &str, a: &ActionNode, st: &Staging) -> Result<(String, String), Abort> {
        let material = match &a.idempotency_key_template {
            Some(t) => template::render_text(t, &self.scope(f, &st.overlay, exec_id, aid)).map_err(|e| Abort::Failed(format!("{aid}: idempotency key: {e}")))?,
            None => format!("{}|{}|{}", f.trigger, f.occurrence, aid),
        };
        Ok((ledger::idempotency_key(&ir.id, &material), material))
    }

    pub(crate) fn action_kind(&self, ir: &ServiceIr, f: &Firing, exec_id: &str, aid: &str, a: &ActionNode, overlay: &Value) -> Result<ActionKind, Abort> {
        let scope = self.scope(f, overlay, exec_id, aid);
        let fail = |e: crate::expr::ExprError| Abort::Failed(format!("{aid}: {e}"));
        match (&a.dialog, &a.wasm_call) {
            (Some(d), _) => Ok(ActionKind::Dialog {
                speech: template::render_text(&d.speech, &scope).map_err(fail)?,
                text: template::render_text(&d.text, &scope).map_err(fail)?,
                priority: d.priority,
            }),
            (_, Some(w)) => Ok(ActionKind::Logic {
                component_ref: w.component_ref.clone(),
                wit_interface: w.wit_interface.clone(),
                function: w.function.clone(),
                args: template::render_value(&Value::Object(w.args_mapping.clone()), &scope).map_err(fail)?,
                fuel_budget: ir.power_budget.wasm_fuel_per_invocation,
            }),
            _ => Err(Abort::Failed(format!("{aid}: `{}` has no host effect", a.ty.as_str()))),
        }
    }

    /// Stage a reversible (or pure) action into the overlay/outbox.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn stage(&mut self, ir: &ServiceIr, f: &Firing, exec_id: &str, aid: &str, a: &ActionNode, key: Option<(String, String)>, st: &mut Staging) -> Result<(), Abort> {
        let render = |s: &Self, v: &Value, overlay: &Value| template::render_value(v, &s.scope(f, overlay, exec_id, aid)).map_err(|e| Abort::Failed(format!("{aid}: {e}")));
        let mut output = Value::Null;
        match a.ty {
            ActionType::StateMutate => {
                for op in a.state_patch.iter().flatten() {
                    let value = op.value.as_ref().map(|v| render(self, v, &st.overlay)).transpose()?;
                    patch::apply(&mut st.overlay, op.op, &op.path, value.clone()).map_err(|e| Abort::Failed(format!("{aid}: {e}")))?;
                    st.deltas.push(json!({ "op": op.op.as_str(), "path": op.path, "value": value }));
                }
            }
            ActionType::UiPatch => {
                let u = a.ui_patch.clone().expect("validated");
                let cap = ir.capabilities.iter().find(|c| c.capability == "ui.surface.render" && c.scope.get("surface_id").is_none_or(|s| s.as_str() == Some(&u.surface_id)));
                self.require_cap(ir, cap.expect("validated"))?;
                self.release_per_invocation(ir, cap.unwrap());
                let value = u.data_model_value.as_ref().map(|v| render(self, v, &st.overlay)).transpose()?;
                st.outbox.push(Staged::Ui { patch: u, value });
            }
            ActionType::BusEmit => {
                let b = a.bus_emit.as_ref().expect("validated");
                let data = render(self, &Value::Object(b.data.clone()), &st.overlay)?;
                st.outbox.push(Staged::Bus { msg_type: b.msg_type.clone(), data });
            }
            ActionType::Logic => {
                let kind = self.action_kind(ir, f, exec_id, aid, a, &st.overlay)?;
                let dkey = format!("{aid}|{}", serde_json::to_string(&kind).unwrap_or_default());
                let cached = a.deduplicate_calls.then(|| self.services[&ir.id].dedup.get(&dkey).cloned()).flatten();
                output = match cached {
                    Some(out) => {
                        self.journal.custom(self.now_ms, "svc.action.deduplicated", json!({ "service": ir.id, "action": aid }));
                        out
                    }
                    None => {
                        let outcome = self.call_host(ir, exec_id, aid, kind, key.as_ref().map(|k| k.0.clone()))?;
                        // The effect happened: undo it on abort even if its cost aborts us.
                        if let Some(c) = &a.compensating_action {
                            st.compensations.push((aid.to_string(), c.clone()));
                        }
                        self.check_outcome(ir, aid, a, &outcome)?;
                        if a.deduplicate_calls {
                            self.services.get_mut(&ir.id).unwrap().dedup.insert(dkey, outcome.output.clone());
                        }
                        outcome.output
                    }
                };
                if let Some(p) = a.wasm_call.as_ref().and_then(|w| w.output_state_pointer.as_ref()) {
                    patch::set(&mut st.overlay, p, output.clone()).map_err(|e| Abort::Failed(format!("{aid}: {e}")))?;
                    st.deltas.push(json!({ "op": "add", "path": p, "value": inline(&output) }));
                }
            }
            ActionType::Dialog | ActionType::SubService => return Err(Abort::Failed(format!("{aid}: cannot be staged"))),
        }
        if let (Some(c), false) = (&a.compensating_action, a.ty == ActionType::Logic) {
            st.compensations.push((aid.to_string(), c.clone()));
        }
        if let (Some((key, material)), true) = (key, a.effect_class != EffectClass::Pure) {
            let row = EffectRow { service_id: ir.id.clone(), key, material, action_id: aid.to_string(), execution_id: exec_id.to_string(), effect_class: a.effect_class, output, at_ms: self.now_ms };
            st.effects.push(row);
        }
        Ok(())
    }

    fn effect_cap<'a>(ir: &'a ServiceIr, kind: &ActionKind) -> Option<&'a CapabilityRequest> {
        let prefs: &[&str] = match kind {
            ActionKind::Dialog { priority: DialogPriority::Critical, .. } => &["audio.playback.alarm", "audio.playback.tts"],
            ActionKind::Dialog { .. } => &["audio.playback.tts", "audio.playback.alarm"],
            _ => &[],
        };
        prefs.iter().find_map(|p| ir.capabilities.iter().find(|c| c.capability == *p))
    }

    /// Perform one effect through the host, powering its demand schemes for
    /// exactly the duration of the call.
    pub(crate) fn call_host(&mut self, ir: &ServiceIr, exec_id: &str, aid: &str, kind: ActionKind, key: Option<String>) -> Result<ActionOutcome, Abort> {
        let cap = Self::effect_cap(ir, &kind);
        if let Some(c) = cap {
            self.require_cap(ir, c)?;
        }
        let is_audio = matches!(kind, ActionKind::Dialog { .. });
        let demand: Vec<String> = ir
            .deps
            .schemes
            .iter()
            .filter(|d| d.lifecycle == Lifecycle::DemandPerAction && d.scheme.starts_with("audio:") == is_audio)
            .map(|d| d.scheme.clone())
            .collect();
        for s in &demand {
            self.power(&ir.id, s, true, format!("{}: {aid} needs it", ir.title));
        }
        let call_id = key.clone().unwrap_or_else(|| format!("{exec_id}/{aid}"));
        let name = match &kind {
            ActionKind::Dialog { .. } => "dialog.speak".to_string(),
            ActionKind::Logic { wit_interface, function, .. } => format!("{wit_interface}.{function}"),
            ActionKind::Compensate { .. } => "compensate".to_string(),
        };
        let now = self.now_ms;
        self.journal.agui(now, AgUiEvent::ToolCallStart { tool_call_id: call_id.clone(), tool_call_name: name, parent_message_id: None });
        self.journal.agui(now, AgUiEvent::ToolCallArgs { tool_call_id: call_id.clone(), delta: serde_json::to_string(&kind).unwrap_or_default() });
        let req = ActionRequest { service_id: ir.id.clone(), action_id: aid.to_string(), execution_id: exec_id.to_string(), idempotency_key: key, now_ms: now, kind };
        let res = self.host.invoke(&req);
        self.journal.agui(now, AgUiEvent::ToolCallEnd { tool_call_id: call_id.clone() });
        let content = match &res {
            Ok(o) => json!({ "ok": inline(&o.output), "fuel_used": o.fuel_used, "network_bytes": o.network_bytes, "elapsed_ms": o.elapsed_ms }),
            Err(e) => json!({ "error": e.0 }),
        };
        self.journal.agui(now, AgUiEvent::ToolCallResult { message_id: format!("{exec_id}/{aid}"), tool_call_id: call_id, content, role: Some("tool".into()) });
        for s in &demand {
            self.power(&ir.id, s, false, format!("{}: {aid} finished", ir.title));
        }
        if let Some(c) = cap {
            self.release_per_invocation(ir, c);
        }
        res.map_err(|e| Abort::Failed(format!("{aid}: host error: {}", e.0)))
    }

    /// Enforce the power budget against what one call cost.
    pub(crate) fn check_outcome(&mut self, ir: &ServiceIr, aid: &str, a: &ActionNode, o: &ActionOutcome) -> Result<(), Abort> {
        let pb = &ir.power_budget;
        let rt = self.services.get_mut(&ir.id).unwrap();
        rt.network_bytes += o.network_bytes;
        let violation = |budget: &str, limit: String, observed: String| Abort::Budget(BudgetViolation { budget: budget.into(), limit, observed });
        if o.fuel_used > pb.wasm_fuel_per_invocation {
            return Err(violation("wasm_fuel_per_invocation", pb.wasm_fuel_per_invocation.to_string(), format!("{} fuel in {aid}", o.fuel_used)));
        }
        if rt.network_bytes > pb.network_egress_bytes_max {
            return Err(violation("network_egress_bytes_max", format!("{} bytes", pb.network_egress_bytes_max), format!("{} bytes after {aid}", rt.network_bytes)));
        }
        if o.wakelock_ms > pb.max_wakelock_sec * 1000 {
            return Err(violation("max_wakelock_sec", format!("{} s", pb.max_wakelock_sec), format!("{} ms in {aid}", o.wakelock_ms)));
        }
        if o.elapsed_ms > a.timeout.total_ms {
            return Err(Abort::Failed(format!("{aid}: timed out ({} ms > total_ms {})", o.elapsed_ms, a.timeout.total_ms)));
        }
        Ok(())
    }

    pub(crate) fn check_memory(&mut self, sid: &str, st: &Staging) -> Result<(), Abort> {
        let outbox: u64 = st
            .outbox
            .iter()
            .map(|s| match s {
                Staged::Ui { patch, value } => size(patch) + value.as_ref().map_or(0, size),
                Staged::Bus { data, .. } => size(data),
            })
            .sum();
        let used = size(&st.overlay) + outbox + st.deferred.iter().map(|d| size(&d.kind)).sum::<u64>();
        let rt = self.services.get_mut(sid).unwrap();
        rt.peak_memory_bytes = rt.peak_memory_bytes.max(used);
        let limit = rt.ir.power_budget.memory_limit_mb * MIB;
        if used > limit {
            return Err(Abort::Budget(BudgetViolation { budget: "memory_limit_mb".into(), limit: format!("{limit} bytes"), observed: format!("{used} bytes staged") }));
        }
        Ok(())
    }

    /// Ask the broker (once per standing grant, every time per invocation).
    pub(crate) fn require_cap(&mut self, ir: &ServiceIr, c: &CapabilityRequest) -> Result<(), Abort> {
        if c.lifetime != CapLifetime::PerInvocation && self.services[&ir.id].grants.contains(&c.cap_id) {
            return Ok(());
        }
        match self.broker.grant(&ir.id, c, self.now_ms) {
            Ok(()) => {
                if c.lifetime != CapLifetime::PerInvocation {
                    self.services.get_mut(&ir.id).unwrap().grants.insert(c.cap_id.clone());
                }
                self.note_grant(&ir.id, c);
                Ok(())
            }
            Err(reason) => {
                let v = json!({ "service": ir.id, "capability": c.capability, "reason": reason });
                self.journal.custom(self.now_ms, "svc.grant.denied", v);
                Err(Abort::Denied(format!("capability `{}` refused: {reason}", c.capability)))
            }
        }
    }

    pub(crate) fn note_grant(&mut self, sid: &str, c: &CapabilityRequest) {
        let ev = Op6Event::GrantGranted {
            service: sid.to_string(),
            capability: c.capability.clone(),
            scope: Some(Value::Object(c.scope.clone()).to_string()),
            expires_us: Some(self.journal_now_us() + c.max_duration_sec * 1_000_000),
        };
        self.journal.op6(self.now_ms, ev);
    }

    fn journal_now_us(&self) -> u64 {
        self.journal.ring().last().map_or(0, |e| e.ts_us)
    }

    pub(crate) fn release_per_invocation(&mut self, ir: &ServiceIr, c: &CapabilityRequest) {
        if c.lifetime == CapLifetime::PerInvocation {
            let ev = Op6Event::GrantRevoked { service: ir.id.clone(), capability: c.capability.clone(), reason: "per-invocation grant ended".into() };
            self.journal.op6(self.now_ms, ev);
        }
    }

    /// Run recorded compensations in reverse order (LIFO) after an abort.
    pub(crate) fn compensate(&mut self, ir: &ServiceIr, exec_id: &str, comps: &[(String, String)]) {
        for (aid, comp) in comps.iter().rev() {
            let kind = ActionKind::Compensate { action_id: aid.clone(), compensating_action: comp.clone() };
            let req = ActionRequest { service_id: ir.id.clone(), action_id: comp.clone(), execution_id: exec_id.to_string(), idempotency_key: None, now_ms: self.now_ms, kind };
            let res = self.host.invoke(&req);
            let v = json!({ "service": ir.id, "action": aid, "compensating_action": comp, "ok": res.is_ok() });
            self.journal.custom(self.now_ms, "svc.action.compensated", v);
        }
    }

    /// Sliding 60 s window of non-user wakeups (`max_wakeups_per_minute`,
    /// rounded up: 0.1/min still allows one wakeup in any minute).
    pub(crate) fn admit_wakeup(&mut self, sid: &str) -> Option<BudgetViolation> {
        let now = self.now_ms;
        let rt = self.services.get_mut(sid).unwrap();
        let max = rt.ir.power_budget.max_wakeups_per_minute;
        while rt.wakeups.front().is_some_and(|&t| t + 60_000 <= now) {
            rt.wakeups.pop_front();
        }
        let allowed = max.ceil() as usize;
        if rt.wakeups.len() + 1 > allowed {
            return Some(BudgetViolation {
                budget: "max_wakeups_per_minute".into(),
                limit: format!("{max} per 60 s"),
                observed: format!("{} wakeups in the 60 s ending at {now} ms", rt.wakeups.len() + 1),
            });
        }
        rt.wakeups.push_back(now);
        None
    }
}
