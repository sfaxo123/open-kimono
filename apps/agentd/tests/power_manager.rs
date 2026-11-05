//! A5 — the power manager: what is on, who needs it, what should be off, and
//! what agentd is actually allowed to switch.

use std::fs;
use std::process::Command;
use std::sync::Arc;
use std::thread::sleep;
use std::time::Duration;

use agentd::ir_help;
use agentd::journal::Journal;
use agentd::phone::ProbeRoot;
use agentd::power::{FakeClock, PowerManager};
use agentd::services::ServiceHub;
use agentd::tools::{self, ToolCtx, ToolError};
use serde_json::{json, Value};

/// A probe tree that looks like the phone: Wi-Fi up, Bluetooth up, a local
/// brain listening on 8080, and deliberately **no** backlight file.
fn probe(tag: &str) -> ProbeRoot {
    let dir = std::env::temp_dir().join(format!("agentd-power-{}-{}", std::process::id(), tag));
    let _ = fs::remove_dir_all(&dir);
    for sub in ["sys/class/power_supply/bq27411-0", "sys/class/net/wlan0", "sys/class/bluetooth/hci0", "proc/net"] {
        fs::create_dir_all(dir.join(sub)).expect("fixture dir");
    }
    fs::write(dir.join("sys/class/power_supply/bq27411-0/capacity"), "88\n").unwrap();
    fs::write(dir.join("sys/class/power_supply/bq27411-0/status"), "Discharging\n").unwrap();
    fs::write(dir.join("sys/class/net/wlan0/operstate"), "up\n").unwrap();
    fs::write(dir.join("sys/class/bluetooth/hci0/state"), "up\n").unwrap();
    // One LISTEN (`0A`) socket on 127.0.0.1:8080 (0x1F90) and one ESTABLISHED.
    fs::write(
        dir.join("proc/net/tcp"),
        "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n\
          0: 0100007F:1F90 00000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 111 1 0 10 0\n\
          1: 0100007F:C001 0200007F:1F90 01 00000000:00000000 00:00000000 00000000     0        0 222 1 0 10 0\n",
    )
    .unwrap();
    fs::write(dir.join("proc/uptime"), "12345.67 40000.00\n").unwrap();
    fs::write(dir.join("proc/meminfo"), "MemTotal:       7 800 000 kB\nMemAvailable:   4 500 000 kB\n").unwrap();
    fs::write(dir.join("proc/loadavg"), "1.00 1.00 1.00 2/300 999\n").unwrap();
    fs::write(dir.join("proc/net/dev"), "face | bytes\nlo: 100\nwlan0: 200\n").unwrap();
    ProbeRoot(dir)
}

fn manager(root: ProbeRoot, idle_sec: u64) -> (Journal, PowerManager) {
    let journal = Journal::new(4096);
    let pm = PowerManager::new(journal.clone(), root, Duration::from_secs(idle_sec), Some(8080));
    (journal, pm)
}

fn power_change_lines(journal: &Journal) -> Vec<Value> {
    let text = journal.jsonl().expect("the journal must serialize");
    text.lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|e| e["payload"]["kind"] == "op6" && e["payload"]["type"] == "power_change")
        .map(|e| e["payload"].clone())
        .collect()
}

fn plan_for<'a>(plan: &'a Value, component: &str) -> &'a Value {
    plan["plan"]
        .as_array()
        .expect("a plan is a list")
        .iter()
        .find(|e| e["component"] == component)
        .unwrap_or_else(|| panic!("no plan entry for {component}: {plan}"))
}

#[test]
fn every_component_is_read_from_a_probe_and_reported_with_its_source() {
    let (_j, pm) = manager(probe("read"), 999);
    let status = pm.status(None);
    assert_eq!(status["read_only"], true);
    let get = |name: &str| -> Value {
        status["components"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["component"] == name)
            .cloned()
            .unwrap_or_else(|| panic!("no component {name}: {status}"))
    };
    assert_eq!(get("wifi")["state"], "on");
    assert_eq!(get("wifi")["source"], "sys/class/net/wlan0/operstate");
    assert_eq!(get("bluetooth")["state"], "on");
    // The local model is judged by a LISTEN line in /proc/net/tcp, never by
    // connecting to it.
    assert_eq!(get("local_model")["state"], "on");
    assert!(get("local_model")["source"].as_str().unwrap().contains("port 8080"));
    assert_eq!(get("services")["state"], "off");
    // No backlight file exists here, so the honest answer is unknown.
    assert_eq!(get("screen")["state"], "unknown");
    assert!(get("screen")["source"].as_str().unwrap().starts_with("not readable"), "got {}", get("screen")["source"]);
    // Radios and the screen are never agentd's to switch.
    for name in ["screen", "bluetooth", "wifi", "services"] {
        assert_eq!(get(name)["agentd_can_switch"], false, "{name} must not be switchable");
    }
    assert_eq!(get("local_model")["agentd_can_switch"], true, "agentd's own child is its own to stop");
}

#[test]
fn a_probe_that_is_not_there_is_unknown_never_off() {
    let root = probe("missing");
    fs::remove_file(root.path().join("sys/class/bluetooth/hci0/state")).unwrap();
    let (_j, pm) = manager(root, 999);
    let status = pm.status(None);
    let bluetooth = status["components"].as_array().unwrap().iter().find(|c| c["component"] == "bluetooth").unwrap();
    assert_eq!(bluetooth["state"], "unknown", "an unreadable probe is not evidence of off");
    assert!(bluetooth["source"].as_str().unwrap().contains("not readable"));
}

#[test]
fn a_local_model_that_is_not_listening_is_off_not_guessed_on() {
    let root = probe("nolisten");
    fs::write(root.path().join("proc/net/tcp"), "  sl  local_address rem_address   st\n  0: 0100007F:1F91 00000000:0000 0A\n").unwrap();
    let (_j, pm) = manager(root, 999);
    let status = pm.status(None);
    let model = status["components"].as_array().unwrap().iter().find(|c| c["component"] == "local_model").unwrap();
    assert_eq!(model["state"], "off");
}

#[test]
fn after_the_idle_timeout_radios_are_proposed_not_touched() {
    let (_j, pm) = manager(probe("idle"), 1);
    sleep(Duration::from_millis(1300));
    let plan = pm.plan(None);
    assert!(plan["idle_seconds"].as_u64().unwrap() >= 1, "the idle clock is real: {plan}");
    for component in ["wifi", "bluetooth"] {
        let entry = plan_for(&plan, component);
        assert_eq!(entry["action"], "propose_off", "{component}: {entry}");
        assert_eq!(entry["dry_run"], true, "{component} must stay a proposal");
        assert!(entry["command"].as_str().unwrap().len() > 3, "the operator needs a real command: {entry}");
        assert!(entry["note"].as_str().unwrap().contains("does not touch"), "say who acts: {entry}");
    }
    // The one thing agentd may do is stop its own child.
    assert_eq!(plan_for(&plan, "local_model")["action"], "off_by_agentd");
    // And a service is never power's to stop, idle or not.
    assert_eq!(plan_for(&plan, "services")["action"], "leave_alone", "stopping a service is the user's call");
    assert_eq!(plan["children_started_by_agentd"], 0, "nothing was started, so nothing was stopped");
}

#[test]
fn a_request_in_flight_keeps_what_it_needs() {
    let (_j, pm) = manager(probe("flight"), 1);
    pm.request_started();
    sleep(Duration::from_millis(1300));
    let plan = pm.plan(None);
    assert_eq!(plan["request_in_flight"], true);
    for component in ["wifi", "local_model"] {
        let entry = plan_for(&plan, component);
        assert_eq!(entry["action"], "keep", "{component} must be kept while a request runs: {entry}");
        assert!(entry["because"].as_str().unwrap().contains("in flight"), "{entry}");
    }
    pm.request_finished();
    sleep(Duration::from_millis(1100));
    assert_eq!(plan_for(&pm.plan(None), "wifi")["action"], "propose_off", "the clock restarts, it does not stop");
}

#[test]
fn a_running_service_is_a_reason_to_keep_the_network_up() {
    let journal = Journal::new(4096);
    let hub = ServiceHub::start(journal.clone(), Default::default());
    let ir: Value = serde_json::from_str(ir_help::WATER_REMINDER).unwrap();
    assert!(hub.create(&ir).get("error").is_none(), "the service must install");
    let pm = PowerManager::new(journal, probe("service"), Duration::from_secs(1), Some(8080));
    sleep(Duration::from_millis(1300));
    let plan = pm.plan(Some(&hub));
    let wifi = plan_for(&plan, "wifi");
    assert_eq!(wifi["action"], "keep", "a live service may need the network: {wifi}");
    assert!(wifi["because"].as_str().unwrap().contains("service"), "{wifi}");
    let services = plan_for(&plan, "services");
    assert_eq!(services["state"], "on");
    assert!(services["because"].as_str().unwrap().contains("svc.demo.water_reminder"));
    // Power never stops a service: that is the user's call.
    assert_ne!(services["action"], "propose_off");
    assert_ne!(services["action"], "off_by_agentd");
}

#[test]
fn agentd_stops_its_own_child_when_it_goes_idle_and_says_why() {
    let (journal, pm) = manager(probe("child"), 1);
    let child = Command::new("sleep").arg("120").spawn().expect("spawn a stand-in child");
    let pid = child.id();
    pm.note_child("local-model", child);
    assert_eq!(pm.status(None)["children_started_by_agentd"], 1);

    sleep(Duration::from_millis(1300));
    pm.tick(None);

    // The process is really gone.
    let gone = Command::new("kill")
        .args(["-0", &pid.to_string()])
        .status()
        .map(|s| !s.success())
        .unwrap_or(true);
    assert!(gone, "child {pid} should have been stopped");
    assert_eq!(pm.status(None)["children_started_by_agentd"], 0);

    let changes = power_change_lines(&journal);
    let off = changes.iter().find(|p| p["component"] == "local_model" && p["state"] == "off");
    let off = off.unwrap_or_else(|| panic!("no local_model off event in {changes:?}"));
    assert!(off["reason"].as_str().unwrap().contains("idle"), "the reason must name the idle window: {off}");
    assert!(off["reason"].as_str().unwrap().contains("own child"), "and say what was stopped: {off}");
}

#[test]
fn a_transition_is_journaled_once_not_on_every_tick() {
    let (journal, pm) = manager(probe("once"), 1);
    sleep(Duration::from_millis(1300));
    for _ in 0..4 {
        pm.tick(None);
        sleep(Duration::from_millis(50));
    }
    let changes = power_change_lines(&journal);
    let bluetooth_off = changes.iter().filter(|p| p["component"] == "bluetooth" && p["state"] == "off").count();
    assert_eq!(bluetooth_off, 1, "four ticks must not produce four events: {changes:?}");
    let reason = changes.iter().find(|p| p["component"] == "bluetooth").unwrap()["reason"].as_str().unwrap();
    assert!(reason.contains("proposed for the operator"), "the reason must not imply it was done: {reason}");
}

#[test]
fn the_power_tools_are_read_only_and_refuse_stray_arguments() {
    let root = probe("tools");
    let (journal, pm) = manager(root.clone(), 1);
    let hub = ServiceHub::start(journal.clone(), Default::default());
    let ctx = ToolCtx::new(&root, Some(&hub), Some(&pm));

    for name in ["power_status", "power_plan"] {
        let parsed = tools::parse_arguments("{}").unwrap();
        let before = journal.len();
        let outcome = tools::execute(name, &parsed, &ctx).unwrap_or_else(|e| panic!("{name} failed: {e:?}"));
        assert_eq!(outcome.terminal, false);
        assert_eq!(journal.len(), before, "{name} must not journal anything");
    }
    let err = tools::execute("power_plan", &json!({"component": "wifi"}), &ctx).expect_err("power_plan takes no arguments");
    assert!(matches!(err, ToolError::BadArguments(_)), "{err:?}");
    assert!(err.reason().contains("unexpected argument `component`"), "{err:?}");
}

#[test]
fn the_power_tools_say_so_when_the_manager_is_absent() {
    let root = probe("nohub");
    let ctx = ToolCtx::new(&root, None, None);
    for name in ["power_status", "power_plan"] {
        let err = tools::execute(name, &json!({}), &ctx).expect_err("no manager means a refusal");
        assert!(err.reason().contains("power manager is not running"), "{name}: {err:?}");
    }
}

#[test]
fn the_advertised_power_tools_are_the_ones_that_run() {
    let names: Vec<String> = tools::definitions()
        .iter()
        .filter_map(|d| d["function"]["name"].as_str().map(str::to_string))
        .collect();
    assert!(names.contains(&"power_status".to_string()) && names.contains(&"power_plan".to_string()), "{names:?}");
    let root = probe("advertised");
    let hub = ServiceHub::start(Journal::new(64), Default::default());
    let pm = PowerManager::new(Journal::new(64), root.clone(), Duration::from_secs(999), None);
    let ctx = ToolCtx::new(&root, Some(&hub), Some(&pm));
    for name in ["power_status", "power_plan"] {
        let parsed = tools::parse_arguments("{}").unwrap();
        assert!(tools::execute(name, &parsed, &ctx).is_ok(), "{name} must run");
    }
    // With no local brain configured the local model is unknown, not off.
    let status = pm.status(None);
    let model = status["components"].as_array().unwrap().iter().find(|c| c["component"] == "local_model").unwrap();
    assert_eq!(model["state"], "unknown");
    assert!(model["source"].as_str().unwrap().contains("no local_url"), "{}", model["source"]);
}

/// Is this pid still alive? Used to prove the idle gate really protects a child.
fn alive(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[test]
fn nothing_is_switched_before_the_idle_timeout_and_something_is_after() {
    // The whole point of the idle timeout is the boundary, so the boundary is
    // tested at the millisecond on a clock the test drives — not by sleeping.
    let journal = Journal::new(4096);
    let clock = Arc::new(FakeClock::default());
    let pm = PowerManager::with_clock(
        journal.clone(),
        probe("gate"),
        Duration::from_secs(60),
        Some(8080),
        clock.clone(),
    );

    // One millisecond short of the window: everything waits.
    clock.advance(59_999);
    let plan = pm.plan(None);
    assert_eq!(plan["idle_seconds"], 59);
    for name in ["bluetooth", "wifi", "local_model", "screen", "services"] {
        assert_eq!(plan_for(&plan, name)["action"], "wait", "{name} must wait out the window: {plan}");
    }
    pm.tick(None);
    let before = power_change_lines(&journal);
    assert!(!before.is_empty(), "the first tick still announces what it sees");
    // `services` may honestly be announced off — there are none running and
    // nobody switched anything. Nothing that was seen *on* may flip inside the
    // window: that is the gate this test exists to catch.
    let switched: Vec<&Value> = before
        .iter()
        .filter(|p| p["state"] == "off" && p["component"] != "services")
        .collect();
    assert!(switched.is_empty(), "nothing that is on may be reported off inside the window: {before:?}");
    assert!(
        before.iter().all(|p| p["reason"].as_str().unwrap_or("").contains("still held")),
        "the reason must say it is holding, not acting: {before:?}"
    );

    // Exactly at the window — the gate is `>=`, so this is the first tick that acts.
    clock.advance(1);
    let plan = pm.plan(None);
    assert_eq!(plan["idle_seconds"], 60);
    assert_eq!(plan_for(&plan, "bluetooth")["action"], "propose_off");
    assert_eq!(plan_for(&plan, "wifi")["action"], "propose_off");
    assert_eq!(plan_for(&plan, "local_model")["action"], "off_by_agentd");
    assert_eq!(plan_for(&plan, "screen")["action"], "cannot_tell", "an unreadable screen is never proposed off");
    assert_eq!(plan_for(&plan, "services")["action"], "leave_alone");

    let announced = power_change_lines(&journal).len();
    pm.tick(None);
    // Only what the crossing itself added.
    let all = power_change_lines(&journal);
    let crossed: Vec<&Value> = all[announced..].iter().collect();
    let names: Vec<&str> = crossed.iter().map(|p| p["component"].as_str().unwrap_or("?")).collect();
    assert_eq!(
        names,
        vec!["bluetooth", "wifi"],
        "crossing the window must journal exactly the two radios it can only propose: {crossed:?}"
    );
    assert!(
        crossed.iter().all(|p| p["state"] == "off" && p["reason"].as_str().unwrap_or("").contains("idle 60s")),
        "each must be off, with the window it crossed named: {crossed:?}"
    );
    assert!(
        crossed.iter().all(|p| p["reason"].as_str().unwrap_or("").contains("proposed for the operator")),
        "a radio is a proposal, never an action: {crossed:?}"
    );
}

#[test]
fn its_own_child_survives_until_the_window_closes_and_dies_after_it() {
    let journal = Journal::new(4096);
    let clock = Arc::new(FakeClock::default());
    let pm = PowerManager::with_clock(
        journal.clone(),
        probe("childgate"),
        Duration::from_secs(60),
        Some(8080),
        clock.clone(),
    );
    let child = Command::new("sleep").arg("120").spawn().expect("spawn sleep");
    let pid = child.id();
    pm.note_child("test-child", child);

    clock.advance(59_000);
    pm.tick(None);
    assert!(alive(pid), "a child must survive inside the idle window");
    assert_eq!(pm.status(None)["children_started_by_agentd"], 1);

    clock.advance(2_000);
    pm.tick(None);
    assert!(!alive(pid), "the child must be stopped once the window has passed");
    assert_eq!(pm.status(None)["children_started_by_agentd"], 0);
    let changes = power_change_lines(&journal);
    let off = changes
        .iter()
        .find(|p| p["component"] == "local_model" && p["state"] == "off")
        .unwrap_or_else(|| panic!("no local_model off event in {changes:?}"));
    assert!(off["reason"].as_str().unwrap().contains("idle 61s"), "the reason must name the real idle time: {off}");
}

#[test]
fn a_child_is_never_killed_while_a_request_is_in_flight() {
    // Found by mutation: deleting the in-flight guard left every test green.
    // A request that is talking to the local model must not have that model
    // pulled out from under it mid-sentence, however long it has been idle.
    let journal = Journal::new(4096);
    let clock = Arc::new(FakeClock::default());
    let pm = PowerManager::with_clock(
        journal.clone(),
        probe("inflightsafe"),
        Duration::from_secs(1),
        Some(8080),
        clock.clone(),
    );
    let child = Command::new("sleep").arg("120").spawn().expect("spawn sleep");
    let pid = child.id();
    pm.note_child("local-model", child);

    pm.request_started();
    clock.advance(10_000); // far past the 1 s window, but a request is running
    pm.tick(None);
    assert!(alive(pid), "a child must survive a tick while a request is in flight");
    assert_eq!(pm.status(None)["children_started_by_agentd"], 1);
    let changes = power_change_lines(&journal);
    assert!(
        !changes.iter().any(|p| p["component"] == "local_model" && p["state"] == "off"),
        "no off event may be journaled for a child that is still running: {changes:?}"
    );

    // The request ends; the next tick may clean up.
    pm.request_finished();
    clock.advance(1_000);
    pm.tick(None);
    assert!(!alive(pid), "once the request is over, the idle child is stopped");
    let changes = power_change_lines(&journal);
    assert!(
        changes.iter().any(|p| p["component"] == "local_model" && p["state"] == "off"),
        "and that is journaled: {changes:?}"
    );
}

#[test]
fn the_screen_probe_finds_the_backlight_wherever_the_driver_put_it() {
    // The OnePlus 6 on pmOS exposes `ae94000.dsi.0/brightness`, not
    // `lcd/brightness`. A probe that only knows one name reports `unknown` for a
    // screen that is plainly lit.
    let root = probe("backlight");
    fs::create_dir_all(root.path().join("sys/class/backlight/ae94000.dsi.0")).unwrap();
    fs::write(root.path().join("sys/class/backlight/ae94000.dsi.0/brightness"), "106\n").unwrap();
    let (_j, pm) = manager(root.clone(), 999);
    let screen = pm.status(None)["components"].as_array().unwrap().iter().find(|c| c["component"] == "screen").unwrap().clone();
    assert_eq!(screen["state"], "on", "a lit backlight is on: {screen}");
    assert_eq!(screen["source"], "sys/class/backlight/ae94000.dsi.0/brightness");

    fs::write(root.path().join("sys/class/backlight/ae94000.dsi.0/brightness"), "0\n").unwrap();
    let (_j2, pm2) = manager(root, 999);
    let screen = pm2.status(None)["components"].as_array().unwrap().iter().find(|c| c["component"] == "screen").unwrap().clone();
    assert_eq!(screen["state"], "off", "brightness 0 is off: {screen}");
}

#[test]
fn a_blocked_bluetooth_controller_is_off_not_unknown() {
    // On the phone `hci0/state` does not exist because rfkill has the controller
    // blocked. That is a fact about the radio, not a missing probe.
    let root = probe("rfkill");
    fs::remove_file(root.path().join("sys/class/bluetooth/hci0/state")).unwrap();
    fs::create_dir_all(root.path().join("sys/class/rfkill/rfkill1")).unwrap();
    fs::write(root.path().join("sys/class/rfkill/rfkill1/type"), "bluetooth\n").unwrap();
    fs::write(root.path().join("sys/class/rfkill/rfkill1/state"), "0\n").unwrap();
    let (_j, pm) = manager(root.clone(), 999);
    let bluetooth = pm.status(None)["components"].as_array().unwrap().iter().find(|c| c["component"] == "bluetooth").unwrap().clone();
    assert_eq!(bluetooth["state"], "off", "a blocked controller is off: {bluetooth}");
    assert!(bluetooth["source"].as_str().unwrap().contains("rfkill"), "and the source says so: {bluetooth}");

    fs::write(root.path().join("sys/class/rfkill/rfkill1/state"), "1\n").unwrap();
    let (_j2, pm2) = manager(root, 999);
    let bluetooth = pm2.status(None)["components"].as_array().unwrap().iter().find(|c| c["component"] == "bluetooth").unwrap().clone();
    assert_eq!(bluetooth["state"], "on", "unblocked is on: {bluetooth}");
}
