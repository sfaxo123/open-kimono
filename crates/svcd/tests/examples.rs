//! The two D1 worked examples, end to end in the simulator: their own
//! embedded tests pass, plus deeper checks on what happened along the way.

mod common;

use common::*;
use serde_json::{json, Value};
use svcd::sim::{self, run_test};
use svcd::{GrantAll, ServiceStatus, SimHost};
use trace_events::{AgUiEvent, Op6Event, Payload, PowerState};

fn customs<'a>(events: &'a [trace_events::Event], name: &'a str) -> impl Iterator<Item = &'a Value> + 'a {
    events.iter().filter_map(move |e| match &e.payload {
        Payload::AgUi(AgUiEvent::Custom { name: n, value }) if n == name => Some(value),
        _ => None,
    })
}

#[test]
fn timer_example_passes_its_embedded_test() {
    let ir = svcd::load(TIMER).unwrap();
    let reports = sim::simulate(&ir);
    assert_eq!(reports.len(), 1);
    assert!(reports[0].passed(), "{:#?}", reports[0].failures);
}

#[test]
fn battery_example_passes_its_embedded_test() {
    let ir = svcd::load(BATTERY).unwrap();
    let reports = sim::simulate(&ir);
    assert!(reports[0].passed(), "{:#?}", reports[0].failures);
}

#[test]
fn timer_lifecycle_in_detail() {
    let ir = svcd::load(TIMER).unwrap();
    let (report, svcd) = run_test(&ir, &ir.tests[0], SimHost::new(), GrantAll);
    assert!(report.passed(), "{:#?}", report.failures);
    assert_eq!(svcd.host().spoken(), vec!["Timer set for 5 minutes.", "Time is up for Timer."]);
    let alarm = svcd.host().calls.iter().find(|c| c.action_id == "act_fire_alarm").unwrap();
    assert_eq!(alarm.now_ms, 300_000, "alarm fires exactly at the virtual deadline");

    let types: Vec<&str> = svcd.bus().iter().map(|m| m.msg_type.as_str()).collect();
    assert_eq!(
        types,
        ["ovos.intent.handler.start", "speak", "ovos.intent.handler.complete", "ovos.intent.handler.start", "speak", "ovos.intent.handler.complete"]
    );

    let armed: Vec<&Value> = customs(&report.journal, "svc.timer.armed").collect();
    assert_eq!(armed.len(), 1);
    assert_eq!(armed[0]["due_ms"], json!(300_000));

    // time:monotonic powered only while the countdown was pending; audio only per speech.
    let power: Vec<(String, PowerState, u64)> = report
        .journal
        .iter()
        .filter_map(|e| match &e.payload {
            Payload::Op6(Op6Event::PowerChange { component, state, .. }) => Some((component.clone(), *state, e.ts_us)),
            _ => None,
        })
        .collect();
    let comps: Vec<(&str, PowerState)> = power.iter().map(|(c, s, _)| (c.as_str(), *s)).collect();
    assert_eq!(
        comps,
        [
            ("audio:wyoming/tts", PowerState::On),
            ("audio:wyoming/tts", PowerState::Off),
            ("time:monotonic", PowerState::On),
            ("audio:wyoming/tts", PowerState::On),
            ("audio:wyoming/tts", PowerState::Off),
            ("time:monotonic", PowerState::Off),
        ]
    );
    assert!(report.active_schemes.is_empty());

    let surface = report.surfaces.iter().find(|s| s.surface_id == "countdown-timer-surface").unwrap();
    assert_eq!(surface.data_model["minutes"], json!(5), "data model bound to state at arm time");
    assert_eq!(surface.components.len(), 14, "components from ui.initial_messages");
}

#[test]
fn every_run_is_journaled_with_zero_llm_tokens() {
    let ir = svcd::load(TIMER).unwrap();
    let report = &sim::simulate(&ir)[0];
    let finished: Vec<&Value> = report
        .journal
        .iter()
        .filter_map(|e| match &e.payload {
            Payload::AgUi(AgUiEvent::RunFinished { result, usage, .. }) => {
                assert!(usage.is_empty());
                result.as_ref()
            }
            _ => None,
        })
        .collect();
    assert_eq!(finished.len(), 2);
    assert!(finished.iter().all(|r| r["llm_tokens"] == json!(0)));
    assert!(!report.journal.iter().any(|e| matches!(e.payload, Payload::GenAi(_))));
    let seqs: Vec<u64> = report.journal.iter().map(|e| e.seq).collect();
    assert!(seqs.windows(2).all(|w| w[1] == w[0] + 1), "gapless journal");
    assert!(report.journal.iter().all(|e| e.ts_us >= 1_792_864_800_000_000), "wall clock anchored at wall_iso");
    // Producer identity on every record (the trace-events `service` field,
    // added by B1 after B4 branched) — same contract as voice-bridge/a2ui-render.
    assert!(report.journal.iter().all(|e| e.service.as_deref() == Some(svcd::journal::SERVICE_NAME)), "every record stamped svcd");
}

#[test]
fn battery_hysteresis_in_detail() {
    let ir = svcd::load(BATTERY).unwrap();
    let (report, svcd) = run_test(&ir, &ir.tests[0], SimHost::new(), GrantAll);
    assert!(report.passed(), "{:#?}", report.failures);
    assert_eq!(svcd.host().spoken(), vec!["Battery is down to 19 percent.", "Battery is down to 19 percent."]);
    let at: Vec<u64> = svcd.host().calls.iter().map(|c| c.now_ms).collect();
    assert_eq!(at, vec![15_000, 505_000], "fires after the 5 s debounce, once per discharge cycle");
    assert_eq!(report.status, Some(ServiceStatus::Running));
    assert_eq!(report.active_schemes, vec!["sys:power/battery"]);
    let banner = report.surfaces.iter().find(|s| s.surface_id == "battery-low-banner").unwrap();
    assert_eq!(banner.openui_statements.len(), 4);
    // Each discharge cycle has its own latch key; nothing was replayed.
    let mut latches: Vec<&str> = svcd.ledger().effects().filter(|e| e.action_id == "act_mark_cycle_notified").map(|e| e.material.as_str()).collect();
    latches.sort();
    assert_eq!(latches, ["svc.power.battery_low_watch:cycle_1:latch", "svc.power.battery_low_watch:cycle_2:latch"]);
}

#[test]
fn battery_noise_below_threshold_does_not_respam() {
    let ir = svcd::load(BATTERY).unwrap();
    let mut test = ir.tests[0].clone();
    test.events.truncate(2);
    for (i, pct) in [19.5, 18.0, 19.9, 17.0].iter().enumerate() {
        let mut ev = test.events[1].clone();
        ev.at_monotonic_ms = 90_000 + i as u64 * 70_000;
        ev.payload.insert("capacity_pct".into(), json!(pct));
        test.events.push(ev);
    }
    let (_, svcd) = run_test(&ir, &test, SimHost::new(), GrantAll);
    assert_eq!(svcd.host().spoken().len(), 1, "one alert per discharge cycle");
}

#[test]
fn a_dip_shorter_than_debounce_never_alerts() {
    let ir = svcd::load(BATTERY).unwrap();
    let mut test = ir.tests[0].clone();
    let mut back_up = test.events[0].clone();
    back_up.at_monotonic_ms = 12_000;
    back_up.payload.insert("capacity_pct".into(), json!(21.0));
    test.events = vec![test.events[0].clone(), back_up];
    let (report, svcd) = run_test(&ir, &test, SimHost::new(), GrantAll);
    assert!(svcd.host().spoken().is_empty());
    assert_eq!(customs(&report.journal, "svc.trigger.suppressed").count(), 0, "cancelled before the debounce timer fired");
    assert_eq!(svcd.state(&ir.id).unwrap()["notified_for_cycle"], json!(false));
}

#[test]
fn ui_cancel_disarms_the_timer_and_powers_down() {
    let ir = svcd::load(TIMER).unwrap();
    let mut svcd = sim::svcd_for(&ir.tests[0], SimHost::new());
    svcd.install(ir.clone()).unwrap();
    svcd.utterance("set a timer for 0 hours and 2 minutes and 30 seconds", None).expect("matched");
    assert_eq!(svcd.pending_timers(), vec![150_000]);
    svcd.advance_to(60_000);
    assert!(svcd.ui_action("countdown-timer-surface", "cancel_timer", Some("btn_cancel")));
    assert!(svcd.pending_timers().is_empty());
    svcd.advance_to(600_000);
    assert_eq!(svcd.host().spoken(), vec!["Timer set for 2 minutes."], "no alarm after cancel");
    assert!(svcd.service(&ir.id).unwrap().active.is_empty());
}

#[test]
fn unmatched_utterance_falls_back_without_running_anything() {
    let ir = svcd::load(TIMER).unwrap();
    let mut svcd = sim::svcd_for(&ir.tests[0], SimHost::new());
    svcd.install(ir).unwrap();
    assert_eq!(svcd.utterance("what's the weather like", None), None);
    assert_eq!(svcd.bus().last().unwrap().msg_type, "ovos.intent.fallback");
    assert!(svcd.host().calls.is_empty());
    assert_eq!(svcd.ledger().runs().count(), 0);
}

#[test]
fn overlong_utterance_is_not_template_matched() {
    let ir = svcd::load(TIMER).unwrap();
    let mut svcd = sim::svcd_for(&ir.tests[0], SimHost::new());
    svcd.install(ir).unwrap();
    let text = format!("set a 5 minute timer named {}", "tea ".repeat(300));
    let t0 = std::time::Instant::now();
    assert_eq!(svcd.utterance(&text, None), None);
    assert!(t0.elapsed().as_millis() < 100, "bounded work on hostile input");
    let fb = svcd.bus().last().unwrap();
    assert_eq!(fb.msg_type, "ovos.intent.fallback");
    assert!(fb.data["reason"].as_str().unwrap().contains("1024"));
}

#[test]
fn admission_gate_runs_the_tests_before_installing() {
    let ir = svcd::load(TIMER).unwrap();
    let mut live = svcd::Svcd::new(SimHost::new(), svcd::Config::default());
    let reports = sim::admit(&mut live, ir.clone()).expect("admitted");
    assert!(reports.iter().all(|r| r.passed()));
    assert_eq!(live.list().len(), 1);

    let mut broken = ir;
    broken.id = "svc.time.broken_timer".into();
    broken.tests[0].assertions.final_state_subset.insert("status".into(), json!("running"));
    let mut live2 = svcd::Svcd::new(SimHost::new(), svcd::Config::default());
    match sim::admit(&mut live2, broken) {
        Err(sim::AdmitError::TestsFailed(r)) => assert!(r[0].failures[0].contains("final state")),
        other => panic!("expected TestsFailed, got {other:?}"),
    }
    assert!(live2.list().is_empty(), "a failing IR never reaches the live manifest");
}
