//! The tools the model may call, and the one place that decides what a tool
//! result means for the request loop.
//!
//! Two rules keep this honest: an argument blob is only ever used after it
//! parses (a malformed tool call is an error the model sees, never a crash),
//! and every tool result is small enough to journal in full — the kimono view
//! shows the same bytes the model saw.

use serde_json::{json, Value};

use crate::phone::{self, ProbeRoot};

/// What a tool call produced, and whether it ends the request.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolOutcome {
    /// The result handed back to the model (and journaled).
    pub content: Value,
    /// `answer` ends the loop; everything else feeds the model and continues.
    pub terminal: bool,
    pub answer_text: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToolError {
    /// The model asked for a tool that does not exist.
    Unknown(String),
    /// The call parsed as JSON but the arguments are wrong.
    BadArguments(String),
    /// The call was not valid JSON at all.
    MalformedArguments(String),
}

impl ToolError {
    pub fn reason(&self) -> String {
        match self {
            ToolError::Unknown(name) => format!("unknown tool `{name}`"),
            ToolError::BadArguments(m) => m.clone(),
            ToolError::MalformedArguments(m) => format!("tool arguments are not valid JSON: {m}"),
        }
    }
}

/// What a tool is allowed to touch. `services` is absent when the Lego layer
/// is not running, and the service tools then say so instead of pretending.
pub struct ToolCtx<'a> {
    pub root: &'a ProbeRoot,
    pub services: Option<&'a crate::services::ServiceHub>,
    pub power: Option<&'a crate::power::PowerManager>,
}

impl<'a> ToolCtx<'a> {
    pub fn new(root: &'a ProbeRoot, services: Option<&'a crate::services::ServiceHub>, power: Option<&'a crate::power::PowerManager>) -> Self {
        Self { root, services, power }
    }
}

/// Tool definitions in the shape the chat API wants.
pub fn definitions() -> Vec<Value> {
    vec![
        crate::model::tool_definition(
            "answer",
            "Give the user the final spoken/visible answer and finish the request. Call this exactly once, at the end.",
            json!({"type": "object", "properties": {"text": {"type": "string", "description": "The answer, plain text, short."}}, "required": ["text"], "additionalProperties": false}),
        ),
        crate::model::tool_definition(
            "get_time",
            "Read the phone's current date and time (UTC and epoch microseconds).",
            json!({"type": "object", "properties": {}, "additionalProperties": false}),
        ),
        crate::model::tool_definition(
            "phone_status",
            "Read the phone's battery, uptime, memory, load and network interfaces. Read-only.",
            json!({"type": "object", "properties": {}, "additionalProperties": false}),
        ),
        crate::model::tool_definition(
            "service_create",
            "Install a new ServiceIR on the phone. The IR is validated, then run through svcd's simulator against its OWN `tests`, and only installed if every test passes — a service whose tests fail is refused and the failure is reported back to you so you can fix it. The IR must be complete: ir_version, id, version, generation, title, description, author, execution_mode, priority_class, triggers, workflow, actions, deps, capabilities, state_schema, power_budget and at least one test. Effects (speech, UI, logic) are recorded as journal records, not performed.",
            json!({"type": "object", "properties": {"ir": {"type": "object", "description": "The complete ServiceIR document."}}, "required": ["ir"], "additionalProperties": false}),
        ),
        crate::model::tool_definition(
            "service_list",
            "List the services installed on the phone with their status, state and power announcement.",
            json!({"type": "object", "properties": {}, "additionalProperties": false}),
        ),
        crate::model::tool_definition(
            "service_describe",
            "Show one installed service in full: its IR, live state, status, grants and the effects it has asked for.",
            json!({"type": "object", "properties": {"id": {"type": "string", "description": "The service id, e.g. svc.demo.water_reminder."}}, "required": ["id"], "additionalProperties": false}),
        ),
        crate::model::tool_definition(
            "service_stop",
            "Stop one installed service (its lifecycle_stop triggers run, then it stops). This only affects services installed through agentd.",
            json!({"type": "object", "properties": {"id": {"type": "string"}, "reason": {"type": "string", "description": "Why it is being stopped, for the journal."}}, "required": ["id"], "additionalProperties": false}),
        ),
        crate::model::tool_definition(
            "power_status",
            "Read which components are on (screen, bluetooth, wifi, local_model, services), how that was observed, and what needs each one. Read-only: it changes nothing.",
            json!({"type": "object", "properties": {}, "additionalProperties": false}),
        ),
        crate::model::tool_definition(
            "power_plan",
            "Ask what should be switched off after the idle timeout and why. The plan is a DRY RUN: for radios, the screen and Bluetooth it only names the command an operator would run, and agentd never executes it. Read-only.",
            json!({"type": "object", "properties": {}, "additionalProperties": false}),
        ),
        crate::model::tool_definition(
            "app_launch",
            "Open an app on the phone at a destination. Only `maps` is allowed: it opens GNOME Maps showing the destination — a place name (e.g. \"San Francisco\") or coordinates as \"lat,lon\". This is navigate-class: it displays a map on the phone itself; it does not send, spend, call or share anything. The destination is data only: it becomes a geo: search, never a command.",
            json!({"type": "object", "properties": {"app": {"type": "string", "description": "The app to open. Only \"maps\" is on the allowlist."}, "destination": {"type": "string", "description": "A place name, or coordinates as \"lat,lon\"."}}, "required": ["app", "destination"], "additionalProperties": false}),
        ),
    ]
}

/// Parse a tool call's raw argument string. Empty means "no arguments".
pub fn parse_arguments(raw: &str) -> Result<Value, ToolError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_str(trimmed).map_err(|e| ToolError::MalformedArguments(e.to_string()))
}

/// Execute one tool call. `args` must already be parsed JSON.
pub fn execute(name: &str, args: &Value, ctx: &ToolCtx) -> Result<ToolOutcome, ToolError> {
    match name {
        "answer" => {
            let text = args
                .get("text")
                .and_then(Value::as_str)
                .filter(|t| !t.trim().is_empty())
                .ok_or_else(|| ToolError::BadArguments("answer requires a non-empty string field `text`".to_string()))?;
            Ok(ToolOutcome { content: json!({"answered": true}), terminal: true, answer_text: Some(text.to_string()) })
        }
        "get_time" => {
            reject_stray_fields(args, &[])?;
            let us = crate::smoke::now_us();
            Ok(ToolOutcome {
                content: json!({"epoch_us": us, "utc": iso8601_utc(us / 1_000_000), "source": "phone_clock"}),
                terminal: false,
                answer_text: None,
            })
        }
        "phone_status" => {
            reject_stray_fields(args, &[])?;
            Ok(ToolOutcome { content: phone::probe_json(ctx.root), terminal: false, answer_text: None })
        }
        "service_create" => {
            reject_stray_fields(args, &["ir"])?;
            let ir = args
                .get("ir")
                .filter(|v| v.is_object())
                .ok_or_else(|| ToolError::BadArguments("service_create requires the full ServiceIR as an object in field `ir`".to_string()))?;
            let hub = hub(ctx)?;
            // A refusal is information, not a crash: the model sees exactly
            // which field or test failed and may fix the IR and try again.
            let reply = unwrap_reply(hub.create(ir))?;
            Ok(ToolOutcome { content: reply, terminal: false, answer_text: None })
        }
        "service_list" => {
            reject_stray_fields(args, &[])?;
            let hub = hub(ctx)?;
            Ok(ToolOutcome { content: unwrap_reply(hub.list())?, terminal: false, answer_text: None })
        }
        "service_describe" => {
            reject_stray_fields(args, &["id"])?;
            let id = required_id(args)?;
            let hub = hub(ctx)?;
            Ok(ToolOutcome { content: unwrap_reply(hub.describe(id))?, terminal: false, answer_text: None })
        }
        "service_stop" => {
            reject_stray_fields(args, &["id", "reason"])?;
            let id = required_id(args)?;
            let reason = args
                .get("reason")
                .and_then(Value::as_str)
                .filter(|r| !r.trim().is_empty())
                .unwrap_or("stopped by the agent at the user's request");
            let hub = hub(ctx)?;
            Ok(ToolOutcome { content: unwrap_reply(hub.stop(id, reason))?, terminal: false, answer_text: None })
        }
        "power_status" => {
            reject_stray_fields(args, &[])?;
            let pm = power(ctx)?;
            Ok(ToolOutcome { content: pm.status(ctx.services), terminal: false, answer_text: None })
        }
        "power_plan" => {
            reject_stray_fields(args, &[])?;
            let pm = power(ctx)?;
            Ok(ToolOutcome { content: pm.plan(ctx.services), terminal: false, answer_text: None })
        }
        "app_launch" => {
            reject_stray_fields(args, &["app", "destination"])?;
            let app = args
                .get("app")
                .and_then(Value::as_str)
                .filter(|a| !a.trim().is_empty())
                .ok_or_else(|| ToolError::BadArguments("app_launch requires a non-empty string field `app`".to_string()))?;
            let destination = args
                .get("destination")
                .and_then(Value::as_str)
                .filter(|d| !d.trim().is_empty())
                .ok_or_else(|| ToolError::BadArguments("app_launch requires a non-empty string field `destination`".to_string()))?;
            // A refusal is information, not a crash: an app that is not on
            // the allowlist, or a destination that cannot become a geo:
            // URI, is reported to the model exactly like a service refusal.
            let launch = crate::launch::build_launch(app, destination).map_err(ToolError::BadArguments)?;
            let output = crate::launch::run_launch(&launch).map_err(ToolError::BadArguments)?;
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let detail: String = stderr.trim().chars().take(300).collect();
                return Err(ToolError::BadArguments(format!("`gio open {}` failed with {}: {detail}", launch.uri, output.status)));
            }
            Ok(ToolOutcome {
                content: json!({"launched": true, "app": launch.app, "uri": launch.uri, "via": "gio open"}),
                terminal: false,
                answer_text: None,
            })
        }
        other => Err(ToolError::Unknown(other.to_string())),
    }
}

fn power<'a>(ctx: &'a ToolCtx<'a>) -> Result<&'a crate::power::PowerManager, ToolError> {
    ctx.power
        .ok_or_else(|| ToolError::BadArguments("the power manager is not running in this agentd".to_string()))
}

/// The executor answers with either a result or `{"error": "..."}`. Turn the
/// latter into a tool error so the model is told what went wrong.
fn unwrap_reply(reply: Value) -> Result<Value, ToolError> {
    if let Some(error) = reply.get("error").and_then(Value::as_str) {
        return Err(ToolError::BadArguments(error.to_string()));
    }
    Ok(reply)
}

fn hub<'a>(ctx: &'a ToolCtx<'a>) -> Result<&'a crate::services::ServiceHub, ToolError> {
    ctx.services
        .ok_or_else(|| ToolError::BadArguments("the service layer (svcd) is not running in this agentd".to_string()))
}

fn required_id(args: &Value) -> Result<&str, ToolError> {
    args.get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.trim().is_empty())
        .ok_or_else(|| ToolError::BadArguments("this service tool requires a non-empty string field `id`".to_string()))
}

/// A tool that takes no arguments must not be handed a bag of invented ones.
fn reject_stray_fields(args: &Value, allowed: &[&str]) -> Result<(), ToolError> {
    let Some(map) = args.as_object() else {
        return Err(ToolError::BadArguments(format!("arguments must be a JSON object, got {args}")));
    };
    if let Some(key) = map.keys().find(|k| !allowed.contains(&k.as_str())) {
        return Err(ToolError::BadArguments(format!("unexpected argument `{key}` for this tool")));
    }
    Ok(())
}

/// Seconds since the epoch → `YYYY-MM-DDTHH:MM:SSZ`. No date library: the
/// civil-from-days algorithm is 20 lines and testable.
pub fn iso8601_utc(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { y + 1 } else { y };
    format!("{year:04}-{month:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
}
