//! Agent-emitted module manifests: a successful app_launch emits a
//! destination card built from the actual tool result, session turns
//! emit a session card from SessionStore state, and neither a refused
//! launch nor a sessionless run invents a card it has no state for.

mod common;

use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use agentd::agent::Agent;
use agentd::config::{self, Config};
use agentd::journal::Journal;
use agentd::modules;
use agentd::phone::ProbeRoot;
use agentd::server::Server;
use common::{chat_body, spawn, text_message, tool_call_message, Step};
use serde_json::Value;
use trace_events::{AgUiEvent, Event, Payload};

fn unique(tag: &str) -> PathBuf {
    let thread: String = format!("{:?}", std::thread::current().id())
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    std::env::temp_dir().join(format!("agentd-modules-{tag}-{}-{thread}", std::process::id()))
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

/// Every `module_manifest` CUSTOM value in the journal, in order.
fn emitted(events: &[Event]) -> Vec<Value> {
    events
        .iter()
        .filter_map(|e| match &e.payload {
            Payload::AgUi(AgUiEvent::Custom { name, value }) if name == "module_manifest" => Some(value.clone()),
            _ => None,
        })
        .collect()
}

fn by_id(values: &[Value], id: &str) -> Vec<Value> {
    values.iter().filter(|v| v["manifest"]["id"].as_str() == Some(id)).cloned().collect()
}

fn assert_valid(entry: &Value) {
    let manifest = &entry["manifest"];
    let inputs = &entry["inputs"];
    assert_eq!(modules::validate_manifest(manifest), Vec::<String>::new(), "emitted manifest must validate: {manifest}");
    assert_eq!(modules::missing_required_inputs(manifest, inputs), Vec::<String>::new(), "emitted card must have every required value: {entry}");
    let purpose = manifest["user_purpose"].as_str().unwrap_or("");
    assert!(!purpose.trim().is_empty(), "every card needs a user_purpose");
}

#[test]
fn destination_card_after_successful_app_launch_is_built_from_the_tool_result() {
    stub_gio();
    let brain = spawn(
        "gateway",
        vec![
            Step::Reply(chat_body(
                tool_call_message("call-1", "app_launch", "{\"app\":\"maps\",\"destination\":\"San Francisco\"}"),
                "fake",
            )),
            Step::Reply(chat_body(text_message("Maps is open with San Francisco on the map."), "fake")),
        ],
    );
    let journal = Journal::new(512);
    let agent = Agent::with_root(config_for(brain.port), journal.clone(), probe_fixture());
    let summary = agent.handle_session("open maps and give me directions to San Francisco", None, Some("sess-mod-1"), false);
    assert_eq!(summary.answer.as_deref(), Some("Maps is open with San Francisco on the map."));

    let all = emitted(&journal.snapshot());
    let dest = by_id(&all, modules::DESTINATION_CARD_ID);
    assert_eq!(dest.len(), 1, "exactly one destination card per successful launch: {all:?}");
    assert_valid(&dest[0]);
    let inputs = &dest[0]["inputs"];
    assert_eq!(inputs["destination"], Value::from("San Francisco"));
    assert_eq!(inputs["app"], Value::from("maps"));
    assert_eq!(inputs["uri"], Value::from("geo:0,0?q=San%20Francisco"), "uri must come from the tool result");
    assert_eq!(inputs["run_id"], Value::from(summary.run_id.as_str()), "card must link back to its run");
    assert_eq!(dest[0]["manifest"]["actions"][0]["tool"], Value::from("app_launch"));
    assert_eq!(dest[0]["manifest"]["actions"][0]["args"]["app"], Value::from("maps"));

    // The same turn also emitted the session card for its session.
    let sess = by_id(&all, modules::SESSION_CARD_ID);
    assert_eq!(sess.len(), 1, "one recorded turn, one session card emission: {all:?}");
    assert_valid(&sess[0]);
    assert_eq!(sess[0]["inputs"]["session_id"], Value::from("sess-mod-1"));
    assert_eq!(sess[0]["inputs"]["turn_count"], Value::from(1));
    assert_eq!(sess[0]["inputs"]["follow_up_window"], Value::from("closed"));
    assert_eq!(sess[0]["inputs"]["pending"], Value::from("none"));
}

#[test]
fn a_refused_app_launch_emits_no_destination_card() {
    let brain = spawn(
        "gateway",
        vec![
            Step::Reply(chat_body(
                tool_call_message("call-1", "app_launch", "{\"app\":\"calculator\",\"destination\":\"San Francisco\"}"),
                "fake",
            )),
            Step::Reply(chat_body(text_message("I cannot open that app."), "fake")),
        ],
    );
    let journal = Journal::new(512);
    let agent = Agent::with_root(config_for(brain.port), journal.clone(), probe_fixture());
    let summary = agent.handle("open calculator");
    assert!(summary.answer.is_some());
    let dest = by_id(&emitted(&journal.snapshot()), modules::DESTINATION_CARD_ID);
    assert!(dest.is_empty(), "a refused launch must emit no destination card: {dest:?}");
}

#[test]
fn session_card_updates_as_the_session_changes() {
    stub_gio();
    let brain = spawn(
        "gateway",
        vec![
            Step::Reply(chat_body(
                tool_call_message("call-1", "app_launch", "{\"app\":\"maps\",\"destination\":\"San Francisco\"}"),
                "fake",
            )),
            Step::Reply(chat_body(text_message("Maps is open."), "fake")),
            Step::Reply(chat_body(text_message("Which place do you mean?"), "fake")),
        ],
    );
    let journal = Journal::new(512);
    let agent = Agent::with_root(config_for(brain.port), journal.clone(), probe_fixture());
    agent.handle_session("open maps to San Francisco", None, Some("sess-mod-2"), false);
    agent.handle_session("how long does it take?", None, Some("sess-mod-2"), true);

    let sess = by_id(&emitted(&journal.snapshot()), modules::SESSION_CARD_ID);
    assert_eq!(sess.len(), 2, "one session card per recorded turn: {sess:?}");
    for s in &sess {
        assert_valid(s);
    }
    assert_eq!(sess[0]["inputs"]["turn_count"], Value::from(1));
    assert_eq!(sess[1]["inputs"]["turn_count"], Value::from(2), "second emission must reflect the second turn");
    assert_eq!(sess[1]["inputs"]["pending"], Value::from("answer_question"), "a turn ending in a question leaves that pending item");
}

#[test]
fn sessionless_run_emits_destination_but_no_session_card() {
    stub_gio();
    let brain = spawn(
        "gateway",
        vec![
            Step::Reply(chat_body(
                tool_call_message("call-1", "app_launch", "{\"app\":\"maps\",\"destination\":\"San Francisco\"}"),
                "fake",
            )),
            Step::Reply(chat_body(text_message("Maps is open."), "fake")),
        ],
    );
    let journal = Journal::new(512);
    let agent = Agent::with_root(config_for(brain.port), journal.clone(), probe_fixture());
    agent.handle("open maps to San Francisco");
    let all = emitted(&journal.snapshot());
    assert_eq!(by_id(&all, modules::DESTINATION_CARD_ID).len(), 1);
    assert!(by_id(&all, modules::SESSION_CARD_ID).is_empty(), "no session, no session card — never invented");
}

fn free_port() -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").expect("probe bind");
    let p = l.local_addr().unwrap().port();
    drop(l);
    p
}

fn send(port: u16, request: &str) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    stream.write_all(request.as_bytes()).expect("write");
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(_) => break,
        }
    }
    String::from_utf8_lossy(&buf).to_string()
}

#[test]
fn follow_up_window_report_reemits_the_session_card_with_the_new_state() {
    stub_gio();
    let brain = spawn(
        "gateway",
        vec![
            Step::Reply(chat_body(
                tool_call_message("call-1", "app_launch", "{\"app\":\"maps\",\"destination\":\"San Francisco\"}"),
                "fake",
            )),
            Step::Reply(chat_body(text_message("Maps is open."), "fake")),
        ],
    );
    let port = free_port();
    let local = free_port();
    let key = key_file();
    let text = format!(
        "listen = \"127.0.0.1:{port}\"\n\
         gateway_url = \"http://127.0.0.1:{}/v1/chat/completions\"\n\
         gateway_model = \"auto\"\n\
         local_url = \"http://127.0.0.1:{local}/v1/chat/completions\"\n\
         local_model = \"fake\"\n\
         key_file = \"{}\"\n\
         model_timeout_sec = 5\n\
         max_tool_rounds = 4\n",
        brain.port,
        key.display()
    );
    let cfg: Config = config::load(&text).expect("config");
    let journal = Journal::new(512);
    let agent = Arc::new(Agent::with_root(cfg.clone(), journal.clone(), probe_fixture()));
    let server = Server { config: cfg, agent: agent.clone(), journal: journal.clone() };
    thread::spawn(move || {
        let _ = server.serve();
    });
    for _ in 0..100 {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }

    let body = "{\"text\":\"open maps to San Francisco\",\"session\":\"sess-mod-3\"}";
    let resp = send(port, &format!("POST /request HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", body.len()));
    assert!(resp.contains("200"), "request run: {resp}");

    let before = by_id(&emitted(&journal.snapshot()), modules::SESSION_CARD_ID);
    assert_eq!(before.len(), 1);
    assert_eq!(before[0]["inputs"]["follow_up_window"], Value::from("closed"));

    let fu = "{\"session\":\"sess-mod-3\",\"state\":\"open\"}";
    let resp = send(port, &format!("POST /follow-up HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{fu}", fu.len()));
    assert!(resp.contains("200"), "follow-up report: {resp}");

    let after = by_id(&emitted(&journal.snapshot()), modules::SESSION_CARD_ID);
    assert_eq!(after.len(), 2, "the window change must re-emit the session card: {after:?}");
    assert_valid(&after[1]);
    assert_eq!(after[1]["inputs"]["follow_up_window"], Value::from("open"));
    assert_eq!(after[1]["inputs"]["turn_count"], Value::from(1), "window report is not a turn");
}
