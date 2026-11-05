//! Schema-level checks serde cannot express: patterns, numeric ranges,
//! string lengths, array bounds, and the capability vocabulary. Each rule
//! cites the D1 schema keyword it mirrors.

use serde_json::Value;

use super::Report;
use crate::error::{ptr, Code};
use crate::ir::*;

fn ident_chars(s: &str, extra: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || extra.contains(c))
}

/// `^svc\.[a-z0-9_]+(\.[a-z0-9_]+)+$`
pub fn is_service_id(s: &str) -> bool {
    let Some(rest) = s.strip_prefix("svc.") else { return false };
    let segs: Vec<&str> = rest.split('.').collect();
    segs.len() >= 2 && segs.iter().all(|seg| ident_chars(seg, "_"))
}

/// `^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$`
pub fn is_semver(s: &str) -> bool {
    let parts: Vec<&str> = s.split('.').collect();
    parts.len() == 3
        && parts.iter().all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) && (*p == "0" || !p.starts_with('0')))
}

fn is_hex64(s: &str, prefix: &str) -> bool {
    s.strip_prefix(prefix).is_some_and(|h| h.len() == 64 && h.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
}

/// `^[a-z][a-z0-9_]*$`
fn is_param_key(s: &str) -> bool {
    s.starts_with(|c: char| c.is_ascii_lowercase()) && ident_chars(s, "_")
}

/// `^trig_[a-z0-9_]+$`
fn is_trigger_id(s: &str) -> bool {
    s.strip_prefix("trig_").is_some_and(|r| ident_chars(r, "_"))
}

/// `^(sys|sensor|power|audio|net):[a-z0-9_/.-]+$`
fn is_sensor_scheme(s: &str) -> bool {
    let Some((head, tail)) = s.split_once(':') else { return false };
    ["sys", "sensor", "power", "audio", "net"].contains(&head) && ident_chars(tail, "_/.-")
}

/// `^[a-z0-9_.-]+:([a-zA-Z0-9_./-]*)?$`
fn is_dep_scheme(s: &str) -> bool {
    let Some((head, tail)) = s.split_once(':') else { return false };
    ident_chars(head, "_.-") && tail.chars().all(|c| c.is_ascii_alphanumeric() || "_./-".contains(c))
}

/// `^[a-z0-9-]+:[a-z0-9-]+/[a-z0-9-]+(@[0-9.]+)?$`
fn is_wit_interface(s: &str) -> bool {
    let (body, ver) = match s.split_once('@') {
        Some((b, v)) => (b, Some(v)),
        None => (s, None),
    };
    let Some((ns, rest)) = body.split_once(':') else { return false };
    let Some((pkg, iface)) = rest.split_once('/') else { return false };
    [ns, pkg, iface].iter().all(|p| ident_chars(p, "-"))
        && ver.is_none_or(|v| !v.is_empty() && v.chars().all(|c| c.is_ascii_digit() || c == '.'))
}

/// `^/[a-zA-Z0-9_/-]*$` (`allow_root`) or `^/[a-zA-Z0-9_/-]+$`.
pub fn is_state_pointer(s: &str, allow_root: bool) -> bool {
    s.strip_prefix('/').is_some_and(|r| (allow_root || !r.is_empty()) && r.chars().all(|c| c.is_ascii_alphanumeric() || "_/-".contains(c)))
}

fn len_between(r: &mut Report, path: String, s: &str, min: usize, max: usize) {
    let n = s.chars().count();
    if n < min || n > max {
        r.err(path, Code::Length, format!("length {n} is outside {min}..={max}"));
    }
}

fn range(r: &mut Report, path: String, v: u64, min: u64, max: u64) {
    if v < min || v > max {
        r.err(path, Code::Range, format!("{v} is outside {min}..={max}"));
    }
}

fn pattern(r: &mut Report, path: String, ok: bool, value: &str, rule: &str) {
    if !ok {
        r.err(path, Code::Pattern, format!("`{value}` does not match {rule}"));
    }
}

pub fn check(ir: &ServiceIr, r: &mut Report) {
    if ir.ir_version != "1.0.0" {
        r.err("/ir_version", Code::Enum, format!("must be \"1.0.0\", got {:?}", ir.ir_version));
    }
    pattern(r, "/id".into(), is_service_id(&ir.id), &ir.id, "svc.<domain>.<name> (lowercase, digits, _)");
    pattern(r, "/version".into(), is_semver(&ir.version), &ir.version, "MAJOR.MINOR.PATCH");
    range(r, "/generation".into(), ir.generation, 1, u64::MAX);
    if let Some(h) = &ir.content_hash {
        pattern(r, "/content_hash".into(), is_hex64(h, "blake3:"), h, "blake3:<64 lowercase hex>");
    }
    len_between(r, "/title".into(), &ir.title, 3, 80);
    len_between(r, "/description".into(), &ir.description, 8, 512);

    if ir.parameters.len() > 32 {
        r.err("/parameters", Code::Length, format!("{} parameters, max 32", ir.parameters.len()));
    }
    for (i, p) in ir.parameters.iter().enumerate() {
        let base = ptr("/parameters", i);
        pattern(r, ptr(&base, "key"), is_param_key(&p.key), &p.key, "^[a-z][a-z0-9_]*$");
        if let Some(d) = &p.default {
            if !matches!(d, Value::String(_) | Value::Number(_) | Value::Bool(_) | Value::Null) {
                r.err(ptr(&base, "default"), Code::Type, "default must be a string, number, boolean or null");
            }
        }
        if p.options.as_ref().is_some_and(|o| o.len() > 200) {
            r.err(ptr(&base, "options"), Code::Length, "more than 200 options");
        }
    }

    if ir.triggers.is_empty() {
        r.err("/triggers", Code::Length, "at least one trigger is required");
    }
    for (i, t) in ir.triggers.iter().enumerate() {
        let base = ptr("/triggers", i);
        pattern(r, ptr(&base, "id"), is_trigger_id(&t.id), &t.id, "^trig_[a-z0-9_]+$");
        if let Some(v) = &t.voice_intent {
            if ["converse", "response", "stop", "fallback", "common_query"].contains(&v.intent_name.as_str()) {
                r.err(ptr(&ptr(&base, "voice_intent"), "intent_name"), Code::Enum, format!("`{}` is a reserved OVOS intent name", v.intent_name));
            }
            if v.sentences.is_empty() {
                r.err(ptr(&ptr(&base, "voice_intent"), "sentences"), Code::Length, "at least one sentence is required");
            }
        }
        if let Some(ts) = &t.timer_schedule {
            range(r, ptr(&ptr(&base, "timer_schedule"), "grace_window_sec"), ts.grace_window_sec, 0, 7200);
        }
        if let Some(s) = &t.sensor_threshold {
            let p = ptr(&ptr(&base, "sensor_threshold"), "scheme_path");
            pattern(r, p, is_sensor_scheme(&s.scheme_path), &s.scheme_path, "(sys|sensor|power|audio|net):<path>");
        }
    }
    if ir.workflow.as_ref().is_some_and(Vec::is_empty) {
        r.err("/workflow", Code::Length, "workflow, when present, needs at least one entry");
    }
    if ir.actions.is_empty() {
        r.err("/actions", Code::Length, "at least one action is required");
    }
    for (id, a) in &ir.actions {
        let base = ptr("/actions", id);
        len_between(r, ptr(&base, "description"), &a.description, 8, 256);
        range(r, ptr(&ptr(&base, "timeout"), "idle_ms"), a.timeout.idle_ms, 10, 300_000);
        range(r, ptr(&ptr(&base, "timeout"), "total_ms"), a.timeout.total_ms, 10, 300_000);
        if let Some(w) = &a.wasm_call {
            let wb = ptr(&base, "wasm_call");
            pattern(r, ptr(&wb, "wit_interface"), is_wit_interface(&w.wit_interface), &w.wit_interface, "ns:pkg/iface[@ver]");
            if let Some(o) = &w.output_state_pointer {
                pattern(r, ptr(&wb, "output_state_pointer"), is_state_pointer(o, true), o, "^/[a-zA-Z0-9_/-]*$");
            }
        }
        for (j, op) in a.state_patch.iter().flatten().enumerate() {
            let p = ptr(&ptr(&ptr(&base, "state_patch"), j), "path");
            pattern(r, p, is_state_pointer(&op.path, false), &op.path, "^/[a-zA-Z0-9_/-]+$");
        }
    }

    for (i, d) in ir.deps.schemes.iter().enumerate() {
        let p = ptr(&ptr("/deps/schemes", i), "scheme");
        pattern(r, p, is_dep_scheme(&d.scheme), &d.scheme, "<scheme>:[path]");
    }
    for (i, c) in ir.deps.wasm_components.iter().enumerate() {
        let base = ptr("/deps/wasm_components", i);
        pattern(r, ptr(&base, "digest"), is_hex64(&c.digest, "sha256:"), &c.digest, "sha256:<64 lowercase hex>");
        let pp = ptr(&base, "pool_policy");
        range(r, ptr(&pp, "pool_size"), c.pool_policy.pool_size, 0, 4);
        range(r, ptr(&pp, "max_invocations"), c.pool_policy.max_invocations, 1, 10_000);
        range(r, ptr(&pp, "max_concurrency"), c.pool_policy.max_concurrency, 1, 4);
        range(r, ptr(&pp, "reclaim_window_seconds"), c.pool_policy.reclaim_window_seconds, 0, 3600);
    }
    for (i, c) in ir.capabilities.iter().enumerate() {
        let base = ptr("/capabilities", i);
        if !CAPABILITIES.contains(&c.capability.as_str()) {
            r.err(ptr(&base, "capability"), Code::Enum, format!("unknown capability `{}`; allowed: {}", c.capability, CAPABILITIES.join(", ")));
        }
        range(r, ptr(&base, "max_duration_sec"), c.max_duration_sec, 1, 86_400);
    }

    let ss = &ir.state_schema;
    let ok = ss.namespace.strip_prefix("state:").is_some_and(|n| ident_chars(n, "_.-"));
    pattern(r, "/state_schema/namespace".into(), ok, &ss.namespace, "state:<name>");
    if let Some(w) = ss.wal_limit_bytes {
        range(r, "/state_schema/wal_limit_bytes".into(), w, 0, 67_108_864);
    }

    let pb = &ir.power_budget;
    range(r, "/power_budget/max_cpu_cores".into(), pb.max_cpu_cores, 1, 4);
    range(r, "/power_budget/cpu_quota_pct".into(), pb.cpu_quota_pct, 1, 100);
    range(r, "/power_budget/memory_limit_mb".into(), pb.memory_limit_mb, 1, 256);
    range(r, "/power_budget/wasm_fuel_per_invocation".into(), pb.wasm_fuel_per_invocation, 1000, 500_000_000);
    range(r, "/power_budget/max_wakelock_sec".into(), pb.max_wakelock_sec, 0, 3600);
    if !(0.0..=600.0).contains(&pb.max_wakeups_per_minute) {
        r.err("/power_budget/max_wakeups_per_minute", Code::Range, format!("{} is outside 0..=600", pb.max_wakeups_per_minute));
    }

    if let Some(rc) = &ir.retry {
        range(r, "/retry/max_retries".into(), rc.max_retries, 1, 5);
        range(r, "/retry/timeout_seconds".into(), rc.timeout_seconds, 1, 300);
        if let Some(t) = rc.on_failure_timeout_seconds {
            range(r, "/retry/on_failure_timeout_seconds".into(), t, 1, 600);
        }
    }

    if ir.tests.is_empty() {
        r.err("/tests", Code::Length, "at least one simulator test is required");
    }
    for (i, t) in ir.tests.iter().enumerate() {
        let base = ptr("/tests", i);
        if t.events.is_empty() {
            r.err(ptr(&base, "events"), Code::Length, "at least one event is required");
        }
        if !(0.0..=100.0).contains(&t.initial_env.battery_pct) {
            r.err(ptr(&ptr(&base, "initial_env"), "battery_pct"), Code::Range, "battery_pct must be within 0..=100");
        }
    }
}
