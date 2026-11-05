//! Power-by-capability-lifetime (SERVICE-IR.md §7 rules 2, 5, 8): a scheme
//! is powered only while the grant that justifies it is alive, timers are
//! programmed from committed state (zero polling), and every switch is
//! journaled with the service's `power_announcement`.

use serde_json::{json, Value};
use trace_events::{Op6Event, PowerState};

use super::run::Firing;
use super::{BudgetViolation, ServiceStatus, Svcd, TimerEntry};
use crate::expr;
use crate::host::ActionHost;
use crate::ir::{CapLifetime, Lifecycle, ServiceIr, TimerMode, TriggerKind};
use crate::sensor::satisfied;
use crate::template;

/// Which capabilities justify keeping a scheme powered.
fn scheme_caps(scheme: &str) -> &'static [&'static str] {
    let head = scheme.split(':').next().unwrap_or("");
    match head {
        "time" => &["timer.monotonic.schedule"],
        "audio" => &["audio.playback.tts", "audio.playback.alarm", "audio.capture"],
        "sys" | "power" if scheme.contains("power") => &["power.battery.read"],
        "disk" | "storage" => &["storage.maps.readonly"],
        "location" => &["location.gnss.read"],
        "net" => &["network.connect"],
        _ => &[],
    }
}

pub(crate) fn default_params(ir: &ServiceIr) -> serde_json::Map<String, Value> {
    ir.parameters.iter().filter_map(|p| Some((p.key.clone(), p.default.clone()?))).collect()
}

impl<H: ActionHost> Svcd<H> {
    pub(crate) fn power(&mut self, sid: &str, scheme: &str, on: bool, reason: String) {
        let Some(rt) = self.services.get_mut(sid) else { return };
        let changed = if on { rt.active.insert(scheme.to_string()) } else { rt.active.remove(scheme) };
        if changed {
            let state = if on { PowerState::On } else { PowerState::Off };
            self.journal.op6(self.now_ms, Op6Event::PowerChange { component: scheme.to_string(), state, reason });
        }
    }

    fn running(&self, sid: &str) -> bool {
        self.services.get(sid).is_some_and(|rt| rt.status == ServiceStatus::Running)
    }

    /// Re-derive timers and powered schemes from committed state.
    pub(crate) fn after_execution(&mut self, sid: &str) {
        if !self.running(sid) {
            return;
        }
        let ir = self.services[sid].ir.clone();
        for t in ir.triggers.iter().filter(|t| t.kind == TriggerKind::Timer) {
            if t.timer_schedule.as_ref().is_none_or(|ts| ts.mode != TimerMode::Countdown) {
                continue;
            }
            let pending = self.services[sid].timers.contains_key(&t.id);
            let wanted = match &t.condition_cel {
                None => true,
                Some(c) => {
                    let f = self.timer_firing(&ir, &t.id, 0);
                    let scope = super::run::ExecScope { state: &self.services[sid].state, firing: &f, session: &self.session, execution_id: "", action_id: None, now_ms: self.now_ms };
                    expr::parse(c).and_then(|e| e.eval_bool(&scope)).unwrap_or(false)
                }
            };
            if wanted && !pending {
                self.schedule_trigger(sid, &t.id);
            } else if !wanted && pending {
                let key = self.services.get_mut(sid).unwrap().timers.remove(&t.id).unwrap();
                self.timers.remove(&key);
                self.journal.custom(self.now_ms, "svc.timer.cancelled", json!({ "service": sid, "trigger": t.id, "due_ms": key.0 }));
            }
        }
        self.recompute_schemes(sid);
    }

    fn recompute_schemes(&mut self, sid: &str) {
        let ir = self.services[sid].ir.clone();
        for dep in &ir.deps.schemes {
            let wanted = match dep.lifecycle {
                Lifecycle::DemandPerAction => continue,
                Lifecycle::AlwaysResident => true,
                Lifecycle::WhileSurfaceVisible => self.has_surface(sid),
                Lifecycle::WhileGrantAlive => self.grant_alive(sid, &ir, &dep.scheme),
            };
            if wanted != self.services[sid].active.contains(&dep.scheme) {
                let reason = if wanted { ir.power_budget.power_announcement.clone() } else { format!("{}: no longer needed", ir.title) };
                self.power(sid, &dep.scheme, wanted, reason);
            }
        }
    }

    fn grant_alive(&self, sid: &str, ir: &ServiceIr, scheme: &str) -> bool {
        let caps = scheme_caps(scheme);
        let rt = &self.services[sid];
        ir.capabilities.iter().filter(|c| caps.contains(&c.capability.as_str())).any(|c| match c.lifetime {
            CapLifetime::UntilDeadline => !rt.timers.is_empty(),
            CapLifetime::StandingWatch => true,
            CapLifetime::WhileVisible => self.has_surface(sid),
            CapLifetime::PerInvocation => false,
        })
    }

    pub(crate) fn timer_firing(&self, ir: &ServiceIr, trigger: &str, due: u64) -> Firing {
        Firing {
            service: ir.id.clone(),
            trigger: trigger.to_string(),
            payload: json!({ "scheduled_instant_ms": due }),
            occurrence: format!("t{due}"),
            utterance_id: None,
            params: default_params(ir),
            user: false,
        }
    }

    /// Program a countdown/interval deadline from committed state.
    pub(crate) fn schedule_trigger(&mut self, sid: &str, trigger: &str) {
        let ir = self.services[sid].ir.clone();
        let t = ir.triggers.iter().find(|t| t.id == trigger).expect("trigger exists");
        let ts = t.timer_schedule.as_ref().expect("timer trigger");
        let f = self.timer_firing(&ir, trigger, 0);
        let scope = super::run::ExecScope { state: &self.services[sid].state, firing: &f, session: &self.session, execution_id: "", action_id: None, now_ms: self.now_ms };
        let secs = template::parse(ts.duration_sec_expr.as_deref().unwrap_or("")).and_then(|tpl| tpl.render(&scope));
        let secs = match secs {
            Ok(v) if v.as_f64().is_some_and(|s| s > 0.0 && s.is_finite()) => v.as_f64().unwrap(),
            other => {
                let why = format!("duration_sec_expr gave {other:?}; timer not armed");
                self.journal.custom(self.now_ms, "svc.timer.error", json!({ "service": sid, "trigger": trigger, "error": why }));
                return;
            }
        };
        let cap = ir.capabilities.iter().find(|c| c.capability == "timer.monotonic.schedule").expect("validated");
        if self.require_cap(&ir, cap).is_err() {
            return;
        }
        let due = self.now_ms + (secs * 1000.0).round() as u64;
        let key = self.add_timer(due, TimerEntry::Schedule { service: sid.into(), trigger: trigger.into() });
        self.services.get_mut(sid).unwrap().timers.insert(trigger.to_string(), key);
        let v = json!({ "service": sid, "trigger": trigger, "due_ms": due, "wake_from_suspend": ts.wake_from_suspend });
        self.journal.custom(self.now_ms, "svc.timer.armed", v);
    }

    pub(crate) fn timer_fired(&mut self, sid: &str, trigger: &str, due: u64) {
        let Some(rt) = self.services.get_mut(sid) else { return };
        rt.timers.remove(trigger);
        let ir = rt.ir.clone();
        self.journal.custom(self.now_ms, "svc.timer.fired", json!({ "service": sid, "trigger": trigger, "due_ms": due }));
        let interval = ir.triggers.iter().any(|t| t.id == trigger && t.timer_schedule.as_ref().is_some_and(|ts| ts.mode == TimerMode::Interval));
        if interval && self.running(sid) {
            // Advance before dispatch: the next deadline exists before this run can fail.
            self.schedule_trigger(sid, trigger);
        }
        let f = self.timer_firing(&ir, trigger, due);
        self.fire(f);
    }

    pub(crate) fn debounce_fired(&mut self, sid: &str, trigger: &str) {
        let Some(rt) = self.services.get_mut(sid) else { return };
        let Some(track) = rt.sensors.get_mut(trigger) else { return };
        if !track.pending {
            return;
        }
        track.pending = false;
        let ir = rt.ir.clone();
        let st = ir.triggers.iter().find(|t| t.id == trigger).and_then(|t| t.sensor_threshold.clone()).expect("sensor trigger");
        let sample = self.sensors.get(&st.scheme_path).cloned().unwrap_or_default();
        let holds = sample.get(&st.metric).and_then(Value::as_f64).is_some_and(|v| satisfied(st.operator, v, st.threshold));
        if !holds {
            self.services.get_mut(sid).unwrap().sensors.get_mut(trigger).unwrap().armed = true;
            self.journal.custom(self.now_ms, "svc.trigger.suppressed", json!({ "service": sid, "trigger": trigger, "reason": "crossing did not hold for debounce_ms" }));
            return;
        }
        let f = Firing {
            service: sid.into(),
            trigger: trigger.into(),
            payload: Value::Object(sample),
            occurrence: format!("s{}", self.now_ms),
            utterance_id: None,
            params: default_params(&ir),
            user: false,
        };
        self.fire(f);
    }

    /// Stop a service: cancel timers, revoke grants, power down its schemes,
    /// drop its surfaces, and say why (journal + bus for budget stops).
    pub fn stop_service(&mut self, sid: &str, reason: &str, violation: Option<BudgetViolation>) {
        if !self.running(sid) {
            return;
        }
        let now = self.now_ms;
        let rt = self.services.get_mut(sid).unwrap();
        rt.status = ServiceStatus::Stopped { reason: reason.to_string(), violation: violation.clone() };
        rt.timers.clear();
        let ir = rt.ir.clone();
        let grants: Vec<String> = std::mem::take(&mut rt.grants).into_iter().collect();
        let active: Vec<String> = rt.active.iter().cloned().collect();
        self.timers.retain(|_, e| !matches!(e, TimerEntry::Schedule { service, .. } | TimerEntry::Debounce { service, .. } if service == sid));
        let full = match &violation {
            Some(v) => format!("{reason}: {} limit {}, observed {}", v.budget, v.limit, v.observed),
            None => reason.to_string(),
        };
        for cap_id in grants {
            let capability = ir.capabilities.iter().find(|c| c.cap_id == cap_id).map_or(cap_id.clone(), |c| c.capability.clone());
            self.journal.op6(now, Op6Event::GrantRevoked { service: sid.into(), capability, reason: format!("service stopped: {full}") });
        }
        for s in active {
            self.power(sid, &s, false, format!("{} stopped: {full}", ir.title));
        }
        self.drop_surfaces(sid);
        self.journal.op6(now, Op6Event::ServiceStopped { service: sid.into(), generation: ir.generation, reason: full });
        if let Some(v) = violation {
            self.emit_bus("svc.budget.exceeded", json!({ "service": sid, "budget": v.budget, "limit": v.limit, "observed": v.observed }));
        }
    }
}
