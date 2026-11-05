//! Destination-bound navigation gate: an inferred destination may
//! only launch after the owner confirms THAT destination. A
//! confirmation for one destination never spends another's pending
//! slot, a naming inside a departure flow is read back before it
//! launches, and a destination the owner named outright in a
//! user-framed request still launches at once.

mod common;

use std::fs;
use std::path::PathBuf;

use agentd::agent::Agent;
use agentd::config::{self, Config};
use agentd::journal::Journal;
use agentd::phone::ProbeRoot;
use common::{chat_body, spawn, text_message, tool_call_message, Step};

fn unique(tag: &str) -> PathBuf {
    let thread: String = format!("{:?}", std::thread::current().id())
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    std::env::temp_dir().join(format!("agentd-navgate-{tag}-{}-{thread}", std::process::id()))
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

fn launch_args(destination: &str) -> String {
    serde_json::json!({ "app": "maps", "destination": destination }).to_string()
}

#[test]
fn inferred_navigation_waits_for_owner_confirmation_of_that_destination() {
    stub_gio();
    let brain = spawn(
        "nav-inferred",
        vec![
            Step::Reply(chat_body(tool_call_message("c1", "app_launch", &launch_args("SFO")), "fake")),
            Step::Reply(chat_body(text_message("Your flight is from SFO - head there?"), "fake")),
            Step::Reply(chat_body(tool_call_message("c2", "app_launch", &launch_args("SFO")), "fake")),
            Step::Reply(chat_body(text_message("Starting navigation to SFO now."), "fake")),
        ],
    );
    let journal = Journal::new(512);
    let agent = Agent::with_root(config_for(brain.port), journal.clone(), probe_fixture());

    // Turn 0 names no airport: an immediate launch is an unconfirmed
    // inference and must be gated into a read-back question.
    let s1 = agent.handle_session("hey phone i need to get to my flight", None, Some("sess-nav-1"), false);
    assert_eq!(s1.answer.as_deref(), Some("Your flight is from SFO - head there?"), "{s1:?}");
    assert!(
        brain.bodies()[1].contains("ask_to_commit") && brain.bodies()[1].contains("app_launch SFO"),
        "the gated result must name exactly this destination: {}",
        brain.bodies()[1]
    );
    let text = journal.jsonl().expect("journal serializes");
    assert!(text.contains("app_launch SFO"), "the gate must be journaled with its label:\n{text}");

    // The owner's yes for SFO spends SFO's slot, and only SFO's.
    let s2 = agent.handle_session("yes sfo go ahead", None, Some("sess-nav-1"), true);
    assert_eq!(s2.answer.as_deref(), Some("Starting navigation to SFO now."), "{s2:?}");
    assert!(brain.bodies()[3].contains("geo:0,0?q=SFO"), "the confirmed turn must really launch: {}", brain.bodies()[3]);
}

#[test]
fn a_different_destination_cannot_spend_the_pending_confirmation() {
    stub_gio();
    let brain = spawn(
        "nav-passenger",
        vec![
            Step::Reply(chat_body(text_message("Work - head there?"), "fake")),
            Step::Reply(chat_body(tool_call_message("c1", "app_launch", &launch_args("airport")), "fake")),
            Step::Reply(chat_body(text_message("I heard airport, but work is still pending - head to work?"), "fake")),
            Step::Reply(chat_body(tool_call_message("c2", "app_launch", &launch_args("work")), "fake")),
            Step::Reply(chat_body(text_message("Starting navigation to work now."), "fake")),
        ],
    );
    let journal = Journal::new(512);
    let agent = Agent::with_root(config_for(brain.port), journal.clone(), probe_fixture());

    let s1 = agent.handle_session("hey phone lets go", None, Some("sess-nav-2"), false);
    assert_eq!(s1.answer.as_deref(), Some("Work - head there?"));

    // A different destination, whoever voices it, never satisfies
    // work's pending confirmation: no launch to the airport happens.
    let s2 = agent.handle_session("no navigate to the airport first", None, Some("sess-nav-2"), true);
    assert!(
        s2.answer.as_deref().unwrap_or("").contains("work"),
        "the pending destination must survive in the answer: {s2:?}"
    );
    assert!(
        brain.bodies()[2].contains("app_launch airport"),
        "the airport launch must be gated with its own label, not executed: {}",
        brain.bodies()[2]
    );
    assert!(
        !brain.bodies()[2].contains("geo:0,0?q=airport"),
        "the airport must never be launched on work's confirmation: {}",
        brain.bodies()[2]
    );

    // The owner confirming work still launches work.
    let s3 = agent.handle_session("yes work is right go ahead", None, Some("sess-nav-2"), true);
    assert_eq!(s3.answer.as_deref(), Some("Starting navigation to work now."), "{s3:?}");
    let text = journal.jsonl().expect("journal serializes");
    assert!(text.contains("app_launch airport"), "the blocked attempt stays in the journal:\n{text}");
}

#[test]
fn naming_inside_a_departure_flow_is_read_back_before_it_launches() {
    stub_gio();
    let brain = spawn(
        "nav-departure",
        vec![
            Step::Reply(chat_body(text_message("Ready - where are we headed today?"), "fake")),
            Step::Reply(chat_body(tool_call_message("c1", "app_launch", &launch_args("farmers market")), "fake")),
            Step::Reply(chat_body(text_message("The farmers market - head there?"), "fake")),
            Step::Reply(chat_body(tool_call_message("c2", "app_launch", &launch_args("farmers market")), "fake")),
            Step::Reply(chat_body(text_message("Starting navigation to the farmers market."), "fake")),
        ],
    );
    let journal = Journal::new(512);
    let agent = Agent::with_root(config_for(brain.port), journal.clone(), probe_fixture());

    let s1 = agent.handle_session("hey phone lets go", None, Some("sess-nav-3"), false);
    assert_eq!(s1.answer.as_deref(), Some("Ready - where are we headed today?"));

    // Inside a departure flow, naming settles WHICH place; it is not
    // the go-ahead. The launch is gated into the read-back.
    let s2 = agent.handle_session("the farmers market", None, Some("sess-nav-3"), true);
    assert_eq!(s2.answer.as_deref(), Some("The farmers market - head there?"), "{s2:?}");
    assert!(brain.bodies()[2].contains("app_launch farmers market"), "the naming-turn launch must be gated with its label: {}", brain.bodies()[2]);

    let s3 = agent.handle_session("yes go ahead", None, Some("sess-nav-3"), true);
    assert_eq!(s3.answer.as_deref(), Some("Starting navigation to the farmers market."), "{s3:?}");
    let _ = journal;
}

#[test]
fn a_user_named_destination_launches_immediately() {
    stub_gio();
    let brain = spawn(
        "nav-named",
        vec![
            Step::Reply(chat_body(tool_call_message("c1", "app_launch", &launch_args("San Francisco")), "fake")),
            Step::Reply(chat_body(text_message("Maps is open with San Francisco on the map."), "fake")),
        ],
    );
    let journal = Journal::new(512);
    let agent = Agent::with_root(config_for(brain.port), journal.clone(), probe_fixture());

    // User-framed and user-named: no read-back tax, no gate.
    let s = agent.handle_session("open maps and give me directions to San Francisco", None, Some("sess-nav-4"), false);
    assert_eq!(s.answer.as_deref(), Some("Maps is open with San Francisco on the map."), "{s:?}");
    let text = journal.jsonl().expect("journal serializes");
    assert!(!text.contains("app_launch San Francisco"), "a named destination must not be gated:\n{text}");
}
