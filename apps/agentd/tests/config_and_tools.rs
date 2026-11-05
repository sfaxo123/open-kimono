//! Config and tool unit tests — the cheap negatives: a typo in a phone config,
//! a half-configured brain chain, an https URL we do not support, and tool
//! calls with wrong or invented arguments.

use std::io::Write;

use agentd::config;
use agentd::journal::Journal;
use agentd::phone::ProbeRoot;
use agentd::power::PowerManager;
use agentd::services::ServiceHub;
use agentd::tools;

const GOOD: &str = "listen = \"127.0.0.1:8770\"\n\
                    gateway_url = \"http://MESH_HOST:4000/v1/chat/completions\"\n\
                    gateway_model = \"auto\"\n\
                    local_url = \"http://127.0.0.1:8080/v1/chat/completions\"\n\
                    local_model = \"gemma3-1b\"\n\
                    key_file = \"$HOME/agentic/litellm.key\"\n";

#[test]
fn a_good_config_produces_the_ordered_brain_chain() {
    let cfg = config::load(GOOD).expect("good config must load");
    assert_eq!(cfg.listen, "127.0.0.1:8770");
    assert_eq!(cfg.brains.len(), 2);
    assert_eq!(cfg.brains[0].name, "homelab-gateway");
    assert_eq!(cfg.brains[0].model, "auto");
    assert!(cfg.brains[0].needs_key);
    assert_eq!(cfg.brains[1].name, "on-device-llama");
    assert!(!cfg.brains[1].needs_key);
    assert_eq!(cfg.model_timeout_sec, 30, "defaults apply to keys that are absent");
}

#[test]
fn config_mistakes_are_named_and_point_at_the_line() {
    let cases: Vec<(String, &str)> = vec![
        (GOOD.replace("gateway_model", "gateway_mode"), "unknown key `gateway_mode`"),
        (GOOD.replace("listen = \"127.0.0.1:8770\"", "listen = \"127.0.0.1:oops\""), "port is not a number"),
        (GOOD.replace("listen = \"127.0.0.1:8770\"", "listen = \"127.0.0.1:0\""), "port must be non-zero"),
        (GOOD.replace("listen = \"127.0.0.1:8770\"", "listen 8770"), "expected `key = value`"),
        (GOOD.replace("gateway_model = \"auto\"", ""), "`gateway_url` and `gateway_model` must both be set"),
        (GOOD.replace("local_url = \"http://127.0.0.1:8080/v1/chat/completions\"\n", ""), "`local_url` and `local_model` must both be set"),
        (GOOD.replace("http://MESH_HOST:4000/v1/chat/completions", "https://example.com/v1/chat/completions"), "only plain http:// is supported"),
        (GOOD.replace("/v1/chat/completions", "/chat"), "URL path must start with `v1/`"),
        (format!("{GOOD}model_timeout_sec = \"thirty\"\n"), "must be a non-negative integer"),
        (GOOD.replace("gateway_url = \"http://MESH_HOST:4000/v1/chat/completions\"", "gateway_url = http://MESH_HOST:4000/v1/chat/completions"), "must be a quoted string"),
    ];
    for (text, expected) in cases {
        let err = config::load(&text).expect_err(&format!("config should be rejected, expected {expected:?}: {text}"));
        assert!(err.to_string().contains(&expected), "expected {expected:?}, got: {err}");
    }
}

#[test]
fn a_config_with_no_brain_at_all_is_refused() {
    let err = config::load("listen = \"127.0.0.1:8770\"\nkey_file = \"/x\"\n")
        .expect_err("agentd with no brain cannot answer anything");
    assert!(err.to_string().contains("no brain configured"), "got: {err}");
}

#[test]
fn a_key_file_of_only_comments_is_an_error_not_an_empty_token() {
    let path = std::env::temp_dir().join(format!("agentd-empty-key-{}", std::process::id()));
    let mut f = std::fs::File::create(&path).unwrap();
    writeln!(f, "# no key here\n\n   \n").unwrap();
    let err = config::read_key(&path).expect_err("a comment-only key file has no key");
    assert!(err.contains("no key line found"), "got: {err}");

    let path2 = std::env::temp_dir().join(format!("agentd-good-key-{}", std::process::id()));
    std::fs::write(&path2, "  sk-real-looking  \n# trailing comment\n").unwrap();
    assert_eq!(config::read_key(&path2).expect("read key"), "sk-real-looking");
}

// — tools —

/// A hub with no seeded sensors: the service tools still work, the battery
/// trigger just starts from "no sample yet" like a cold phone.
fn hub() -> ServiceHub {
    ServiceHub::start(Journal::new(256), Default::default())
}

fn ctx_with_hub<'a>(root: &'a ProbeRoot, h: &'a ServiceHub) -> tools::ToolCtx<'a> {
    tools::ToolCtx::new(root, Some(h), None)
}

#[test]
fn the_clock_formats_utc_correctly() {
    assert_eq!(tools::iso8601_utc(0), "1970-01-01T00:00:00Z");
    assert_eq!(tools::iso8601_utc(946_684_800), "2000-01-01T00:00:00Z");
    assert_eq!(tools::iso8601_utc(1_709_209_845), "2024-02-29T12:30:45Z", "leap day");
    assert_eq!(tools::iso8601_utc(1_793_577_600), "2026-11-02T00:00:00Z");
}

#[test]
fn get_time_and_phone_status_read_the_phone_not_the_model() {
    let dir = std::env::temp_dir().join(format!("agentd-tools-probe-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("sys/class/power_supply/bq27411-0")).unwrap();
    std::fs::write(dir.join("sys/class/power_supply/bq27411-0/capacity"), "42\n").unwrap();
    let root = ProbeRoot(dir);

    let time = tools::execute("get_time", &serde_json::json!({}), &ctx_with_hub(&root, &hub())).expect("get_time");
    assert!(!time.terminal);
    assert!(time.content["utc"].as_str().unwrap().ends_with('Z'));

    let status = tools::execute("phone_status", &serde_json::json!({}), &ctx_with_hub(&root, &hub())).expect("phone_status");
    assert_eq!(status.content["battery_percent"], 42);
    assert_eq!(status.content["read_only"], true, "the tool must declare itself read-only");
}

#[test]
fn tool_arguments_are_checked_before_anything_runs() {
    let root = ProbeRoot(std::env::temp_dir());

    let err = tools::execute("answer", &serde_json::json!({}), &ctx_with_hub(&root, &hub())).expect_err("answer needs text");
    assert!(err.reason().contains("non-empty string field `text`"), "got: {err:?}");

    let err = tools::execute("answer", &serde_json::json!({"text": "   "}), &ctx_with_hub(&root, &hub())).expect_err("blank text is not an answer");
    assert!(err.reason().contains("non-empty string field `text`"), "got: {err:?}");

    let err = tools::execute("get_time", &serde_json::json!({"format": "epoch"}), &ctx_with_hub(&root, &hub())).expect_err("invented arguments must be refused");
    assert!(err.reason().contains("unexpected argument `format`"), "got: {err:?}");

    let err = tools::execute("phone_status", &serde_json::json!("a string"), &ctx_with_hub(&root, &hub())).expect_err("arguments must be an object");
    assert!(err.reason().contains("must be a JSON object"), "got: {err:?}");

    let err = tools::execute("sudo_reboot", &serde_json::json!({}), &ctx_with_hub(&root, &hub())).expect_err("no such tool may exist");
    assert_eq!(err.reason(), "unknown tool `sudo_reboot`");
}

#[test]
fn malformed_argument_strings_are_a_distinct_error() {
    let err = tools::parse_arguments("{not json").expect_err("must not parse");
    assert!(matches!(err, tools::ToolError::MalformedArguments(_)), "got: {err:?}");
    assert!(err.reason().contains("not valid JSON"));

    // Empty means "no arguments", which is legal for the no-arg tools.
    assert_eq!(tools::parse_arguments("").unwrap(), serde_json::json!({}));
    assert_eq!(tools::parse_arguments("  {} ").unwrap(), serde_json::json!({}));
}

#[test]
fn the_tool_list_advertises_only_tools_that_exist() {
    let defs = tools::definitions();
    assert!(!defs.is_empty());
    let names: Vec<&str> = defs
        .iter()
        .map(|d| d["function"]["name"].as_str().expect("each definition needs a name"))
        .collect();
    let root = ProbeRoot(std::env::temp_dir());
    let h = hub();
    let pm = PowerManager::new(Journal::new(64), root.clone(), std::time::Duration::from_secs(999), None);
    let ctx = tools::ToolCtx::new(&root, Some(&h), Some(&pm));
    for name in &names {
        // The arguments each advertised tool needs to be callable at all.
        let args = match *name {
            "service_create" => serde_json::json!({"ir": {}}),
            "service_describe" | "service_stop" => serde_json::json!({"id": "svc.nope.missing"}),
            "app_launch" => serde_json::json!({"app": "calculator", "destination": "San Francisco"}),
            _ => serde_json::json!({}),
        };
        let outcome = tools::execute(name, &args, &ctx);
        match *name {
            // `answer` needs text; the service tools refuse an empty IR and an
            // unknown id. Both refusals must be *named*, not crashes.
            // `app_launch` refuses an app that is not on the allowlist.
            "answer" => assert!(matches!(outcome, Err(tools::ToolError::BadArguments(_))), "{name}: {outcome:?}"),
            "service_create" | "service_describe" | "service_stop" | "app_launch" => {
                assert!(matches!(outcome, Err(tools::ToolError::BadArguments(_))), "{name}: {outcome:?}");
            }
            _ => assert!(outcome.is_ok(), "advertised tool {name} does not run: {outcome:?}"),
        }
    }
}
