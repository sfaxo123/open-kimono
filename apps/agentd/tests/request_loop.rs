//! The request loop against fake brains: tool calls, malformed tool calls,
//! timeouts, a dead chain, and the round limit. Every case asserts what landed
//! in the journal, because the journal is the product.

mod common;

use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use agentd::agent::Agent;
use agentd::config::{self, Config};
use agentd::journal::Journal;
use agentd::phone::ProbeRoot;
use common::{chat_body, spawn, text_message, tool_call_message, Step};
use serde_json::Value;
use trace_events::{Event, Op6Event, Payload};

/// Tests run in parallel threads of one process, so every fixture path needs
/// the thread id too — a shared path means one thread truncates a file another
/// is mid-read.
fn unique(tag: &str) -> PathBuf {
    let thread: String = format!("{:?}", std::thread::current().id())
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    std::env::temp_dir().join(format!("agentd-{tag}-{}-{thread}", std::process::id()))
}

/// A phone-shaped fixture tree so `phone_status` is deterministic on the host.
fn probe_fixture() -> ProbeRoot {
    let dir = unique("probe");
    fs::create_dir_all(dir.join("sys/class/power_supply/bq27411-0")).expect("fixture battery");
    fs::write(dir.join("sys/class/power_supply/bq27411-0/capacity"), "88\n").unwrap();
    fs::write(dir.join("sys/class/power_supply/bq27411-0/status"), "Discharging\n").unwrap();
    fs::create_dir_all(dir.join("proc/net")).unwrap();
    fs::write(dir.join("proc/uptime"), "3600.5 12000.0\n").unwrap();
    fs::write(dir.join("proc/meminfo"), "MemTotal:  7723008 kB\nMemAvailable:  4800000 kB\n").unwrap();
    fs::write(dir.join("proc/loadavg"), "0.42 0.11 0.05 1/100 1234\n").unwrap();
    fs::write(
        dir.join("proc/net/dev"),
        "Inter-|   Receive\n face |bytes\n    lo: 100 0\n wlan0: 4321 987\n vpn0: 555 666\n",
    )
    .unwrap();
    ProbeRoot(dir)
}

fn key_file() -> PathBuf {
    let path = unique("key");
    fs::write(&path, "test-key-not-a-secret\n").unwrap();
    path
}

fn config_for(gateway_port: u16, local_port: u16, timeout_sec: u64, rounds: usize) -> Config {
    let text = format!(
        "listen = \"127.0.0.1:8791\"\n\
         gateway_url = \"http://127.0.0.1:{gateway_port}/v1/chat/completions\"\n\
         gateway_model = \"auto\"\n\
         local_url = \"http://127.0.0.1:{local_port}/v1/chat/completions\"\n\
         local_model = \"gemma3-1b\"\n\
         key_file = \"{}\"\n\
         model_timeout_sec = {timeout_sec}\n\
         max_tool_rounds = {rounds}\n",
        key_file().display()
    );
    config::load(&text).expect("test config must load")
}

fn agent_with(gateway_port: u16, local_port: u16, timeout_sec: u64, rounds: usize) -> (Agent, Journal) {
    let journal = Journal::new(512);
    let agent = Agent::with_root(
        config_for(gateway_port, local_port, timeout_sec, rounds),
        journal.clone(),
        probe_fixture(),
    );
    (agent, journal)
}

/// Find the payload tag strings in order, e.g. "ag_ui:RUN_STARTED".
fn tags(events: &[Event]) -> Vec<String> {
    events
        .iter()
        .map(|e| match &e.payload {
            Payload::AgUi(a) => format!("ag_ui:{}", agui_type(a)),
            Payload::GenAi(g) => format!("gen_ai:{}", genai_type(g)),
            Payload::Op6(o) => format!("op6:{}", op6_type(o)),
        })
        .collect()
}

fn agui_type(a: &trace_events::AgUiEvent) -> String {
    serde_json::to_value(a).unwrap()["type"].as_str().unwrap_or("?").to_string()
}
fn genai_type(g: &trace_events::GenAiEvent) -> String {
    serde_json::to_value(g).unwrap()["event"].as_str().unwrap_or("?").to_string()
}
fn op6_type(o: &trace_events::Op6Event) -> String {
    serde_json::to_value(o).unwrap()["type"].as_str().unwrap_or("?").to_string()
}

fn find(events: &[Event], needle: &str) -> Vec<String> {
    events
        .iter()
        .filter_map(|e| trace_events::to_json_line(e).ok())
        .filter(|line| line.contains(needle))
        .collect()
}

#[test]
fn a_tool_call_runs_and_its_result_feeds_the_second_round() {
    let gateway = spawn("gateway", vec![
        Step::Reply(chat_body(tool_call_message("call-1", "phone_status", "{}"), "auto")),
        Step::Reply(chat_body(text_message("Battery is 88 percent and discharging."), "auto")),
    ]);
    let local = spawn("local", vec![Step::HangUp]);
    let (agent, journal) = agent_with(gateway.port, local.port, 5, 4);

    let summary = agent.handle("how is the phone doing?");
    assert_eq!(summary.answer.as_deref(), Some("Battery is 88 percent and discharging."));
    assert_eq!(summary.error, None);
    assert_eq!(summary.brain.as_deref(), Some("homelab-gateway"));
    assert_eq!(gateway.hits(), 2, "the model must be asked again after the tool result");
    assert_eq!(local.hits(), 0, "a working gateway must not trigger the fallback");

    let events = journal.snapshot();
    let tags = tags(&events);
    assert_eq!(tags.first().map(String::as_str), Some("ag_ui:RUN_STARTED"));
    // A5: the power manager runs after the request, so power records may follow
    // the run. The run itself must still end with RUN_FINISHED.
    let finished = tags.iter().rposition(|t| t == "ag_ui:RUN_FINISHED").expect("the run must finish");
    let after_run: Vec<&str> = tags[finished + 1..].iter().map(String::as_str).collect();
    assert!(after_run.iter().all(|t| *t == "op6:power_change"), "only power records may follow a run: {after_run:?}");
    for expected in [
        "gen_ai:span_start",
        "gen_ai:span_end",
        "ag_ui:TOOL_CALL_START",
        "ag_ui:TOOL_CALL_ARGS",
        "ag_ui:TOOL_CALL_END",
        "ag_ui:TOOL_CALL_RESULT",
    ] {
        assert!(tags.iter().any(|t| t == expected), "missing {expected} in {tags:?}");
    }

    // The tool really read the fixture, and the model saw the same bytes.
    let results = find(&events, "\"battery_percent\":88");
    assert!(!results.is_empty(), "phone_status must return the fixture battery");
    let second_request = gateway.bodies()[1].clone();
    // The tool result travels back as a JSON *string*, so its quotes are escaped.
    assert!(second_request.contains("\"battery_percent\\\":88"), "the tool result must be fed back to the model:\n{second_request}");
    assert!(second_request.contains("\"tool_call_id\":\"call-1\""), "the result must be tied to the call id");
}

#[test]
fn malformed_tool_call_json_is_an_error_the_model_can_recover_from() {
    let gateway = spawn("gateway", vec![
        Step::Reply(chat_body(tool_call_message("call-bad", "get_time", "{not json at all"), "auto")),
        Step::Reply(chat_body(text_message("I could not read my own tool call."), "auto")),
    ]);
    let local = spawn("local", vec![Step::HangUp]);
    let (agent, journal) = agent_with(gateway.port, local.port, 5, 4);

    let summary = agent.handle("what time is it?");
    assert_eq!(summary.answer.as_deref(), Some("I could not read my own tool call."));
    assert_eq!(gateway.hits(), 2);

    let events = journal.snapshot();
    let errors = find(&events, "tool arguments are not valid JSON");
    assert!(!errors.is_empty(), "the malformed arguments must be named in the journal");
    assert!(find(&events, "\"status\":\"error\"").iter().any(|l| l.contains("execute_tool")), "the tool span must end in error");
    // The model is told what went wrong, not silently given an empty result.
    assert!(gateway.bodies()[1].contains("tool arguments are not valid JSON"));
}

#[test]
fn an_invented_tool_is_refused_by_name() {
    let gateway = spawn("gateway", vec![
        Step::Reply(chat_body(tool_call_message("call-x", "launch_missiles", "{}"), "auto")),
        Step::Reply(chat_body(text_message("I cannot do that."), "auto")),
    ]);
    let local = spawn("local", vec![Step::HangUp]);
    let (agent, journal) = agent_with(gateway.port, local.port, 5, 4);

    let summary = agent.handle("launch the missiles");
    assert_eq!(summary.answer.as_deref(), Some("I cannot do that."));
    let events = journal.snapshot();
    assert!(!find(&events, "unknown tool `launch_missiles`").is_empty(), "the refusal must name the tool");
}

#[test]
fn a_stalled_gateway_falls_back_to_the_phone_and_says_so() {
    let gateway = spawn("gateway", vec![Step::Stall(Duration::from_secs(5))]);
    let local = spawn("local", vec![Step::Reply(chat_body(text_message("Local model answering."), "gemma3-1b"))]);
    let (agent, journal) = agent_with(gateway.port, local.port, 1, 4);

    let summary = agent.handle("hello");
    assert_eq!(summary.answer.as_deref(), Some("Local model answering."));
    assert_eq!(summary.brain.as_deref(), Some("on-device-llama"));

    let events = journal.snapshot();
    let failed = find(&events, "\"destination\":\"homelab-gateway\"");
    assert!(!failed.is_empty(), "the failed brain must be journaled");
    assert!(failed.iter().any(|l| l.contains("model call timed out")), "the reason must be the timeout");
    let switched = find(&events, "\"destination\":\"on-device-llama\"");
    assert!(
        switched.iter().any(|l| l.contains("using on-device-llama")),
        "the fallback must be announced, not silent: {switched:?}"
    );
}

#[test]
fn a_dead_chain_ends_with_an_explicit_reason() {
    let gateway = spawn("gateway", vec![Step::Status(500, "upstream exploded".to_string())]);
    let local = spawn("local", vec![Step::HangUp]);
    let (agent, journal) = agent_with(gateway.port, local.port, 5, 4);

    let summary = agent.handle("hello");
    assert_eq!(summary.answer, None, "no answer is honest only if it says why");
    let error = summary.error.clone().expect("a dead chain must report an error");
    assert!(error.contains("HTTP 500"), "got: {error}");

    let events = journal.snapshot();
    let run_errors = find(&events, "\"type\":\"RUN_ERROR\"");
    assert_eq!(run_errors.len(), 1, "exactly one RUN_ERROR: {run_errors:?}");
    assert!(run_errors[0].contains("can't answer, here's why"), "got: {}", run_errors[0]);
    assert!(run_errors[0].contains("homelab-gateway") && run_errors[0].contains("on-device-llama"), "both brains must be named");
}

#[test]
fn the_answer_tool_ends_the_loop_without_another_model_call() {
    let gateway = spawn("gateway", vec![
        Step::Reply(chat_body(tool_call_message("call-a", "answer", "{\"text\":\"Done.\"}"), "auto")),
    ]);
    let local = spawn("local", vec![Step::HangUp]);
    let (agent, journal) = agent_with(gateway.port, local.port, 5, 4);

    let summary = agent.handle("say done");
    assert_eq!(summary.answer.as_deref(), Some("Done."));
    assert_eq!(gateway.hits(), 1, "`answer` must end the loop");

    let events = journal.snapshot();
    assert!(!find(&events, "\"type\":\"TEXT_MESSAGE_CONTENT\"").is_empty(), "the answer must be a text message");
    assert!(!find(&events, "\"type\":\"RUN_FINISHED\"").is_empty());
}

#[test]
fn a_model_that_loops_forever_hits_the_round_limit_with_a_reason() {
    let gateway = spawn("gateway", vec![
        Step::Reply(chat_body(tool_call_message("c1", "get_time", "{}"), "auto")),
        Step::Reply(chat_body(tool_call_message("c2", "get_time", "{}"), "auto")),
        Step::Stall(Duration::from_secs(5)),
    ]);
    let local = spawn("local", vec![Step::HangUp]);
    let (agent, journal) = agent_with(gateway.port, local.port, 5, 2);

    let summary = agent.handle("keep going");
    assert_eq!(summary.answer, None);
    assert!(summary.error.clone().unwrap_or_default().contains("tool round limit"));
    let events = journal.snapshot();
    assert!(!find(&events, "\"code\":\"round_limit\"").is_empty(), "the round limit must be in the journal");
}

#[test]
fn a_missing_key_file_skips_the_gateway_instead_of_failing_the_request() {
    let gateway = spawn("gateway", vec![Step::HangUp]);
    let local = spawn("local", vec![Step::Reply(chat_body(text_message("Phone model it is."), "gemma3-1b"))]);
    let journal = Journal::new(512);
    let text = format!(
        "gateway_url = \"http://127.0.0.1:{}/v1/chat/completions\"\n\
         gateway_model = \"auto\"\n\
         local_url = \"http://127.0.0.1:{}/v1/chat/completions\"\n\
         local_model = \"gemma3-1b\"\n\
         key_file = \"/nonexistent/agentic/litellm.key\"\n\
         model_timeout_sec = 5\n",
        gateway.port, local.port
    );
    let cfg = config::load(&text).expect("config");
    let agent = Agent::with_root(cfg, journal.clone(), probe_fixture());

    let summary = agent.handle("hello");
    assert_eq!(summary.answer.as_deref(), Some("Phone model it is."));
    assert_eq!(gateway.hits(), 0, "a brain with no key must not be called");
    let events = journal.snapshot();
    assert!(!find(&events, "key file /nonexistent/agentic/litellm.key").is_empty(), "the skip must be explained");
}

#[test]
fn every_event_carries_the_run_trace_id_and_the_journal_never_reuses_a_seq() {
    let gateway = spawn("gateway", vec![Step::Reply(chat_body(text_message("One."), "auto"))]);
    let local = spawn("local", vec![Step::HangUp]);
    let (agent, journal) = agent_with(gateway.port, local.port, 5, 2);
    agent.handle("first");
    agent.handle("second");

    let events = journal.snapshot();
    assert!(events.len() >= 6, "two runs should leave a real trail");
    let mut seqs: Vec<u64> = events.iter().map(|e| e.seq).collect();
    seqs.sort_unstable();
    let count = seqs.len();
    seqs.dedup();
    assert_eq!(seqs.len(), count, "sequence numbers must be unique");
    // Every run event belongs to a trace. The only records allowed to be
    // untraced are the power manager's, which are system events, not run events.
    for event in events.iter().filter(|e| e.trace_id.is_none()) {
        assert!(
            matches!(event.payload, Payload::Op6(Op6Event::PowerChange { .. })),
            "an untraced record must be a power change, got {:?}",
            event.payload
        );
    }

    let traces: Vec<_> = events.iter().map(|e| e.trace_id).collect();
    assert!(traces.windows(2).any(|w| w[0] != w[1]), "two requests must be two traces");
}

#[test]
fn the_model_is_given_a_completion_budget_a_reasoning_model_can_actually_use() {
    // On the phone the gateway answered with 512 completion tokens of reasoning
    // and no tool call at all, which surfaced as "empty reply from model". The
    // budget is ours to set, so it is set here and asserted in the body.
    let gateway = spawn("budget", vec![Step::Reply(chat_body(text_message("done"), "auto"))]);
    let (agent, _journal) = agent_with(gateway.port, 9, 10, 4);
    let summary = agent.handle("hi");
    assert_eq!(summary.answer.as_deref(), Some("done"));

    let body: Value = serde_json::from_str(&gateway.bodies()[0]).expect("the fake brain records a JSON body");
    let budget = body["max_tokens"].as_u64().unwrap_or(0);
    assert!(budget >= 1024, "512 was not enough for a reasoning model to reach its tool call, got {budget}: {body}");
}

#[test]
fn the_completion_budget_is_configurable() {
    let gateway = spawn("budget2", vec![Step::Reply(chat_body(text_message("done"), "auto"))]);
    let journal = Journal::new(64);
    let mut cfg = config_for(gateway.port, 9, 10, 4);
    cfg.max_completion_tokens = 4096;
    let agent = Agent::with_root(cfg, journal, probe_fixture());
    agent.handle("hi");
    let body: Value = serde_json::from_str(&gateway.bodies()[0]).expect("a JSON body");
    assert_eq!(body["max_tokens"], 4096, "the config value must reach the request");
}
