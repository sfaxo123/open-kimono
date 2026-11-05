//! Resource budgets (VISION §8, SERVICE-IR.md §7): exceeding a
//! `power_budget` limit stops the service, says why (journal + bus), powers
//! its schemes down, revokes its grants, and drops every later trigger.
//! Timeouts and capability refusals fail the run but do not stop the service.

mod common;

use common::*;
use serde_json::{json, Map, Value};
use svcd::sim::{run_test, svcd_for};
use svcd::{ActionOutcome, Config, DenyCaps, InstallError, ServiceStatus, SimHost, Svcd};
use trace_events::{AgUiEvent, Op6Event, Payload};

const CHIME: &str = include_str!("fixtures/svc.test.chime.json");

fn chime(host: SimHost) -> Svcd<SimHost> {
    let mut s = Svcd::new(host, Config::default());
    s.install(svcd::load(CHIME).unwrap()).unwrap();
    s
}

fn ok(o: ActionOutcome) -> Result<ActionOutcome, svcd::HostError> {
    Ok(o)
}

#[track_caller]
fn assert_stopped_for(s: &Svcd<SimHost>, sid: &str, budget: &str) {
    match &s.service(sid).unwrap().status {
        ServiceStatus::Stopped { violation: Some(v), .. } => assert_eq!(v.budget, budget),
        other => panic!("expected a {budget} stop, got {other:?}"),
    }
    let bus = s.bus().iter().find(|m| m.msg_type == "svc.budget.exceeded").expect("announced on the bus");
    assert_eq!(bus.data["budget"], json!(budget));
    let stopped = s.journal().iter().find_map(|e| match &e.payload {
        Payload::Op6(Op6Event::ServiceStopped { service, reason, .. }) if service == sid => Some(reason.clone()),
        _ => None,
    });
    assert!(stopped.expect("ServiceStopped journaled").contains(budget));
    assert!(s.service(sid).unwrap().active.is_empty(), "schemes powered down");
    assert!(s.service(sid).unwrap().grants.is_empty(), "grants revoked");
}

fn sample(pct: f64) -> Map<String, Value> {
    json!({ "scheme": "sys:power/battery", "capacity_pct": pct, "charging": false }).as_object().unwrap().clone()
}

#[test]
fn wakeup_storm_stops_the_battery_watcher() {
    // A level (not edge) trigger with no guard: every sample below 20% fires.
    let mut ir = json(BATTERY);
    let t = &mut ir["triggers"][0];
    t["sensor_threshold"]["edge_only"] = json!(false);
    t["debounce_ms"] = json!(0);
    t["cooldown_ms"] = json!(0);
    t.as_object_mut().unwrap().remove("condition_cel");
    let ir = svcd::validate::load_value(ir).unwrap();
    let mut s = svcd_for(&ir.tests[0], SimHost::new());
    s.install(ir.clone()).unwrap();

    s.advance_to(10_000);
    s.sensor_sample("sys:power/battery", sample(19.0));
    s.advance_to(40_000);
    s.sensor_sample("sys:power/battery", sample(18.0));
    assert_stopped_for(&s, &ir.id, "max_wakeups_per_minute");
    assert_eq!(s.host().spoken().len(), 1, "the second wakeup never ran");

    s.advance_to(400_000);
    s.sensor_sample("sys:power/battery", sample(10.0));
    assert_eq!(s.host().spoken().len(), 1, "a stopped service is unsubscribed from its sensors");
    let dropped = s.journal().iter().filter(|e| matches!(&e.payload, Payload::AgUi(AgUiEvent::Custom { name, .. }) if name == "svc.trigger.dropped")).count();
    assert_eq!(dropped, 1, "the refused wakeup is journaled with a reason");
}

#[test]
fn memory_budget_stops_before_anything_commits() {
    let big = "x".repeat(1_100_000);
    let host = SimHost::new().script("act_reserve", ok(ActionOutcome { output: json!(big), ..Default::default() }));
    let mut s = chime(host);
    s.utterance("ring the chime", Some("u1"));
    assert_stopped_for(&s, "svc.test.chime", "memory_limit_mb");
    assert_eq!(s.state("svc.test.chime").unwrap(), &json!({ "count": 0 }));
    assert!(s.host().spoken().is_empty());
    assert_eq!(s.ledger().runs().next().unwrap().attempts, 1, "budget stops are never retried");
}

#[test]
fn network_egress_on_an_offline_service_stops_it() {
    let host = SimHost::new().script("act_report", ok(ActionOutcome { network_bytes: 512, ..Default::default() }));
    let mut s = chime(host);
    s.utterance("ring the chime", Some("u1"));
    assert_stopped_for(&s, "svc.test.chime", "network_egress_bytes_max");
    assert_eq!(s.service("svc.test.chime").unwrap().network_bytes, 512);
    let report = s.ledger().effects().find(|e| e.action_id == "act_report");
    assert!(report.is_some(), "the irreversible effect happened, so it is ledgered and never repeated");
    assert!(s.bus().iter().any(|m| m.msg_type == "ovos.intent.handler.error"));
}

#[test]
fn wasm_fuel_over_budget_stops_the_service() {
    let host = SimHost::new().script("act_reserve", ok(ActionOutcome { fuel_used: 10_001, ..Default::default() }));
    let mut s = chime(host);
    s.utterance("ring the chime", None);
    assert_stopped_for(&s, "svc.test.chime", "wasm_fuel_per_invocation");
    assert!(s.host().calls.iter().any(|c| matches!(c.kind, svcd::ActionKind::Compensate { .. })), "reserved slot released");
}

#[test]
fn wake_lock_over_budget_stops_the_service() {
    let host = SimHost::new().script("act_chime", ok(ActionOutcome { wakelock_ms: 2_500, ..Default::default() }));
    let mut s = chime(host);
    s.utterance("ring the chime", None);
    assert_stopped_for(&s, "svc.test.chime", "max_wakelock_sec");
}

#[test]
fn a_timeout_fails_the_run_but_keeps_the_service() {
    let slow = || ok(ActionOutcome { elapsed_ms: 101, ..Default::default() });
    let host = SimHost::new().script("act_reserve", slow()).script("act_reserve", slow()).script("act_reserve", slow());
    let mut s = chime(host);
    s.utterance("ring the chime", None);
    assert_eq!(s.service("svc.test.chime").unwrap().status, ServiceStatus::Running);
    let run = s.ledger().runs().next().unwrap();
    assert_eq!(run.attempts, 3);
    assert!(run.error.as_deref().unwrap().contains("timed out"));
    s.utterance("ring the chime", None);
    assert_eq!(s.host().spoken().len(), 1, "next utterance works");
}

#[test]
fn refused_capability_fails_honestly_without_stopping() {
    let ir = svcd::load(TIMER).unwrap();
    let (report, s) = run_test(&ir, &ir.tests[0], SimHost::new(), DenyCaps(vec!["audio.playback.alarm".into()]));
    assert!(!report.passed());
    assert!(s.host().spoken().is_empty(), "no speech without the audio grant");
    let errors: Vec<&Value> = s.bus().iter().filter(|m| m.msg_type == "ovos.intent.handler.error").map(|m| &m.data["error"]).collect();
    assert_eq!(errors.len(), 2);
    assert!(errors.iter().all(|e| e.as_str().unwrap().contains("capability `audio.playback.alarm` refused")));
    assert_eq!(s.ledger().runs().map(|r| r.attempts).collect::<Vec<_>>(), vec![1, 1], "refusals are not retried");
    assert_eq!(s.service(&ir.id).unwrap().status, ServiceStatus::Running);
    assert_eq!(s.state(&ir.id).unwrap()["status"], json!("idle"), "the alarm run still disarmed the timer");
}

#[test]
fn refused_standing_grant_blocks_install() {
    let ir = svcd::load(BATTERY).unwrap();
    let mut s = Svcd::new(SimHost::new(), Config::default()).with_broker(DenyCaps(vec!["power.battery.read".into()]));
    match s.install(ir) {
        Err(InstallError::Denied { capability, .. }) => assert_eq!(capability, "power.battery.read"),
        other => panic!("expected Denied, got {other:?}"),
    }
    assert!(s.list().is_empty());
}

#[test]
fn stopping_a_service_cancels_its_pending_timer() {
    let ir = svcd::load(TIMER).unwrap();
    let mut s = svcd_for(&ir.tests[0], SimHost::new());
    s.install(ir.clone()).unwrap();
    s.utterance("timer for 5 minutes", None);
    assert_eq!(s.pending_timers(), vec![300_000]);
    s.stop(Some(&ir.id), "user said stop the timer");
    assert!(s.pending_timers().is_empty());
    assert!(s.surfaces().next().is_none(), "its surface is gone");
    s.advance_to(1_000_000);
    assert_eq!(s.host().spoken(), vec!["Timer set for 5 minutes."]);
}

#[test]
fn simulator_budget_assertion_catches_a_memory_regression() {
    let mut ir = json(TIMER);
    ir["tests"][0]["assertions"]["budget_assertions"]["max_memory_mb_used"] = json!(0.0001);
    let ir = svcd::validate::load_value(ir).unwrap();
    let r = &svcd::sim::simulate(&ir)[0];
    assert!(r.failures.iter().any(|f| f.contains("peak memory")), "{:?}", r.failures);
}
