//! W1 (A1.2 / A1.GATE): every tool call in the request loop passes
//! `Broker::gate`. Installing a service commits a generated block to the
//! phone, so `service_create` is commit-class: without an explicit
//! `confirm` in the request it must not run, the journal must carry
//! `ask_to_commit` with the label, and the model must be told why.
//! Read-only tools are free and run with no confirmation at all.

mod common;

use std::fs;
use std::path::PathBuf;

use agentd::agent::Agent;
use agentd::config::{self, Config};
use agentd::ir_help;
use agentd::journal::Journal;
use agentd::phone::ProbeRoot;
use common::{chat_body, spawn, text_message, tool_call_message, Step};
use serde_json::{json, Value};

fn unique(tag: &str) -> PathBuf {
    let thread: String = format!("{:?}", std::thread::current().id())
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    std::env::temp_dir().join(format!("agentd-broker-{tag}-{}-{thread}", std::process::id()))
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
    fs::write(dir.join("proc/net/dev"), "Inter-|   Receive\n face |bytes\n    lo: 100 0\n wlan0: 4321 987\n").unwrap();
    ProbeRoot(dir)
}

fn config_for(gateway_port: u16) -> Config {
    let key = unique("key");
    fs::write(&key, "test-key-not-a-secret\n").unwrap();
    let text = format!(
        "listen = \"127.0.0.1:8791\"\n\
         gateway_url = \"http://127.0.0.1:{gateway_port}/v1/chat/completions\"\n\
         gateway_model = \"auto\"\n\
         key_file = \"{}\"\n\
         model_timeout_sec = 5\n\
         max_tool_rounds = 4\n",
        key.display()
    );
    config::load(&text).expect("test config must load")
}

fn agent_with(gateway_port: u16) -> (Agent, Journal) {
    let journal = Journal::new(512);
    let agent = Agent::with_root(config_for(gateway_port), journal.clone(), probe_fixture());
    (agent, journal)
}

fn create_args() -> String {
    let ir: Value = serde_json::from_str(ir_help::WATER_REMINDER).expect("example 1 is JSON");
    json!({ "ir": ir }).to_string()
}

const LABEL: &str = "service_create svc.demo.water_reminder";

#[test]
fn unconfirmed_commit_tool_never_executes() {
    let gateway = spawn("gate-block", vec![
        Step::Reply(chat_body(tool_call_message("call-1", "service_create", &create_args()), "auto")),
        Step::Reply(chat_body(text_message("That install needs your confirmation first."), "auto")),
    ]);
    let (agent, journal) = agent_with(gateway.port);

    let summary = agent.handle("Remind me to drink water every hour.");
    assert_eq!(summary.answer.as_deref(), Some("That install needs your confirmation first."));
    assert_eq!(agent.services.list()["count"], 0, "an unconfirmed commit must install nothing");

    let text = journal.jsonl().expect("journal serializes");
    assert!(text.contains("ask_to_commit"), "the gate must be journaled:\n{text}");
    assert!(text.contains(LABEL), "the event must carry the exact label:\n{text}");
    // The model is told why, in the tool result it reads next round.
    assert!(gateway.bodies()[1].contains("ask_to_commit"), "the model must see the block: {}", gateway.bodies()[1]);
    assert!(gateway.bodies()[1].contains(LABEL), "and the label to confirm: {}", gateway.bodies()[1]);
}

#[test]
fn confirmed_commit_tool_runs() {
    let gateway = spawn("gate-confirm", vec![
        Step::Reply(chat_body(tool_call_message("call-1", "service_create", &create_args()), "auto")),
        Step::Reply(chat_body(text_message("Your water reminder is running."), "auto")),
    ]);
    let (agent, journal) = agent_with(gateway.port);

    let summary = agent.handle_with_confirm("Remind me to drink water every hour.", Some(LABEL));
    assert_eq!(summary.answer.as_deref(), Some("Your water reminder is running."));
    let listed = agent.services.list();
    assert_eq!(listed["count"], 1, "a confirmed commit must really install: {listed}");
    assert_eq!(listed["services"][0]["status"]["status"], "running");
    assert!(!journal.jsonl().unwrap().contains("ask_to_commit"), "a confirmed call must not be gated");
}

#[test]
fn negative_a_confirmation_for_a_different_label_does_not_run() {
    let gateway = spawn("gate-wrong", vec![
        Step::Reply(chat_body(tool_call_message("call-1", "service_create", &create_args()), "auto")),
        Step::Reply(chat_body(text_message("Still needs confirmation."), "auto")),
    ]);
    let (agent, journal) = agent_with(gateway.port);

    let summary = agent.handle_with_confirm("Remind me.", Some("service_create svc.demo.other"));
    assert_eq!(summary.answer.as_deref(), Some("Still needs confirmation."));
    assert_eq!(agent.services.list()["count"], 0, "confirming a different action must not run this one");
    assert!(journal.jsonl().unwrap().contains("ask_to_commit"), "the mismatch must still be gated and journaled");
}

#[test]
fn free_tools_run_without_any_confirmation() {
    let gateway = spawn("gate-free", vec![
        Step::Reply(chat_body(tool_call_message("call-1", "phone_status", "{}"), "auto")),
        Step::Reply(chat_body(text_message("Battery is 88 percent."), "auto")),
    ]);
    let (agent, journal) = agent_with(gateway.port);

    let summary = agent.handle("how is the phone doing?");
    assert_eq!(summary.answer.as_deref(), Some("Battery is 88 percent."));
    assert_eq!(gateway.hits(), 2, "the free tool result must feed a second round");
    assert!(!journal.jsonl().unwrap().contains("ask_to_commit"), "free tools are never gated");
}

#[test]
fn service_stop_is_free_because_it_deletes_nothing() {
    // Recorded decision (WIRE-STATUS): svcd `stop` keeps the IR and state
    // under a `stopped` status — it deletes no user data — so stopping is
    // free-class and needs no confirmation. Creating is the commit.
    let stop_args = json!({ "id": "svc.demo.water_reminder" }).to_string();
    let gateway = spawn("gate-stop", vec![
        Step::Reply(chat_body(tool_call_message("c1", "service_create", &create_args()), "auto")),
        Step::Reply(chat_body(text_message("Installed."), "auto")),
        Step::Reply(chat_body(tool_call_message("c2", "service_stop", &stop_args), "auto")),
        Step::Reply(chat_body(text_message("Stopped."), "auto")),
    ]);
    let (agent, _journal) = agent_with(gateway.port);

    agent.handle_with_confirm("Install the water reminder.", Some(LABEL));
    assert_eq!(agent.services.list()["services"][0]["status"]["status"], "running");

    let summary = agent.handle("Stop the water reminder.");
    assert_eq!(summary.answer.as_deref(), Some("Stopped."));
    let after = agent.services.list();
    assert_eq!(after["count"], 1, "stop must not delete the service record: {after}");
    assert_eq!(after["services"][0]["status"]["status"], "stopped");
}

/// The confirm re-runs the request, and a real model (Muse Spark, measured
/// 2026-11-04) re-authors the IR with a new id each time, so the label the
/// user approved never matched. Confirming a label now installs the exact
/// call the user was shown.
#[test]
fn confirming_a_shown_label_installs_the_held_call_even_if_the_model_renames_it() {
    let renamed = create_args().replace("svc.demo.water_reminder", "svc.user.hydrate");
    let gateway = spawn("gate-held", vec![
        Step::Reply(chat_body(tool_call_message("call-1", "service_create", &create_args()), "auto")),
        Step::Reply(chat_body(text_message("Needs your confirmation."), "auto")),
        Step::Reply(chat_body(tool_call_message("call-2", "service_create", &renamed), "auto")),
        Step::Reply(chat_body(text_message("Your water reminder is running."), "auto")),
    ]);
    let (agent, _journal) = agent_with(gateway.port);

    agent.handle("Remind me to drink water every hour.");
    assert_eq!(agent.services.list()["count"], 0, "first run is only the ask");
    let summary = agent.handle_with_confirm("Remind me to drink water every hour.", Some(LABEL));
    assert_eq!(summary.answer.as_deref(), Some("Your water reminder is running."));
    let listed = agent.services.list();
    assert_eq!(listed["count"], 1, "the confirmed call must install: {listed}");
    assert!(listed.to_string().contains("svc.demo.water_reminder"), "it must be the call the user saw: {listed}");
    assert!(!listed.to_string().contains("svc.user.hydrate"), "never the re-authored one: {listed}");
}

#[test]
fn negative_a_renamed_call_with_no_shown_label_stays_gated() {
    let renamed = create_args().replace("svc.demo.water_reminder", "svc.user.hydrate");
    let gateway = spawn("gate-held-neg", vec![
        Step::Reply(chat_body(tool_call_message("call-1", "service_create", &renamed), "auto")),
        Step::Reply(chat_body(text_message("Still needs confirmation."), "auto")),
    ]);
    let (agent, journal) = agent_with(gateway.port);
    // Nothing was ever shown for LABEL, so confirming it holds nothing to replay.
    agent.handle_with_confirm("Remind me.", Some(LABEL));
    assert_eq!(agent.services.list()["count"], 0, "a never-shown label confirms nothing");
    assert!(journal.jsonl().unwrap().contains("ask_to_commit"));
}
