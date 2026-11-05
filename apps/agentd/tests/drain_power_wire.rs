//! W3 (A4.1 / A4.2 / A4.GATE): the power manager samples the real battery
//! (fake sysfs + injected clock here), feeds `Drain`, and journals a
//! `power_recommendation` through `power_policy` citing the sample count
//! and window. With no measured baseline there is no recommendation —
//! only the honest line saying so. Radios stay dry-run either way.

mod common;

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use agentd::agent::Agent;
use agentd::config::{self, Config};
use agentd::journal::Journal;
use agentd::phone::ProbeRoot;
use agentd::power::{FakeClock, PowerManager};
use common::{chat_body, spawn, text_message, tool_call_message, Step};

fn fixture(tag: &str, capacity: u64) -> ProbeRoot {
    let dir = std::env::temp_dir().join(format!("agentd-drain-{}-{}", std::process::id(), tag));
    let _ = fs::remove_dir_all(&dir);
    for sub in ["sys/class/power_supply/bq27411-0", "sys/class/net/wlan0", "sys/class/bluetooth/hci0", "proc/net"] {
        fs::create_dir_all(dir.join(sub)).expect("fixture dir");
    }
    fs::write(dir.join("sys/class/power_supply/bq27411-0/capacity"), format!("{capacity}\n")).unwrap();
    fs::write(dir.join("sys/class/power_supply/bq27411-0/status"), "Discharging\n").unwrap();
    fs::write(dir.join("sys/class/net/wlan0/operstate"), "up\n").unwrap();
    fs::write(dir.join("sys/class/bluetooth/hci0/state"), "up\n").unwrap();
    fs::write(
        dir.join("proc/net/tcp"),
        "  sl  local_address rem_address   st\n  0: 0100007F:1F90 00000000:0000 0A\n",
    )
    .unwrap();
    fs::write(dir.join("proc/uptime"), "12345.67 40000.00\n").unwrap();
    fs::write(dir.join("proc/meminfo"), "MemTotal:  7800000 kB\nMemAvailable:  4500000 kB\n").unwrap();
    fs::write(dir.join("proc/loadavg"), "1.00 1.00 1.00 2/300 999\n").unwrap();
    fs::write(dir.join("proc/net/dev"), "face | bytes\nlo: 100\nwlan0: 200\n").unwrap();
    ProbeRoot(dir)
}

fn set_capacity(root: &ProbeRoot, pct: u64) {
    fs::write(root.path().join("sys/class/power_supply/bq27411-0/capacity"), format!("{pct}\n")).unwrap();
}

fn manager(root: ProbeRoot, clock: Arc<FakeClock>) -> (Journal, PowerManager) {
    let journal = Journal::new(4096);
    let pm = PowerManager::with_clock(journal.clone(), root, Duration::from_secs(60), Some(8080), clock);
    (journal, pm)
}

/// Two one-hour drops of 4 points each: 88 → 84 → 80.
fn measure_two_hours(root: &ProbeRoot, pm: &PowerManager, clock: &FakeClock) {
    pm.tick(None);
    set_capacity(root, 84);
    clock.advance(3_600_000);
    pm.tick(None);
    set_capacity(root, 80);
    clock.advance(3_600_000);
    pm.tick(None);
}

#[test]
fn a_measured_battery_drop_produces_a_cited_recommendation() {
    let root = fixture("drop", 88);
    let clock = Arc::new(FakeClock::default());
    let (journal, pm) = manager(root.clone(), clock.clone());
    measure_two_hours(&root, &pm, &clock);

    let text = journal.jsonl().expect("journal serializes");
    assert!(text.contains("power_recommendation"), "the recommendation must be journaled:\n{text}");
    assert!(text.contains("\"samples\":2"), "it must cite the sample count:\n{text}");
    assert!(text.contains("\"window_ms\":3600000"), "and the measured window:\n{text}");
    assert!(text.contains("PROPOSE OFF bluetooth"), "bluetooth is on, unneeded, switchable:\n{text}");
    assert!(text.contains("battery at ~4.0%/hr over 2 sample(s) spanning 1.0h"), "the rate, count and window in words:\n{text}");
    // A proposal is not an action: the radio is still on.
    let status = pm.status(None);
    let bluetooth = status["components"].as_array().unwrap().iter().find(|c| c["component"] == "bluetooth").unwrap();
    assert_eq!(bluetooth["state"], "on", "dry-run means nothing was switched: {status}");
}

#[test]
fn negative_no_baseline_means_no_recommendation_only_the_honest_line() {
    let root = fixture("flat", 88);
    let clock = Arc::new(FakeClock::default());
    let (journal, pm) = manager(root, clock);
    pm.tick(None);
    pm.tick(None);

    let text = journal.jsonl().expect("journal serializes");
    assert!(!text.contains("power_recommendation"), "no baseline, no recommendation event:\n{text}");
    assert!(!text.contains("PROPOSE OFF"), "and certainly no proposal:\n{text}");
    let plan = pm.plan(None);
    assert_eq!(plan["drain"]["samples"], 0, "{plan}");
    assert!(
        plan["drain"]["note"].as_str().unwrap().contains("no measured baseline — no recommendation"),
        "the plan must say why there is no recommendation: {plan}"
    );
}

#[test]
fn a_preapproved_candidate_is_marked_auto_but_radios_stay_dry_run() {
    let root = fixture("preapproved", 88);
    let clock = Arc::new(FakeClock::default());
    let (journal, pm) = manager(root.clone(), clock.clone());
    pm.set_preapproved(["bluetooth".to_string()]);
    measure_two_hours(&root, &pm, &clock);

    let text = journal.jsonl().expect("journal serializes");
    assert!(text.contains("OFF bluetooth (pre-approved, dry-run only)"), "pre-approval changes the wording, not the honesty:\n{text}");
    assert!(text.contains("\"auto_switch\":true"), "{text}");
    let status = pm.status(None);
    let bluetooth = status["components"].as_array().unwrap().iter().find(|c| c["component"] == "bluetooth").unwrap();
    assert_eq!(bluetooth["state"], "on", "even pre-approved, agentd never touches a radio: {status}");
}

#[test]
fn negative_charging_records_no_drain_sample() {
    let root = fixture("charging", 80);
    fs::write(root.path().join("sys/class/power_supply/bq27411-0/status"), "Charging\n").unwrap();
    let clock = Arc::new(FakeClock::default());
    let (_journal, pm) = manager(root.clone(), clock.clone());
    pm.tick(None);
    set_capacity(&root, 84);
    clock.advance(3_600_000);
    pm.tick(None);
    assert_eq!(pm.plan(None)["drain"]["samples"], 0, "a charging battery is not draining");
}

#[test]
fn the_preapproval_list_comes_from_the_config_file() {
    let text = "listen = \"127.0.0.1:8791\"\n\
                local_url = \"http://127.0.0.1:8080/v1/chat/completions\"\n\
                local_model = \"gemma3-1b\"\n\
                power_preapproved = \"wifi, bluetooth\"\n";
    let cfg: Config = config::load(text).expect("config with preapprovals must load");
    assert_eq!(cfg.power_preapproved, vec!["wifi".to_string(), "bluetooth".to_string()]);

    let journal = Journal::new(64);
    let agent = Agent::with_root(cfg, journal, fixture("cfgagent", 88));
    assert!(agent.power.preapproved().contains(&"wifi".to_string()), "agentd must hand the list to its power manager");
    assert!(agent.power.preapproved().contains(&"bluetooth".to_string()));
}

#[test]
fn negative_power_plan_through_a_request_reports_no_baseline() {
    let gateway = spawn("drain-req", vec![
        Step::Reply(chat_body(tool_call_message("c1", "power_plan", "{}"), "auto")),
        Step::Reply(chat_body(text_message("No measured baseline yet, so no recommendation."), "auto")),
    ]);
    let key = std::env::temp_dir().join(format!("agentd-drain-key-{}", std::process::id()));
    fs::write(&key, "test-key-not-a-secret\n").unwrap();
    let text = format!(
        "listen = \"127.0.0.1:8791\"\n\
         gateway_url = \"http://127.0.0.1:{}/v1/chat/completions\"\n\
         gateway_model = \"auto\"\n\
         key_file = \"{}\"\n\
         model_timeout_sec = 5\n",
        gateway.port,
        key.display()
    );
    let cfg: Config = config::load(&text).expect("config");
    let journal = Journal::new(512);
    let agent = Agent::with_root(cfg, journal.clone(), fixture("req", 88));

    let summary = agent.handle("Should I turn anything off to save battery?");
    assert_eq!(summary.answer.as_deref(), Some("No measured baseline yet, so no recommendation."));
    assert!(
        gateway.bodies()[1].contains("no measured baseline — no recommendation"),
        "the model must read the honest line, not an invented rate: {}",
        gateway.bodies()[1]
    );
    assert!(
        journal.jsonl().unwrap().contains("no measured baseline — no recommendation"),
        "and it must be in the trace the user sees"
    );
}
