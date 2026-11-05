//! A0 — the cross-build smoke path: build one valid journal line, anywhere.
//!
//! The point of this module is provable portability: it depends on nothing
//! but `trace-events` (which is `no_std` + `alloc`) and `serde_json`, so if
//! the aarch64-musl build of this crate links and this line parses back into
//! an [`Event`], the toolchain recipe in `docs/track-a/BUILD.md` is real.
//!
//! The event itself is an `ag_ui` `CUSTOM` record named `track_a.smoke` —
//! the journal has no "hello" event and inventing one would break the rule
//! that every payload comes from a copied vocabulary.

use std::fs;

use serde_json::json;
use trace_events::{AgUiEvent, Event, Payload};

/// Producer identity carried in the envelope, so the kimono view can filter.
pub const SERVICE: &str = "agentd-smoke";
/// Name of the custom event this path emits.
pub const EVENT_NAME: &str = "track_a.smoke";

/// Where the smoke line was produced. Read from the host, never assumed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostInfo {
    pub hostname: String,
    pub os_release: String,
    pub arch: String,
}

impl HostInfo {
    /// Read the identity of the machine we are running on. Each probe has a
    /// fallback so a minimal rootfs (musl, no `uname` wrapper) still works.
    pub fn probe() -> HostInfo {
        HostInfo {
            hostname: read_first_line("/etc/hostname")
                .or_else(|| std::env::var("HOSTNAME").ok())
                .unwrap_or_else(|| "unknown-host".to_string()),
            os_release: read_first_line("/proc/sys/kernel/osrelease")
                .unwrap_or_else(|| "unknown-os".to_string()),
            arch: read_first_line("/proc/device-tree/cpus/compatible")
                .map(|s| s.trim_matches('\0').to_string())
                .unwrap_or_else(|| std::env::consts::ARCH.to_string()),
        }
    }
}

fn read_first_line(path: &str) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|s| s.lines().next().unwrap_or("").to_string())
        .filter(|s| !s.is_empty())
}

/// Wall clock in microseconds since the Unix epoch (the envelope's `ts_us`).
pub fn now_us() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_micros() as u64)
        .unwrap_or(0)
}

/// Build the smoke event. `seq` is a placeholder: a real journal reassigns
/// it, and the standalone binary has no journal, so it stays 0.
pub fn smoke_event(ts_us: u64, host: &HostInfo) -> Event {
    Event {
        seq: 0,
        ts_us,
        service: Some(SERVICE.to_string()),
        trace_id: None,
        span_id: None,
        parent_span_id: None,
        payload: Payload::AgUi(AgUiEvent::Custom {
            name: EVENT_NAME.to_string(),
            value: json!({
                "hostname": host.hostname,
                "os_release": host.os_release,
                "arch": host.arch,
                "target": target_triple(),
                "libc": if target_is_musl() { "musl" } else { "native" },
            }),
        }),
    }
}

/// The target this binary was built for, baked in at compile time.
pub fn target_triple() -> &'static str {
    match (std::env::consts::ARCH, std::env::consts::OS, cfg!(target_env = "musl")) {
        ("aarch64", "linux", true) => "aarch64-unknown-linux-musl",
        ("aarch64", "linux", false) => "aarch64-unknown-linux-gnu",
        ("x86_64", "linux", true) => "x86_64-unknown-linux-musl",
        ("x86_64", "linux", false) => "x86_64-unknown-linux-gnu",
        _ => "unknown-target",
    }
}

/// True when this binary statically links musl (the phone's libc).
pub fn target_is_musl() -> bool {
    cfg!(target_env = "musl")
}

/// Serialize the smoke event to exactly one JSONL line (no trailing newline).
pub fn smoke_line(ts_us: u64, host: &HostInfo) -> Result<String, serde_json::Error> {
    trace_events::to_json_line(&smoke_event(ts_us, host))
}
