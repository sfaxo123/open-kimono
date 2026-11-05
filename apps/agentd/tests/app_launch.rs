//! app_launch refusals — everything that must be refused is refused
//! before anything runs, so these are host tests: no gio, no phone.
//! The happy path (a real `gio open` on the phone) is covered by the
//! KEY-TEST-1 phone smoke, not here.

use agentd::launch;
use agentd::phone::ProbeRoot;
use agentd::tools;

fn ctx(root: &ProbeRoot) -> tools::ToolCtx<'_> {
    tools::ToolCtx::new(root, None, None)
}

#[test]
fn app_launch_is_in_the_tool_definitions() {
    let names: Vec<String> = tools::definitions()
        .iter()
        .filter_map(|d| d["function"]["name"].as_str().map(str::to_string))
        .collect();
    assert!(names.contains(&"app_launch".to_string()), "{names:?}");
}

#[test]
fn non_maps_apps_are_refused_before_anything_runs() {
    let root = ProbeRoot(std::env::temp_dir());
    let err = tools::execute(
        "app_launch",
        &serde_json::json!({"app": "calculator", "destination": "San Francisco"}),
        &ctx(&root),
    )
    .expect_err("only maps may launch");
    assert!(err.reason().contains("allowlist"), "got: {err:?}");
}

#[test]
fn missing_and_stray_arguments_are_refused() {
    let root = ProbeRoot(std::env::temp_dir());
    let err = tools::execute("app_launch", &serde_json::json!({"app": "maps"}), &ctx(&root))
        .expect_err("destination is required");
    assert!(err.reason().contains("`destination`"), "got: {err:?}");

    let err = tools::execute(
        "app_launch",
        &serde_json::json!({"app": "maps", "destination": "San Francisco", "shell": "id"}),
        &ctx(&root),
    )
    .expect_err("invented arguments must be refused");
    assert!(err.reason().contains("unexpected argument `shell`"), "got: {err:?}");
}

#[test]
fn the_geo_uri_is_built_like_the_phone_proved() {
    assert_eq!(launch::geo_uri("San Francisco").unwrap(), "geo:0,0?q=San%20Francisco");
    let launch = launch::build_launch("maps", "San Francisco").unwrap();
    assert_eq!(launch.program, "gio");
    assert_eq!(launch.args, vec!["open".to_string(), "geo:0,0?q=San%20Francisco".to_string()]);
}
