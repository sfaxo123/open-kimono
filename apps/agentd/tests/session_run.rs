//! CV1 conversation sessions: the store's carry-over / expiry /
//! isolation rules, and the run-level proof that a follow-up turn
//! reaches the brain carrying the session's earlier turns and its
//! destination entity — under a system prompt the session never edits.

mod common;

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use agentd::agent::Agent;
use agentd::config::{self, Config};
use agentd::journal::Journal;
use agentd::phone::ProbeRoot;
use agentd::server::{parse_follow_up, parse_request_full, valid_session_id};
use agentd::session::SessionStore;
use common::{chat_body, spawn, text_message, tool_call_message, Step};
use serde_json::Value;
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

/// `gio` does not exist as a working handler on the host, and a real
/// `app_launch` shells out to it. Stub it on PATH for this test binary
/// (each integration test file is its own process, so the mutation
/// cannot leak into another suite) so the tool call succeeds exactly
/// as it does on the phone.
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

fn session_contexts(events: &[Event]) -> Vec<Value> {
    events
        .iter()
        .filter_map(|e| match &e.payload {
            Payload::AgUi(AgUiEvent::Custom { name, value }) if name == "session_context" => Some(value.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn store_carry_over_expiry_and_isolation() {
    let store = SessionStore::with_ttl(Duration::from_millis(60));
    let t0 = Instant::now();

    // A new id begins fresh and empty.
    let ctx = store.begin("sess-a", t0);
    assert!(ctx.fresh);
    assert!(ctx.turns.is_empty() && ctx.entities.is_empty() && ctx.pending.is_empty());

    // Record a turn that established a destination and left a slot open.
    store.record(
        "sess-a",
        "open maps and give me directions to San Francisco",
        Some("Maps is open with San Francisco on the map.".to_string()),
        vec![("destination".to_string(), "San Francisco".to_string())],
        vec!["answer_question".to_string()],
        t0,
    );

    // The next turn of the same session carries all of it.
    let ctx = store.begin("sess-a", t0);
    assert!(!ctx.fresh);
    assert_eq!(ctx.turns.len(), 1);
    assert_eq!(ctx.turns[0].user, "open maps and give me directions to San Francisco");
    assert_eq!(ctx.turns[0].answer.as_deref(), Some("Maps is open with San Francisco on the map."));
    assert_eq!(ctx.entities.get("destination").map(String::as_str), Some("San Francisco"));
    assert_eq!(ctx.pending, vec!["answer_question".to_string()]);

    // A different id inherits nothing.
    let other = store.begin("sess-b", t0);
    assert!(other.fresh && other.turns.is_empty() && other.entities.is_empty());

    // The next recorded turn replaces the pending list (it consumed the
    // slot the previous turn left open).
    store.record("sess-a", "how long does it take?", Some("About 55 minutes.".to_string()), vec![], vec![], t0);
    let ctx = store.begin("sess-a", t0);
    assert_eq!(ctx.turns.len(), 2);
    assert!(ctx.pending.is_empty());
    assert_eq!(ctx.entities.get("destination").map(String::as_str), Some("San Francisco"));

    // After the TTL the session is gone whole: same id, fresh start.
    std::thread::sleep(Duration::from_millis(90));
    let t1 = Instant::now();
    assert!(!store.is_live("sess-a", t1));
    let ctx = store.begin("sess-a", t1);
    assert!(ctx.fresh && ctx.turns.is_empty() && ctx.entities.is_empty());
}

#[test]
fn follow_up_turn_reaches_the_brain_with_the_session_context() {
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
            Step::Reply(chat_body(text_message("Which place do you mean?"), "fake")),
        ],
    );
    let journal = Journal::new(512);
    let agent = Agent::with_root(config_for(brain.port), journal.clone(), probe_fixture());

    // Turn 1: the wake-word turn. It launches maps and answers.
    let s1 = agent.handle_session("open maps and give me directions to San Francisco", None, Some("sess-test-1"), false);
    assert_eq!(s1.answer.as_deref(), Some("Maps is open with San Francisco on the map."), "turn 1: {s1:?}");

    // Turn 2: the follow-up, no wake word, same session.
    let s2 = agent.handle_session("how long does it take to get there?", None, Some("sess-test-1"), true);
    assert_eq!(s2.answer.as_deref(), Some("About 55 minutes to San Francisco."), "turn 2: {s2:?}");

    // The brain's third request (turn 2, round 1) must carry the session:
    // the untouched system prompt, turn 1 as history, the destination
    // entity as background, and only then the follow-up itself.
    let bodies = brain.bodies();
    assert_eq!(bodies.len(), 3, "expected 3 brain calls, got {}", bodies.len());
    let payload: Value = serde_json::from_str(&bodies[2]).expect("brain request is JSON");
    let messages = payload["messages"].as_array().expect("messages array");
    assert_eq!(messages[0]["content"].as_str().unwrap(), agent.system_prompt(), "session state edited the system prompt");
    let contents: Vec<&str> = messages.iter().filter_map(|m| m["content"].as_str()).collect();
    assert!(contents.iter().any(|c| c.contains("open maps and give me directions to San Francisco")), "history user turn missing: {contents:?}");
    assert!(contents.iter().any(|c| c.contains("Maps is open with San Francisco on the map.")), "history assistant turn missing: {contents:?}");
    assert!(
        contents.iter().any(|c| c.contains("destination: San Francisco") && c.contains("not a new request")),
        "entity background note missing: {contents:?}"
    );
    assert_eq!(messages.last().unwrap()["content"].as_str(), Some("how long does it take to get there?"));

    // The journal tells the same story: two session_context records, the
    // second continuing the first with its turn and entity in hand.
    let contexts = session_contexts(&journal.snapshot());
    assert_eq!(contexts.len(), 2, "contexts: {contexts:?}");
    assert_eq!(contexts[0]["fresh"], Value::Bool(true));
    assert_eq!(contexts[0]["prior_turns"], Value::from(0));
    assert_eq!(contexts[0]["follow_up"], Value::Bool(false));
    assert_eq!(contexts[1]["fresh"], Value::Bool(false));
    assert_eq!(contexts[1]["prior_turns"], Value::from(1));
    assert_eq!(contexts[1]["follow_up"], Value::Bool(true));
    assert_eq!(contexts[1]["entities"]["destination"].as_str(), Some("San Francisco"));

    // Isolation at the run level: the same question in a NEW session
    // reaches the brain with no history and no San Francisco anywhere.
    let s3 = agent.handle_session("how long does it take to get there?", None, Some("sess-test-2"), false);
    assert_eq!(s3.answer.as_deref(), Some("Which place do you mean?"));
    let bodies = brain.bodies();
    let payload: Value = serde_json::from_str(&bodies[3]).expect("brain request is JSON");
    let messages = payload["messages"].as_array().expect("messages array");
    assert_eq!(messages.len(), 2, "a fresh session sends system + the request only: {messages:?}");
    // No message may carry the old session: not its history, not its
    // entity note. (The string "San Francisco" legitimately appears in
    // the app_launch tool DESCRIPTION in every request — that is the
    // tool catalogue, not session state, so the check is per-message.)
    for m in messages.iter() {
        let c = m["content"].as_str().unwrap_or("");
        assert!(!c.contains("Maps is open"), "new session inherited history: {c}");
        assert!(!c.contains("destination: San Francisco"), "new session inherited the entity: {c}");
        assert!(!c.contains("Background from earlier"), "new session inherited the entity note: {c}");
    }
    let contexts = session_contexts(&journal.snapshot());
    assert_eq!(contexts.len(), 3);
    assert_eq!(contexts[2]["fresh"], Value::Bool(true));
    assert_eq!(contexts[2]["session"], Value::from("sess-test-2"));
}

#[test]
fn request_and_follow_up_wire_shapes() {
    // The CV1 fields ride /request as optional extras; their absence is
    // the old wire exactly.
    let f = parse_request_full("{\"text\": \"hello\"}").expect("plain request");
    assert_eq!(f.text, "hello");
    assert!(f.confirm.is_none() && f.session.is_none() && !f.follow_up);

    let f = parse_request_full("{\"text\": \"hi\", \"session\": \"sess-1730000000-1\", \"follow_up\": true}").expect("session request");
    assert_eq!(f.session.as_deref(), Some("sess-1730000000-1"));
    assert!(f.follow_up);

    for bad in [
        "{\"text\": \"hi\", \"session\": \"has space\"}",
        "{\"text\": \"hi\", \"session\": \"\"}",
        "{\"text\": \"hi\", \"follow_up\": \"yes\"}",
    ] {
        assert!(parse_request_full(bad).is_err(), "accepted a bad body: {bad}");
    }
    let long = format!("{{\"text\": \"hi\", \"session\": \"{}\"}}", "a".repeat(65));
    assert!(parse_request_full(&long).is_err(), "accepted a 65-char session id");

    assert!(valid_session_id("sess-1730000000-1"));
    assert!(!valid_session_id("sess/1"));
    assert!(!valid_session_id(""));

    // /follow-up takes exactly the two window transitions.
    assert_eq!(parse_follow_up("{\"session\": \"sess-1\", \"state\": \"open\"}").unwrap(), ("sess-1".to_string(), "open".to_string()));
    assert_eq!(parse_follow_up("{\"session\": \"sess-1\", \"state\": \"closed\"}").unwrap(), ("sess-1".to_string(), "closed".to_string()));
    for bad in [
        "{\"session\": \"sess-1\", \"state\": \"ajar\"}",
        "{\"session\": \"sess-1\"}",
        "{\"state\": \"open\"}",
        "not json",
        "",
    ] {
        assert!(parse_follow_up(bad).is_err(), "accepted a bad follow-up body: {bad}");
    }
}
