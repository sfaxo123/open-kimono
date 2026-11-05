//! Transaction model: a non-idempotent (committed_irreversible) effect is
//! performed at most once per idempotency key — across retries, redelivery,
//! duplicate delivery and a restart — and an abort before commit leaves no
//! trace in state, bus or ledger except the compensation.

mod common;

use common::*;
use serde_json::{json, Value};
use svcd::ledger::LedgerError;
use svcd::{ActionKind, ActionOutcome, Config, ExecStatus, HostError, Ledger, SimHost, Svcd};
use trace_events::{AgUiEvent, Payload};

const CHIME: &str = include_str!("fixtures/svc.test.chime.json");

fn live(host: SimHost) -> Svcd<SimHost> {
    let mut s = Svcd::new(host, Config::default());
    s.install(svcd::load(CHIME).expect("chime IR")).expect("installed");
    s
}

fn fail(msg: &str) -> Result<ActionOutcome, HostError> {
    Err(HostError(msg.into()))
}

fn speaks(s: &Svcd<SimHost>) -> usize {
    s.bus().iter().filter(|m| m.msg_type == "speak").count()
}

fn custom_count(s: &Svcd<SimHost>, name: &str) -> usize {
    s.journal().iter().filter(|e| matches!(&e.payload, Payload::AgUi(AgUiEvent::Custom { name: n, .. }) if n == name)).count()
}

#[test]
fn chime_fixture_passes_its_own_simulation() {
    let ir = svcd::load(CHIME).unwrap();
    let r = &svcd::sim::simulate(&ir)[0];
    assert!(r.passed(), "{:#?}", r.failures);
}

#[test]
fn retry_after_a_late_failure_never_repeats_the_spoken_chime() {
    let host = SimHost::new().script("act_report", fail("sink offline"));
    let mut s = live(host);
    s.utterance("ring the chime", Some("u1")).expect("matched");

    assert_eq!(s.host().performed("act_chime"), 1, "speech happened exactly once");
    assert_eq!(s.host().attempts.iter().filter(|c| c.action_id == "act_report").count(), 2, "report retried");
    assert_eq!(s.host().performed("act_report"), 1);
    assert_eq!(speaks(&s), 1);
    assert_eq!(s.state("svc.test.chime").unwrap()["count"], json!(1), "committed patch not re-applied on retry");
    let run = s.ledger().runs().next().unwrap();
    assert_eq!((run.status, run.attempts), (ExecStatus::Completed, 2));
    assert!(custom_count(&s, "svc.action.replayed") >= 2, "count + chime replayed from the ledger");
}

#[test]
fn redelivery_after_exhausted_retries_resumes_without_repeating() {
    let host = SimHost::new().script("act_report", fail("down")).script("act_report", fail("down")).script("act_report", fail("down"));
    let mut s = live(host);
    s.utterance("ring the chime", Some("u1"));
    let first = s.ledger().runs().next().unwrap().clone();
    assert_eq!((first.status, first.attempts), (ExecStatus::Failed, 3));
    assert_eq!(s.bus().iter().filter(|m| m.msg_type == "ovos.intent.handler.error").count(), 1);

    // Same utterance delivered again (e.g. after the sink came back).
    s.utterance("ring the chime", Some("u1"));
    let runs: Vec<_> = s.ledger().runs().cloned().collect();
    assert_eq!(runs.len(), 2);
    assert!(runs[1].run_id.ends_with("/2") && runs[1].status == ExecStatus::Completed);
    assert_eq!(s.host().performed("act_chime"), 1, "still exactly one chime");
    assert_eq!(s.host().performed("act_report"), 1);
    assert_eq!(s.state("svc.test.chime").unwrap()["count"], json!(1));
}

#[test]
fn duplicate_delivery_of_a_completed_occurrence_is_a_noop() {
    let mut s = live(SimHost::new());
    s.utterance("ring the chime", Some("u1"));
    s.utterance("ring the chime", Some("u1"));
    assert_eq!(s.host().performed("act_chime"), 1);
    assert_eq!(s.ledger().runs().count(), 1);
    assert_eq!(custom_count(&s, "svc.execution.duplicate"), 1);
    // A new utterance is a new occurrence and rings again.
    s.utterance("ring the brass chime", Some("u2"));
    assert_eq!(s.host().spoken(), vec!["Ding (chime).", "Ding (brass)."]);
}

#[test]
fn ledger_survives_a_restart() {
    let mut s = live(SimHost::new().script("act_report", fail("x")).script("act_report", fail("x")).script("act_report", fail("x")));
    s.utterance("ring the chime", Some("u1"));
    let saved = s.ledger().to_json();

    let mut restarted = Svcd::new(SimHost::new(), Config::default()).with_ledger(Ledger::from_json(&saved).unwrap());
    restarted.install(svcd::load(CHIME).unwrap()).unwrap();
    restarted.utterance("ring the chime", Some("u1"));
    assert_eq!(restarted.host().performed("act_chime"), 0, "spoken before the restart; not again");
    assert_eq!(restarted.host().performed("act_report"), 1, "the unfinished effect completes");
}

#[test]
fn abort_before_commit_compensates_and_leaves_nothing_behind() {
    // act_reserve (compensated) succeeds, then act_count writes a string into
    // the integer `count`, so the pre-commit state-schema check fails.
    let mut ir: Value = json(CHIME);
    ir["actions"]["act_count"]["state_patch"][0]["value"] = json!("{{'not a number'}}");
    let mut s = Svcd::new(SimHost::new(), Config::default());
    s.install(svcd::validate::load_value(ir).unwrap()).unwrap();
    s.utterance("ring the chime", Some("u1"));

    assert_eq!(s.state("svc.test.chime").unwrap(), &json!({ "count": 0 }), "overlay discarded");
    assert_eq!(speaks(&s), 0);
    assert_eq!(s.ledger().effects().count(), 0);
    let comps: Vec<_> = s.host().calls.iter().filter(|c| matches!(c.kind, ActionKind::Compensate { .. })).collect();
    assert_eq!(comps.len(), 3, "one compensation per attempt (1 + 2 retries)");
    assert!(comps.iter().all(|c| c.action_id == "act_release"));
    let err = s.bus().iter().find(|m| m.msg_type == "ovos.intent.handler.error").unwrap();
    assert!(err.data["error"].as_str().unwrap().contains("state_schema at /count"), "{}", err.data);
}

#[test]
fn terminal_execution_rows_are_immutable() {
    let mut l = Ledger::new();
    let run = l.claim("x-1", "svc.a.b", "trig_x", 0);
    l.transition(&run, ExecStatus::Running, 1, None).unwrap();
    l.transition(&run, ExecStatus::Completed, 2, None).unwrap();
    for to in [ExecStatus::Running, ExecStatus::Failed, ExecStatus::Completed] {
        assert!(matches!(l.transition(&run, to, 3, None), Err(LedgerError::IllegalTransition { .. })));
    }
    assert_eq!(l.claim("x-1", "svc.a.b", "trig_x", 4), "x-1/2", "a redelivery is a new row");
}

#[test]
fn duplicate_effect_keys_are_refused_by_the_ledger() {
    let mut l = Ledger::new();
    let row = svcd::ledger::EffectRow {
        service_id: "svc.a.b".into(),
        key: svcd::ledger::idempotency_key("svc.a.b", "k"),
        material: "k".into(),
        action_id: "act".into(),
        execution_id: "x".into(),
        effect_class: svcd::ir::EffectClass::CommittedIrreversible,
        output: json!(null),
        at_ms: 0,
    };
    l.record_effect(row.clone()).unwrap();
    assert!(matches!(l.record_effect(row.clone()), Err(LedgerError::DuplicateEffect { .. })));
    let other_service = svcd::ledger::EffectRow { service_id: "svc.c.d".into(), ..row };
    assert!(l.record_effect(other_service).is_ok(), "keys are scoped per service");
}

#[test]
fn idempotency_keys_are_deterministic_and_scoped() {
    use svcd::ledger::{execution_id, idempotency_key};
    assert_eq!(idempotency_key("svc.a.b", "m"), idempotency_key("svc.a.b", "m"));
    assert_ne!(idempotency_key("svc.a.b", "m"), idempotency_key("svc.a.c", "m"));
    assert!(idempotency_key("svc.a.b", "m").starts_with("blake3:"));
    assert_eq!(execution_id("s", "t", "u1"), execution_id("s", "t", "u1"));
    assert_ne!(execution_id("s", "t", "u1"), execution_id("s", "t", "u2"));
}
