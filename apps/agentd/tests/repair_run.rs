//! CV2 ASR-confidence repair strata: the run-level proof that a
//! low-confidence turn asks exactly one targeted question and
//! resumes from the spoken answer, that medium proceeds with a
//! restatement note, that high and no-signal turns behave exactly
//! as before — and that none of it weakens BRAINFIX: the system
//! prompt stays byte-identical, a LOW turn never reaches a tool or
//! the broker, and the commit gate still fires on MEDIUM and on
//! resumed turns.

mod common;

use std::fs;
use std::path::PathBuf;
use std::time::Instant;

use agentd::agent::Agent;
use agentd::config::{self, Config};
use agentd::ir_help;
use agentd::journal::Journal;
use agentd::phone::ProbeRoot;
use agentd::repair::{classify, compose_resume, extract_span, question_for, PendingRepair, SpanKind, Stratum};
use agentd::session::SessionStore;
use common::{chat_body, spawn, text_message, tool_call_message, Step};
use serde_json::{json, Value};
use trace_events::{AgUiEvent, Event, Payload};

fn unique(tag: &str) -> PathBuf {
    let thread: String = format!("{:?}", std::thread::current().id())
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    std::env::temp_dir().join(format!("agentd-{tag}-{}-{thread}", std::process::id()))
}

fn probe_fixture() -> ProbeRoot {
    let dir = unique("probe");
    fs::create_dir_all(dir.join("sys/class/power_supply/bq27411-0")).expect("fixture battery");
    fs::write(dir.join("sys/class/power_supply/bq27411-0/capacity"), "88\n").unwrap();
    fs::write(dir.join("sys/class/power_supply/bq27411-0/status"), "Discharging\n").unwrap();
    fs::create_dir_all(dir.join("proc/net")).unwrap();
    fs::write(dir.join("proc/uptime"), "3600.5 12000.0\n").unwrap();
    fs::write(dir.join("proc/meminfo"), "MemTotal:  7723008 kB\nMemAvailable:  4800000 kB\n").unwrap();
    fs::write(dir.join("proc/loadavg"), "0.42 0.11 0.05 1/100 1234\n").unwrap();
    ProbeRoot(dir)
}

fn key_file() -> PathBuf {
    let path = unique("key");
    fs::write(&path, "test-key-not-a-secret\n").unwrap();
    path
}

fn config_for(gateway_port: u16) -> Config {
    let text = format!(
        "listen = \"127.0.0.1:8791\"\n\
         gateway_url = \"http://127.0.0.1:{gateway_port}/v1/chat/completions\"\n\
         gateway_model = \"auto\"\n\
         key_file = \"{}\"\n\
         model_timeout_sec = 5\n\
         max_tool_rounds = 4\n",
        key_file().display()
    );
    config::load(&text).expect("test config must load")
}

fn stub_gio() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let dir = unique("giobin");
        fs::create_dir_all(&dir).unwrap();
        let gio = dir.join("gio");
        fs::write(&gio, "#!/bin/sh\nexit 0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&gio, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let old = std::env::var("PATH").unwrap_or_default();
        std::env::set_var("PATH", format!("{}:{old}", dir.display()));
    });
}

fn customs(events: &[Event], name: &str) -> Vec<Value> {
    events
        .iter()
        .filter_map(|e| match &e.payload {
            Payload::AgUi(AgUiEvent::Custom { name: n, value }) if n == name => Some(value.clone()),
            _ => None,
        })
        .collect()
}

fn create_args() -> String {
    let ir: Value = serde_json::from_str(ir_help::WATER_REMINDER).expect("example is JSON");
    json!({ "ir": ir }).to_string()
}

const LABEL: &str = "service_create svc.demo.water_reminder";

// ---------------------------------------------------------------------------
// The pure machinery: classification boundaries, span extraction,
// question shape, resume composition.
// ---------------------------------------------------------------------------

#[test]
fn classify_boundaries_and_no_signal() {
    // No signal is HIGH behavior with an honest reason — the strata
    // never invent doubt the caller did not report.
    let (s, reason) = classify(None, 0.7, 0.55);
    assert_eq!(s, Stratum::High);
    assert!(reason.contains("no asr_confidence"), "{reason}");
    assert_eq!(classify(Some(0.8), 0.7, 0.55).0, Stratum::High, "the live proxy's clear value");
    assert_eq!(classify(Some(0.7), 0.7, 0.55).0, Stratum::High, "at the high bar");
    assert_eq!(classify(Some(0.69), 0.7, 0.55).0, Stratum::Medium);
    assert_eq!(classify(Some(0.55), 0.7, 0.55).0, Stratum::Medium, "the low bar itself is medium");
    assert_eq!(classify(Some(0.54), 0.7, 0.55).0, Stratum::Low);
    assert_eq!(classify(Some(0.4), 0.7, 0.55).0, Stratum::Low, "the live proxy's quiet value");
}

#[test]
fn extract_span_shapes() {
    assert_eq!(
        extract_span("Open maps and give me directions to tour."),
        (SpanKind::Destination, "tour".to_string())
    );
    assert_eq!(
        extract_span("give me directions to San Francisco"),
        (SpanKind::Destination, "San Francisco".to_string())
    );
    assert_eq!(extract_span("open maps"), (SpanKind::App, "maps".to_string()));
    assert_eq!(extract_span("what time is it"), (SpanKind::Generic, "what time is it".to_string()));
}

#[test]
fn compose_resume_replaces_only_the_span() {
    let pending = PendingRepair {
        original: "Open maps and give me directions to tour".to_string(),
        span: "tour".to_string(),
        kind: SpanKind::Destination,
    };
    assert_eq!(
        compose_resume(&pending, "San Francisco"),
        "Open maps and give me directions to San Francisco"
    );
    // A confirming answer keeps the original words exactly.
    assert_eq!(compose_resume(&pending, "yes"), pending.original);
    assert_eq!(compose_resume(&pending, "That's right."), pending.original);
    // The question asks about the span only — one question mark, and
    // it names what was heard.
    let q = question_for(SpanKind::Destination, "tour");
    assert_eq!(q.matches('?').count(), 1, "{q}");
    assert!(q.contains("\"tour\""), "{q}");
}

#[test]
fn repair_thresholds_are_config() {
    let base = "gateway_url = \"http://127.0.0.1:9/v1/chat/completions\"\ngateway_model = \"auto\"\nkey_file = \"/tmp/k\"\n";
    let cfg = config::load(&format!("{base}repair_high = 0.9\nrepair_low = 0.3\n")).expect("thresholds load");
    assert_eq!((cfg.repair_high, cfg.repair_low), (0.9, 0.3));
    let bad = config::load(&format!("{base}repair_high = 0.4\nrepair_low = 0.6\n"));
    assert!(bad.is_err(), "an inverted band is a config error, not a silent medium-never");
    let oor = config::load(&format!("{base}repair_high = 1.4\n"));
    assert!(oor.is_err(), "out-of-range thresholds are rejected");
}

// ---------------------------------------------------------------------------
// Run-level strata behavior against the fake brain.
// ---------------------------------------------------------------------------

#[test]
fn low_turn_asks_one_targeted_question_then_the_answer_resumes() {
    stub_gio();
    let brain = spawn(
        "gateway",
        vec![
            Step::Reply(chat_body(
                tool_call_message("call-1", "app_launch", "{\"app\":\"maps\",\"destination\":\"San Francisco\"}"),
                "fake",
            )),
            Step::Reply(chat_body(text_message("Maps is open with San Francisco on the map."), "fake")),
            Step::Reply(chat_body(text_message("About 55 minutes to San Francisco."), "fake")),
        ],
    );
    let journal = Journal::new(512);
    let agent = Agent::with_root(config_for(brain.port), journal.clone(), probe_fixture());

    // Turn 1 arrives quiet (the live proxy's LOW value): the turn is
    // the question and nothing else — the brain is never called and
    // no tool runs, even though the request names a launchable app.
    let s1 = agent.handle_session_conf(
        "open maps and give me directions to tour",
        None,
        Some("sess-repair-1"),
        false,
        Some(0.4),
    );
    assert_eq!(
        s1.answer.as_deref(),
        Some("I heard the destination as \"tour\", but I'm not confident I got it right. Where did you want directions to?")
    );
    assert_eq!(brain.hits(), 0, "a LOW turn must not call the brain");
    let text = journal.jsonl().expect("journal serializes");
    assert!(text.contains("repair_stratum") && text.contains("\"low\""), "stratum journaled:\n{text}");
    assert!(!text.contains("app_launch"), "a LOW turn must not launch anything:\n{text}");

    // Turn 2 is the spoken answer — arriving quiet too, and accepted
    // anyway: a repair never asks twice. The brain serves the
    // COMPOSED request (original words, span replaced by the answer).
    let s2 = agent.handle_session_conf("San Francisco", None, Some("sess-repair-1"), true, Some(0.4));
    assert_eq!(s2.answer.as_deref(), Some("Maps is open with San Francisco on the map."), "{s2:?}");
    let bodies = brain.bodies();
    assert_eq!(bodies.len(), 2, "the resumed turn runs the normal loop: {}", bodies.len());
    let payload: Value = serde_json::from_str(&bodies[0]).expect("brain request is JSON");
    let messages = payload["messages"].as_array().expect("messages array");
    assert_eq!(
        messages.last().unwrap()["content"].as_str(),
        Some("open maps and give me directions to San Francisco"),
        "the brain must serve the composed request: {messages:?}"
    );
    let resumed = customs(&journal.snapshot(), "repair_resumed");
    assert_eq!(resumed.len(), 1, "{resumed:?}");
    assert_eq!(resumed[0]["original"].as_str(), Some("open maps and give me directions to tour"));
    assert_eq!(resumed[0]["heard_span"].as_str(), Some("tour"));
    assert_eq!(resumed[0]["answer"].as_str(), Some("San Francisco"));
    assert_eq!(resumed[0]["composed"].as_str(), Some("open maps and give me directions to San Francisco"));

    // The repair is consumed: a third turn is an ordinary follow-up
    // against the entity the resumed turn established.
    let s3 = agent.handle_session_conf("how long does it take to get there?", None, Some("sess-repair-1"), true, None);
    assert_eq!(s3.answer.as_deref(), Some("About 55 minutes to San Francisco."));
    let contexts = customs(&journal.snapshot(), "session_context");
    let last = contexts.last().expect("a session context");
    assert_eq!(last["entities"]["destination"].as_str(), Some("San Francisco"));
    assert_eq!(customs(&journal.snapshot(), "repair_resumed").len(), 1, "no second resume");
}

#[test]
fn high_and_no_signal_turns_behave_as_before() {
    let brain = spawn(
        "gateway",
        vec![
            Step::Reply(chat_body(text_message("It is noon."), "fake")),
            Step::Reply(chat_body(text_message("Still noon."), "fake")),
        ],
    );
    let journal = Journal::new(512);
    let agent = Agent::with_root(config_for(brain.port), journal.clone(), probe_fixture());

    let s1 = agent.handle_session_conf("what time is it", None, Some("sess-repair-2"), false, Some(0.8));
    assert_eq!(s1.answer.as_deref(), Some("It is noon."));
    let s2 = agent.handle_session_conf("and now", None, Some("sess-repair-2"), true, None);
    assert_eq!(s2.answer.as_deref(), Some("Still noon."));
    assert_eq!(brain.hits(), 2, "both turns went straight to the brain — no question added");
    let strata = customs(&journal.snapshot(), "repair_stratum");
    assert_eq!(strata.len(), 2, "every classified turn is journaled: {strata:?}");
    assert!(strata.iter().all(|v| v["stratum"] == "high"));
    assert!(strata[1]["reason"].as_str().unwrap().contains("no asr_confidence"));
    assert!(customs(&journal.snapshot(), "repair_resumed").is_empty());
}

#[test]
fn medium_turn_proceeds_restates_and_keeps_the_prompt_untouched() {
    stub_gio();
    let brain = spawn(
        "gateway",
        vec![
            Step::Reply(chat_body(
                tool_call_message("call-1", "app_launch", "{\"app\":\"maps\",\"destination\":\"San Francisco\"}"),
                "fake",
            )),
            Step::Reply(chat_body(text_message("Showing San Francisco on the map."), "fake")),
        ],
    );
    let journal = Journal::new(512);
    let agent = Agent::with_root(config_for(brain.port), journal.clone(), probe_fixture());

    let s = agent.handle_session_conf(
        "open maps and give me directions to San Francisco",
        None,
        Some("sess-repair-3"),
        false,
        Some(0.6),
    );
    assert_eq!(s.answer.as_deref(), Some("Showing San Francisco on the map."), "medium proceeds: {s:?}");
    let bodies = brain.bodies();
    let payload: Value = serde_json::from_str(&bodies[0]).expect("brain request is JSON");
    let messages = payload["messages"].as_array().expect("messages array");
    // BRAINFIX regression: the strata ride as data; the system prompt
    // is byte-identical to the sessionless, stratum-free build.
    assert_eq!(messages[0]["content"].as_str().unwrap(), agent.system_prompt(), "medium edited the system prompt");
    let contents: Vec<&str> = messages.iter().filter_map(|m| m["content"].as_str()).collect();
    assert!(
        contents.iter().any(|c| c.contains("Transcription note") && c.contains("\"San Francisco\"")),
        "the restatement note with the heard span must reach the brain: {contents:?}"
    );
    let strata = customs(&journal.snapshot(), "repair_stratum");
    assert_eq!(strata.len(), 1);
    assert_eq!(strata[0]["stratum"], "medium");
    assert_eq!(strata[0]["span"].as_str(), Some("San Francisco"));
}

// ---------------------------------------------------------------------------
// BRAINFIX composition regressions: the strata must not weaken the
// confirmation discipline in any direction.
// ---------------------------------------------------------------------------

#[test]
fn low_turn_never_reaches_a_tool_or_the_broker() {
    let brain = spawn("gateway", vec![]);
    let journal = Journal::new(512);
    let agent = Agent::with_root(config_for(brain.port), journal.clone(), probe_fixture());

    // A commit-class request at LOW confidence: the answer is the
    // repair question. Nothing is created, nothing is gated (the
    // broker never even sees it), and the brain is never asked to
    // improvise around the mishearing.
    let s = agent.handle_session_conf("remind me to drink water every hour", None, Some("sess-repair-4"), false, Some(0.4));
    let answer = s.answer.expect("the question is the answer");
    assert!(answer.contains("remind me to drink water every hour"), "generic question quotes the heard request: {answer}");
    assert_eq!(brain.hits(), 0);
    assert_eq!(agent.services.list()["count"], 0, "nothing installed");
    let text = journal.jsonl().expect("journal serializes");
    assert!(!text.contains("ask_to_commit"), "the broker must not be reached:\n{text}");
    assert!(!text.contains("service_create"), "no tool vocabulary in a LOW turn:\n{text}");
}

#[test]
fn medium_does_not_bypass_the_commit_gate() {
    let brain = spawn(
        "gateway",
        vec![
            Step::Reply(chat_body(tool_call_message("call-1", "service_create", &create_args()), "fake")),
            Step::Reply(chat_body(text_message("That install needs your confirmation first."), "fake")),
        ],
    );
    let journal = Journal::new(512);
    let agent = Agent::with_root(config_for(brain.port), journal.clone(), probe_fixture());

    let s = agent.handle_session_conf("Remind me to drink water every hour.", None, Some("sess-repair-5"), false, Some(0.6));
    assert_eq!(s.answer.as_deref(), Some("That install needs your confirmation first."));
    assert_eq!(agent.services.list()["count"], 0, "medium confidence is not a confirmation");
    let text = journal.jsonl().expect("journal serializes");
    assert!(text.contains("ask_to_commit") && text.contains(LABEL), "the gate fires exactly as without strata:\n{text}");
}

#[test]
fn a_resumed_turn_is_still_gated() {
    let brain = spawn(
        "gateway",
        vec![
            Step::Reply(chat_body(tool_call_message("call-1", "service_create", &create_args()), "fake")),
            Step::Reply(chat_body(text_message("That install needs your confirmation first."), "fake")),
        ],
    );
    let journal = Journal::new(512);
    let agent = Agent::with_root(config_for(brain.port), journal.clone(), probe_fixture());

    // Turn 1 at LOW asks the question; turn 2 restates the request
    // (the generic composition: the answer IS the request). The
    // resumed request meets the broker exactly as a fresh one does —
    // repair is not a confirmation channel.
    let s1 = agent.handle_session_conf("remind me to drink water", None, Some("sess-repair-6"), false, Some(0.4));
    assert!(s1.answer.expect("question").contains('?'));
    assert_eq!(brain.hits(), 0);
    let s2 = agent.handle_session_conf("remind me to drink water every hour", None, Some("sess-repair-6"), true, None);
    assert_eq!(s2.answer.as_deref(), Some("That install needs your confirmation first."), "{s2:?}");
    assert_eq!(agent.services.list()["count"], 0, "resumed does not mean confirmed");
    let text = journal.jsonl().expect("journal serializes");
    assert!(text.contains("ask_to_commit") && text.contains(LABEL), "the gate fires on the resumed turn:\n{text}");
    let resumed = customs(&journal.snapshot(), "repair_resumed");
    assert_eq!(resumed.len(), 1);
    assert_eq!(resumed[0]["composed"].as_str(), Some("remind me to drink water every hour"));
}

#[test]
fn session_store_carries_the_repair_beside_pending() {
    let store = SessionStore::with_ttl(std::time::Duration::from_secs(60));
    let t0 = Instant::now();
    let _ = store.begin("sess-store", t0);
    assert!(store.begin("sess-store", t0).repair.is_none());
    let pending = PendingRepair {
        original: "open maps and give me directions to tour".to_string(),
        span: "tour".to_string(),
        kind: SpanKind::Destination,
    };
    store.set_repair("sess-store", Some(pending.clone()), t0);
    let ctx = store.begin("sess-store", t0);
    assert_eq!(ctx.repair, Some(pending), "the next turn's snapshot carries the open repair");
    store.set_repair("sess-store", None, t0);
    assert!(store.begin("sess-store", t0).repair.is_none(), "consuming clears it");
}
