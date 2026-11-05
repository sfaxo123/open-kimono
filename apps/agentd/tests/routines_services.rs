//! W2 (A3.2): user routines ARE svcd services. There is no separate cron
//! engine: a routine is created, listed, and stopped through the same
//! service tools, driven here through the whole request loop. Pause does
//! not exist in svcd (its status is only running/stopped), so a pause
//! request must fail honestly by name instead of pretending.

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
    std::env::temp_dir().join(format!("agentd-routines-{tag}-{}-{thread}", std::process::id()))
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

fn agent_with(gateway_port: u16) -> (Agent, Journal) {
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
    let cfg: Config = config::load(&text).expect("test config must load");
    let journal = Journal::new(512);
    (Agent::with_root(cfg, journal.clone(), probe_fixture()), journal)
}

fn create_args() -> String {
    let ir: Value = serde_json::from_str(ir_help::WATER_REMINDER).expect("example 1 is JSON");
    json!({ "ir": ir }).to_string()
}

#[test]
fn a_routine_is_created_listed_and_stopped_as_a_service_through_requests() {
    let stop_args = json!({ "id": "svc.demo.water_reminder" }).to_string();
    let gateway = spawn("routine", vec![
        Step::Reply(chat_body(tool_call_message("c1", "service_create", &create_args()), "auto")),
        Step::Reply(chat_body(text_message("Your hourly water routine is set."), "auto")),
        Step::Reply(chat_body(tool_call_message("c2", "service_list", "{}"), "auto")),
        Step::Reply(chat_body(text_message("One routine: the water reminder, running."), "auto")),
        Step::Reply(chat_body(tool_call_message("c3", "service_stop", &stop_args), "auto")),
        Step::Reply(chat_body(text_message("Routine stopped."), "auto")),
    ]);
    let (agent, journal) = agent_with(gateway.port);

    let created = agent.handle_with_confirm("Every hour, remind me to drink water.", Some("service_create svc.demo.water_reminder"));
    assert_eq!(created.answer.as_deref(), Some("Your hourly water routine is set."));
    let listed = agent.services.list();
    assert_eq!(listed["count"], 1, "the routine is one installed service: {listed}");
    assert_eq!(listed["services"][0]["status"]["status"], "running");

    let asked = agent.handle("What routines do I have?");
    assert_eq!(asked.answer.as_deref(), Some("One routine: the water reminder, running."));
    // The list the model read in round two really contained the routine.
    assert!(gateway.bodies()[3].contains("svc.demo.water_reminder"), "the model must read the real list: {}", gateway.bodies()[3]);

    let stopped = agent.handle("Stop the water routine.");
    assert_eq!(stopped.answer.as_deref(), Some("Routine stopped."));
    let after = agent.services.list();
    assert_eq!(after["services"][0]["status"]["status"], "stopped", "the routine stops as a service: {after}");

    let text = journal.jsonl().expect("journal serializes");
    for tool in ["service_create", "service_list", "service_stop"] {
        assert!(text.contains(tool), "the trace must show {tool}:\n{text}");
    }
}

#[test]
fn negative_pause_is_refused_by_name_and_the_routine_keeps_running() {
    // svcd has no paused state, so agentd offers no pause tool. The model
    // asking for one must be refused by name — never silently "paused".
    let pause_args = json!({ "id": "svc.demo.water_reminder" }).to_string();
    let gateway = spawn("routine-pause", vec![
        Step::Reply(chat_body(tool_call_message("c1", "service_create", &create_args()), "auto")),
        Step::Reply(chat_body(text_message("Routine set."), "auto")),
        Step::Reply(chat_body(tool_call_message("c2", "service_pause", &pause_args), "auto")),
        Step::Reply(chat_body(text_message("I cannot pause it; you can stop it instead."), "auto")),
    ]);
    let (agent, journal) = agent_with(gateway.port);

    agent.handle_with_confirm("Set the water routine.", Some("service_create svc.demo.water_reminder"));
    let summary = agent.handle("Pause the water routine.");
    assert_eq!(summary.answer.as_deref(), Some("I cannot pause it; you can stop it instead."));
    assert_eq!(
        agent.services.list()["services"][0]["status"]["status"],
        "running",
        "a refused pause must change nothing"
    );
    let text = journal.jsonl().expect("journal serializes");
    assert!(text.contains("unknown tool `service_pause`"), "the refusal must name the missing tool:\n{text}");
}
