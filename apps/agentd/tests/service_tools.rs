//! The Lego layer: services the model composes, proven in simulation before
//! they are allowed to run, and recorded rather than performed.

use std::fs;
use std::thread::sleep;
use std::time::Duration;

mod common;

use agentd::agent::Agent;
use agentd::config;
use agentd::ir_help;
use agentd::journal::Journal;
use agentd::services::ServiceHub;
use agentd::tools::{self, ToolCtx, ToolError};
use common::{chat_body, spawn, text_message, tool_call_message, Step};
use agentd::phone::ProbeRoot;
use serde_json::{json, Value};

fn hub() -> ServiceHub {
    ServiceHub::start(Journal::new(4096), Default::default())
}

fn water_ir(id: &str, every_sec: u64, expected_reminders: u64) -> Value {
    let mut ir: Value = serde_json::from_str(ir_help::WATER_REMINDER).expect("example 1 must be JSON");
    ir["id"] = json!(id);
    ir["state_schema"]["schema"]["properties"]["every_sec"]["minimum"] = json!(1);
    ir["state_schema"]["initial_state"]["every_sec"] = json!(every_sec);
    ir["tests"] = json!([{
        "name": "interval_fires_once",
        "description": "One interval elapses and exactly one reminder is counted.",
        "initial_env": {
            "monotonic_ms": 0,
            "wall_iso": "2026-11-02T12:00:00Z",
            "battery_pct": 80.0,
            "charging": false,
            "network_online": false,
            "location": { "lat": 45.5152, "lon": -122.6784, "tz": "America/Los_Angeles" }
        },
        "events": [{ "at_monotonic_ms": every_sec * 1000, "event_type": "advance_clock", "payload": {} }],
        "assertions": {
            "final_state_subset": { "reminders_sent": expected_reminders },
            "expected_bus_emissions": [],
            "expected_ui_surfaces": [],
            "budget_assertions": { "max_memory_mb_used": 2.0, "network_bytes_sent": 0, "active_schemes_after_completion": [] }
        }
    }]);
    ir
}

fn journal_text(journal: &Journal) -> String {
    journal.jsonl().expect("the journal must serialize")
}

#[test]
fn both_prompt_examples_are_services_that_validate_and_pass_their_own_tests() {
    let h = hub();
    for (name, text) in [("example 1", ir_help::WATER_REMINDER), ("example 2", ir_help::BATTERY_LOW_NOTICE)] {
        let ir: Value = serde_json::from_str(text).unwrap_or_else(|e| panic!("{name} is not JSON: {e}"));
        let summary = h.create(&ir);
        assert!(summary.get("error").is_none(), "{name} refused: {summary}");
        assert_eq!(summary["status"], "running", "{name} did not start: {summary}");
        let tests = summary["tests"].as_array().expect("the summary lists its tests");
        assert!(!tests.is_empty(), "{name} shipped without tests");
        for test in tests {
            assert_eq!(test["passed"], true, "{name} test {} failed: {test}", test["name"]);
        }
    }
    let listed = h.list();
    assert_eq!(listed["count"], 2, "both services must be installed: {listed}");
}

#[test]
fn an_ir_the_validator_refuses_is_named_by_field_path() {
    let h = hub();
    // A value serde itself cannot read is refused with the bad value named.
    let mut ir = water_ir("svc.demo.bad_trigger", 3600, 1);
    ir["triggers"][0]["kind"] = json!("telepathy");
    let error = h.create(&ir)["error"].as_str().expect("a refusal must carry an error").to_string();
    assert!(error.contains("unknown variant"), "got: {error}");
    assert!(error.contains("telepathy"), "the bad value must be named: {error}");

    // A value serde accepts but the validator forbids is refused by field path.
    let mut ir = water_ir("svc.demo.bad_cap", 3600, 1);
    ir["capabilities"][1]["capability"] = json!("ui.notify");
    let error = h.create(&ir)["error"].as_str().expect("a refusal must carry an error").to_string();
    assert!(error.contains("/capabilities/1/capability"), "expected a field path, got: {error}");
    assert!(error.contains("ui.notify"), "the bad capability must be named: {error}");
    assert_eq!(h.list()["count"], 0, "a refused IR must not be installed");
}

#[test]
fn a_service_that_fails_its_own_simulation_never_runs() {
    let h = hub();
    // The IR is legal; only its own assertion is wrong (it claims 99 reminders).
    let ir = water_ir("svc.demo.wishful", 3600, 99);
    let reply = h.create(&ir);
    let error = reply["error"].as_str().expect("a refusal must carry an error");
    assert!(error.contains("does not pass its own simulation"), "got: {error}");
    assert!(error.contains("interval_fires_once"), "the failing test must be named: {error}");
    let tests = reply["tests"].as_array().expect("the refusal reports every test");
    assert_eq!(tests[0]["passed"], false);
    assert_eq!(h.list()["count"], 0, "simulation failure means no install, ever");
}

#[test]
fn installing_the_same_id_twice_is_refused_with_a_way_forward() {
    let h = hub();
    let ir = water_ir("svc.demo.duplicate", 3600, 1);
    assert!(h.create(&ir).get("error").is_none(), "first install must succeed");
    let second = h.create(&ir);
    let error = second["error"].as_str().expect("the duplicate must be refused");
    assert!(error.contains("already installed"), "got: {error}");
    assert!(error.contains("stop it first"), "the refusal must say what to do: {error}");
    assert_eq!(h.list()["count"], 1, "exactly one copy may be running");
}

#[test]
fn list_describe_and_stop_round_trip_through_the_executor() {
    let h = hub();
    let created = h.create(&water_ir("svc.demo.round_trip", 3600, 1));
    assert_eq!(created["id"], "svc.demo.round_trip");
    assert_eq!(created["admitted_by"], "validate, then simulate, then admit");

    let listed = h.list();
    let services = listed["services"].as_array().unwrap();
    assert_eq!(services.len(), 1);
    assert_eq!(services[0]["status"]["status"], "running", "svcd status shape: {listed}");
    assert_eq!(services[0]["state"]["reminders_sent"], 0);
    assert!(services[0]["power"].as_str().unwrap().contains("Water Reminder"));

    let described = h.describe("svc.demo.round_trip");
    assert_eq!(described["id"], "svc.demo.round_trip");
    assert_eq!(described["ir"]["id"], "svc.demo.round_trip", "describe must return the real IR");
    assert_eq!(described["status"]["status"], "running");

    let stopped = h.stop("svc.demo.round_trip", "the test is done");
    assert_eq!(stopped["id"], "svc.demo.round_trip");
    let status = stopped["status"].as_str().expect("a stop reports a status");
    assert!(status.starts_with("stopped"), "got: {stopped}");

    let after = h.list();
    assert_eq!(after["services"][0]["status"]["reason"], "the test is done", "the reason must survive: {after}");
    // The lifecycle_stop trigger ran, so the service said so in its own state.
    assert_eq!(after["services"][0]["state"]["status"], "stopped");
}

#[test]
fn an_unknown_service_id_is_refused_by_name() {
    let h = hub();
    for reply in [h.describe("svc.nope.missing"), h.stop("svc.nope.missing", "because")] {
        let error = reply["error"].as_str().expect("unknown id must be refused");
        assert!(error.contains("svc.nope.missing"), "the id must be named: {error}");
    }
}

#[test]
fn a_timer_that_comes_due_fires_on_the_real_clock_and_is_journaled() {
    let journal = Journal::new(4096);
    let h = ServiceHub::start(journal.clone(), Default::default());
    // every_sec = 1, so the interval is due one second after install.
    let created = h.create(&water_ir("svc.demo.fast_water", 1, 1));
    assert!(created.get("error").is_none(), "install must succeed: {created}");

    sleep(Duration::from_millis(2500));
    h.tick();

    let listed = h.list();
    let fired = listed["services"][0]["state"]["reminders_sent"].as_u64().unwrap_or(0);
    assert!(fired >= 1, "the interval must have fired by now: {listed}");

    let text = journal_text(&journal);
    assert!(text.contains("svc.effect_requested"), "the reminder must be in the journal:\n{text}");
    assert!(text.contains("\"performed\":false"), "the effect must be recorded, not performed:\n{text}");
    assert!(text.contains("\"dry_run\":true"), "and it must say so:\n{text}");
    assert!(text.contains("Time to drink some water."), "the reminder text must be there:\n{text}");
    assert!(text.contains("\"service\":\"svcd\""), "svcd's own records must be mirrored:\n{text}");
    assert!(!text.contains("\"kind\":\"gen_ai\""), "a deterministic service must never journal an LLM span");
}

#[test]
fn the_service_tools_say_so_when_the_layer_is_not_running() {
    let root = ProbeRoot(std::env::temp_dir());
    let ctx = ToolCtx::new(&root, None, None);
    for (name, args) in [
        ("service_create", json!({"ir": {}})),
        ("service_list", json!({})),
        ("service_describe", json!({"id": "svc.x.y"})),
        ("service_stop", json!({"id": "svc.x.y"})),
    ] {
        let parsed = tools::parse_arguments(&args.to_string()).unwrap();
        let err = tools::execute(name, &parsed, &ctx).expect_err("no hub means a refusal");
        assert!(matches!(err, ToolError::BadArguments(_)), "{name}: {err:?}");
        assert!(err.reason().contains("not running"), "{name}: {}", err.reason());
    }
}

#[test]
fn service_tools_refuse_arguments_that_are_not_there() {
    let root = ProbeRoot(std::env::temp_dir());
    let h = hub();
    let ctx = ToolCtx::new(&root, Some(&h), None);

    let err = tools::execute("service_create", &json!({}), &ctx).expect_err("no IR, no service");
    assert!(err.reason().contains("field `ir`"), "got: {err:?}");
    let err = tools::execute("service_create", &json!({"ir": "a string"}), &ctx).expect_err("IR must be an object");
    assert!(err.reason().contains("field `ir`"), "got: {err:?}");
    let err = tools::execute("service_describe", &json!({}), &ctx).expect_err("describe needs an id");
    assert!(err.reason().contains("`id`"), "got: {err:?}");
    let err = tools::execute("service_list", &json!({"extra": 1}), &ctx).expect_err("list takes nothing");
    assert!(err.reason().contains("unexpected argument `extra`"), "got: {err:?}");
}

#[test]
fn the_prompt_forbids_narrating_instead_of_acting() {
    // On the phone the 5090 model answered "I'll build the service…" and stopped,
    // so the demo run created nothing. The prompt has to close that door.
    let dir = std::env::temp_dir().join(format!("agentd-prompt-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("fixture dir");
    let cfg = "listen = \"127.0.0.1:8796\"\ngateway_url = \"http://127.0.0.1:1/v1/chat/completions\"\ngateway_model = \"fake\"\nkey_file = \"/dev/null\"\n";
    let agent = Agent::with_root(config::load(cfg).expect("a valid config"), Journal::new(64), ProbeRoot(dir));
    let text = agent.system_prompt();
    assert!(text.contains("Acting is a tool call, never a promise"), "the prompt must forbid narrating: {text}");
    assert!(text.contains("call the tool that does it in this turn"), "and name the alternative: {text}");
}

#[test]
fn the_prompt_carries_the_shape_and_both_examples() {
    let guide = ir_help::section();
    for needle in [
        "ir_version",
        "idempotency_key_template",
        "timer.monotonic.schedule",
        "sensor_threshold",
        "svc.demo.water_reminder",
        "svc.demo.battery_low_notice",
        "EXAMPLE 1",
        "EXAMPLE 2",
    ] {
        assert!(guide.contains(needle), "the prompt is missing {needle:?}");
    }
    assert!(guide.contains(ir_help::WATER_REMINDER.trim()), "example 1 must be embedded verbatim");
    assert!(guide.contains(ir_help::BATTERY_LOW_NOTICE.trim()), "example 2 must be embedded verbatim");
}

// — the whole path: a request in, a service installed —

/// An agent pointed at a scripted fake brain, with its own journal and probes.
fn agent_with_gateway(port: u16, tag: &str) -> (Agent, Journal) {
    let dir = std::env::temp_dir().join(format!("agentd-e2e-{}-{}", std::process::id(), tag));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("fixture dir");
    fs::write(dir.join("litellm.key"), "not-a-real-key\n").expect("key file");
    let text = format!(
        "listen = \"127.0.0.1:8795\"\ngateway_url = \"http://127.0.0.1:{port}/v1/chat/completions\"\ngateway_model = \"fake\"\nkey_file = \"{}\"\nmodel_timeout_sec = 5\nmax_tool_rounds = 4\nidle_timeout_sec = 120\n",
        dir.join("litellm.key").display()
    );
    let journal = Journal::new(4096);
    let agent = Agent::with_root(config::load(&text).expect("a valid config"), journal.clone(), ProbeRoot(dir));
    (agent, journal)
}

#[test]
fn a_request_can_install_a_service_all_the_way_through() {
    let ir: Value = serde_json::from_str(ir_help::WATER_REMINDER).expect("example 1 is JSON");
    let args = json!({ "ir": ir }).to_string();
    let gateway = spawn("svc-install", vec![
        Step::Reply(chat_body(tool_call_message("call-1", "service_create", &args), "fake")),
        Step::Reply(chat_body(text_message("Your water reminder is running."), "fake")),
    ]);
    let (agent, journal) = agent_with_gateway(gateway.port, "install");

    // Installing commits a block to the phone, so the request confirms
    // exactly this install (A1.2 broker gate); without it nothing runs.
    let summary = agent.handle_with_confirm(
        "Remind me to drink water every hour.",
        Some("service_create svc.demo.water_reminder"),
    );
    assert_eq!(summary.answer.as_deref(), Some("Your water reminder is running."));
    assert_eq!(summary.error, None, "the run must not fail: {summary:?}");

    let listed = agent.services.list();
    assert_eq!(listed["count"], 1, "the service must really be installed: {listed}");
    assert_eq!(listed["services"][0]["id"], "svc.demo.water_reminder");
    assert_eq!(listed["services"][0]["status"]["status"], "running");

    let text = journal.jsonl().expect("the journal must serialize");
    assert!(text.contains("service_create"), "the tool call must be in the journal");
    assert!(text.contains("\"service\":\"svcd\""), "svcd's own records must be mirrored into the run");
    assert!(text.contains("svc.demo.water_reminder"), "the service id must be visible");
}

#[test]
fn a_broken_ir_comes_back_to_the_model_as_a_named_refusal() {
    let gateway = spawn("svc-broken", vec![
        Step::Reply(chat_body(tool_call_message("call-1", "service_create", "{\"ir\": {}}"), "fake")),
        Step::Reply(chat_body(text_message("I could not create it: the ServiceIR was empty."), "fake")),
    ]);
    let (agent, _journal) = agent_with_gateway(gateway.port, "broken");

    // An IR with no id carries the generic label; confirming it lets the
    // call reach the validator, whose refusal is what this test is about.
    let summary = agent.handle_with_confirm("Make me a water reminder.", Some("service_create"));
    assert_eq!(gateway.hits(), 2, "the refusal must be fed back, not swallowed");
    assert_eq!(summary.answer.as_deref(), Some("I could not create it: the ServiceIR was empty."));
    assert_eq!(agent.services.list()["count"], 0, "a refused IR must install nothing");

    let bodies = gateway.bodies();
    assert!(bodies[1].contains("not a ServiceIR"), "the model must be told why: {}", bodies[1]);
}
