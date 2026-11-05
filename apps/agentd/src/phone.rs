//! Read-only phone status probes for the `phone_status` tool.
//!
//! Everything here is a file read under `/sys` or `/proc` — no commands, no
//! writes, nothing that can change the phone. The root is a parameter so the
//! host tests can point it at a fixture tree instead of the developer's own
//! machine, and a missing probe is reported as absent rather than guessed.

use std::path::PathBuf;

use serde_json::{json, Value};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PhoneStatus {
    pub battery_percent: Option<u64>,
    pub battery_status: Option<String>,
    pub uptime_sec: Option<u64>,
    pub mem_total_kb: Option<u64>,
    pub mem_available_kb: Option<u64>,
    pub load_1m: Option<String>,
    /// (interface, rx_bytes, tx_bytes) for every non-loopback interface.
    pub interfaces: Vec<(String, u64, u64)>,
    /// Power supplies the kernel exposes, by name.
    pub power_supplies: Vec<String>,
}

/// A filesystem root to probe. `/` on the phone, a fixture on the host.
#[derive(Clone, Debug)]
pub struct ProbeRoot(pub PathBuf);

impl ProbeRoot {
    /// The directory every probe is read from.
    pub fn path(&self) -> &std::path::Path {
        &self.0
    }

    pub fn system() -> ProbeRoot {
        ProbeRoot(PathBuf::from("/"))
    }

    fn read(&self, rel: &str) -> Option<String> {
        std::fs::read_to_string(self.0.join(rel)).ok()
    }
}

/// Probe the phone. Never fails: an unreadable file is simply an absent field,
/// and the journal shows exactly which fields were absent.
pub fn probe(root: &ProbeRoot) -> PhoneStatus {
    let mut status = PhoneStatus::default();

    if let Ok(supplies) = std::fs::read_dir(root.0.join("sys/class/power_supply")) {
        for entry in supplies.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            status.power_supplies.push(name.clone());
            let base = format!("sys/class/power_supply/{name}");
            if status.battery_percent.is_none() {
                if let Some(cap) = root.read(&format!("{base}/capacity")) {
                    if let Ok(v) = cap.trim().parse::<u64>() {
                        status.battery_percent = Some(v);
                        status.battery_status = root
                            .read(&format!("{base}/status"))
                            .map(|s| s.trim().to_string());
                    }
                }
            }
        }
        status.power_supplies.sort();
    }

    if let Some(uptime) = root.read("proc/uptime") {
        status.uptime_sec = uptime
            .split_whitespace()
            .next()
            .and_then(|s| s.parse::<f64>().ok())
            .map(|f| f as u64);
    }

    if let Some(meminfo) = root.read("proc/meminfo") {
        for line in meminfo.lines() {
            let mut fields = line.split_whitespace();
            let (Some(key), Some(value)) = (fields.next(), fields.next()) else { continue };
            if let Ok(kb) = value.parse::<u64>() {
                match key {
                    "MemTotal:" => status.mem_total_kb = Some(kb),
                    "MemAvailable:" => status.mem_available_kb = Some(kb),
                    _ => {}
                }
            }
        }
    }

    if let Some(loadavg) = root.read("proc/loadavg") {
        status.load_1m = loadavg.split_whitespace().next().map(str::to_string);
    }

    if let Some(netdev) = root.read("proc/net/dev") {
        for line in netdev.lines() {
            let Some((name, rest)) = line.split_once(':') else { continue };
            let name = name.trim();
            if name == "lo" {
                continue;
            }
            let nums: Vec<u64> = rest.split_whitespace().filter_map(|v| v.parse().ok()).collect();
            if nums.len() >= 9 {
                status.interfaces.push((name.to_string(), nums[1], nums[9]));
            }
        }
    }

    status
}

/// The JSON the `phone_status` tool returns to the model (and the journal).
pub fn to_json(status: &PhoneStatus) -> Value {
    json!({
        "battery_percent": status.battery_percent,
        "battery_status": status.battery_status,
        "uptime_sec": status.uptime_sec,
        "mem_total_kb": status.mem_total_kb,
        "mem_available_kb": status.mem_available_kb,
        "load_1m": status.load_1m,
        "interfaces": status
            .interfaces
            .iter()
            .map(|(name, rx, tx)| json!({"name": name, "rx_bytes": rx, "tx_bytes": tx}))
            .collect::<Vec<_>>(),
        "power_supplies": status.power_supplies,
        "read_only": true,
    })
}

/// Probe and render in one call, the way the tool uses it.
pub fn probe_json(root: &ProbeRoot) -> Value {
    to_json(&probe(root))
}

/// One battery reading for drain sampling: the charge level and whether
/// the phone is charging. `None` when no capacity probe exists — an
/// unreadable battery is no sample at all, never a guessed one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BatterySample {
    pub percent: u64,
    pub charging: bool,
}

pub fn battery_sample(root: &ProbeRoot) -> Option<BatterySample> {
    let status = probe(root);
    status.battery_percent.map(|percent| BatterySample {
        percent,
        charging: status
            .battery_status
            .as_deref()
            .map(|s| s.eq_ignore_ascii_case("charging"))
            .unwrap_or(false),
    })
}
