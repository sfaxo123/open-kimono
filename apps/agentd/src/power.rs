//! The power manager: what is on, who needs it, and what should be off.
//!
//! Three rules decide everything here:
//!
//! - **Observe, don't guess.** Every component state comes from a read-only
//!   probe (`/sys`, `/proc`). A file that is missing or unreadable is reported
//!   as `unknown` — never as `off`, because "we did not look" and "it is off"
//!   are different facts.
//! - **Only agentd's own children may be switched.** The plan allows
//!   starting/stopping agentd's own child processes and nothing else. Radios,
//!   the screen and Bluetooth are reported and *proposed*: the plan carries the
//!   exact command an operator would run, marked `dry_run: true`, and nothing
//!   here executes it.
//! - **Every decision is an event.** A component going from needed to not-needed
//!   is journaled as `op6.PowerChange` with a reason that names the idle time
//!   and who has to act, so the kimono view can show it and a reader can tell
//!   a proposal from an action.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Child;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use trace_events::{AgUiEvent, Op6Event, Payload, PowerState};

use crate::journal::Journal;
use crate::phone::ProbeRoot;
use crate::services::ServiceHub;

/// What a probe actually saw. `Unknown` is its own state on purpose: a missing
/// `/sys` entry is not evidence that a component is off.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Observed {
    On,
    Off,
    Unknown,
}

impl Observed {
    pub fn as_str(self) -> &'static str {
        match self {
            Observed::On => "on",
            Observed::Off => "off",
            Observed::Unknown => "unknown",
        }
    }
}

/// One tracked component.
#[derive(Clone, Debug)]
pub struct Component {
    pub name: &'static str,
    pub state: Observed,
    /// How this was observed — a probe path, or the reason it is unknown.
    pub source: String,
    /// Whether agentd is allowed to switch this component itself.
    pub agentd_can_switch: bool,
    /// What needs it right now ("a request is in flight", "service svc.x"), or
    /// `None` when nothing does.
    pub needed_by: Option<String>,
    /// The command an operator would run to turn it off, if agentd may not.
    pub operator_command: Option<&'static str>,
    /// Whether power may suggest switching this component off at all. Services
    /// are excluded: stopping one is the user's call through `service_stop`.
    pub operator_may_switch: bool,
}

/// The manager's view of time. Everything idle-related is measured through
/// this, so the boundary of the idle timeout can be tested at exactly the
/// millisecond instead of by sleeping and hoping.
pub trait Clock: Send + Sync {
    /// Milliseconds since the manager was created.
    fn now_ms(&self) -> u64;
}

/// The real clock: monotonic elapsed time, never wall-of-day.
pub struct WallClock {
    start: Instant,
}

impl Clock for WallClock {
    fn now_ms(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }
}

/// A clock a test drives by hand.
#[derive(Clone, Default)]
pub struct FakeClock {
    ms: std::sync::Arc<Mutex<u64>>,
}

impl FakeClock {
    pub fn advance(&self, delta_ms: u64) {
        let mut now = self.ms.lock().expect("fake clock poisoned");
        *now += delta_ms;
    }
}

impl Clock for FakeClock {
    fn now_ms(&self) -> u64 {
        *self.ms.lock().expect("fake clock poisoned")
    }
}


/// The backlight file, wherever this device's driver put it. The OnePlus 6 on
/// pmOS exposes `sys/class/backlight/ae94000.dsi.0/brightness` — not the
/// `lcd/brightness` that other pmOS devices use — so the directory is listed
/// instead of guessed at.
fn backlight(root: &ProbeRoot) -> Option<(String, u32)> {
    let mut names: Vec<String> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(root.path().join("sys/class/backlight")) {
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                names.push(name.to_string());
            }
        }
    }
    names.sort();
    for name in names {
        for file in ["brightness", "actual_brightness"] {
            let rel = format!("sys/class/backlight/{name}/{file}");
            if let Some(raw) = probe_first(root, &[rel.as_str()]) {
                if let Ok(value) = raw.trim().parse::<u32>() {
                    return Some((rel, value));
                }
            }
        }
    }
    // Some kernels expose the connector instead: `enabled` / `disabled`.
    if let Some(raw) = probe_first(root, &["sys/class/drm/card0-DSI-1/enabled"]) {
        let on = raw.trim() == "enabled";
        return Some(("sys/class/drm/card0-DSI-1/enabled".to_string(), if on { 1 } else { 0 }));
    }
    None
}

/// Bluetooth, including the case the phone actually shows: the controller is
/// registered but rfkill-blocked, so `hci0/state` does not exist at all. That is
/// not "unknown" — rfkill says it is off.
fn bluetooth(root: &ProbeRoot) -> (Observed, String) {
    if let Some(raw) = probe_first(root, &["sys/class/bluetooth/hci0/state"]) {
        let text = raw.trim().to_string();
        let state = if text == "down" { Observed::Off } else { Observed::On };
        return (state, "sys/class/bluetooth/hci0/state".to_string());
    }
    let mut names: Vec<String> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(root.path().join("sys/class/rfkill")) {
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                names.push(name.to_string());
            }
        }
    }
    names.sort();
    for name in names {
        let kind = probe_first(root, &[format!("sys/class/rfkill/{name}/type").as_str()]).unwrap_or_default();
        if kind.trim() == "bluetooth" {
            if let Some(state) = probe_first(root, &[format!("sys/class/rfkill/{name}/state").as_str()]) {
                let on = state.trim() != "0";
                return (
                    if on { Observed::On } else { Observed::Off },
                    format!("sys/class/rfkill/{name}/state — the controller is rfkill-blocked, so hci0/state does not exist"),
                );
            }
        }
    }
    (Observed::Unknown, "not readable: sys/class/bluetooth/hci0/state, sys/class/rfkill/*/type".to_string())
}

pub struct PowerManager {
    journal: Journal,
    root: ProbeRoot,
    idle_timeout: Duration,
    /// The local brain's port, from `local_url` — how "local model is up" is
    /// decided without connecting to it.
    local_model_port: Option<u16>,
    clock: std::sync::Arc<dyn Clock>,
    /// What the clock read when the last request started or finished.
    last_request_ms: Mutex<u64>,
    /// The last state powerd announced per component, so a transition is
    /// journaled once and not on every tick.
    announced: Mutex<BTreeMap<String, Observed>>,
    /// Children agentd started. These are the only processes it may stop.
    children: Mutex<Vec<Child>>,
    in_flight: std::sync::atomic::AtomicBool,
    /// Measured drain + pre-approvals behind every recommendation (A4).
    policy: Mutex<crate::power_policy::PowerPolicy>,
    /// The previous battery reading: (percent, clock ms, charging).
    last_battery: Mutex<Option<(u64, u64, bool)>>,
    /// Drain samples already covered by a journaled recommendation.
    advised_samples: Mutex<usize>,
}

impl PowerManager {
    pub fn new(journal: Journal, root: ProbeRoot, idle_timeout: Duration, local_model_port: Option<u16>) -> PowerManager {
        PowerManager::with_clock(journal, root, idle_timeout, local_model_port, std::sync::Arc::new(WallClock { start: Instant::now() }))
    }

    /// The same manager on a clock the caller controls. Tests use this to put
    /// the idle window exactly on both sides of the timeout.
    pub fn with_clock(
        journal: Journal,
        root: ProbeRoot,
        idle_timeout: Duration,
        local_model_port: Option<u16>,
        clock: std::sync::Arc<dyn Clock>,
    ) -> PowerManager {
        PowerManager {
            journal,
            root,
            idle_timeout,
            local_model_port,
            clock,
            last_request_ms: Mutex::new(0),
            announced: Mutex::new(BTreeMap::new()),
            children: Mutex::new(Vec::new()),
            in_flight: std::sync::atomic::AtomicBool::new(false),
            policy: Mutex::new(crate::power_policy::PowerPolicy::new()),
            last_battery: Mutex::new(None),
            advised_samples: Mutex::new(0),
        }
    }

    /// The user's pre-approval list (from `agentd.toml`): the only
    /// components a recommendation may mark for auto-switch.
    pub fn set_preapproved(&self, items: impl IntoIterator<Item = String>) {
        let mut policy = self.policy.lock().expect("power policy poisoned");
        for item in items {
            policy.preapprove(&item);
        }
    }

    /// The current pre-approval list, sorted for display and tests.
    pub fn preapproved(&self) -> Vec<String> {
        let policy = self.policy.lock().expect("power policy poisoned");
        let mut items: Vec<String> = policy.preapproved.iter().cloned().collect();
        items.sort();
        items
    }

    /// A request started: the idle clock restarts and nothing may be proposed
    /// off while it runs.
    pub fn request_started(&self) {
        *self.last_request_ms.lock().expect("power idle clock poisoned") = self.clock.now_ms();
        self.in_flight.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// A request finished.
    pub fn request_finished(&self) {
        *self.last_request_ms.lock().expect("power idle clock poisoned") = self.clock.now_ms();
        self.in_flight.store(false, std::sync::atomic::Ordering::SeqCst);
    }

    /// Register a child agentd started. It is the only kind of process the
    /// manager is allowed to stop when it goes idle.
    pub fn note_child(&self, label: &'static str, child: Child) {
        let mut children = self.children.lock().expect("power children poisoned");
        // Drop children that already exited, so the count is honest.
        let mut alive = Vec::new();
        for mut existing in children.drain(..) {
            if existing.try_wait().ok().flatten().is_none() {
                alive.push(existing);
            }
        }
        *children = alive;
        children.push(child);
        let _ = label;
        self.journal.push(Payload::Op6(Op6Event::PowerChange {
            component: "local_model".to_string(),
            state: PowerState::On,
            reason: format!("agentd started its own child `{label}`"),
        }));
    }

    fn idle_for(&self) -> Duration {
        let last = *self.last_request_ms.lock().expect("power idle clock poisoned");
        Duration::from_millis(self.clock.now_ms().saturating_sub(last))
    }

    fn in_flight(&self) -> bool {
        self.in_flight.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Read every component from the read-only probes.
    pub fn components(&self, services: Option<&ServiceHub>) -> Vec<Component> {
        let running = self.running_services(services);
        let flight = self.in_flight();
        let mut out = Vec::new();

        // — screen: pmOS exposes backlight power under /sys/class; absent here
        //   means we do not know, and agentd may not touch it either way.
        out.push(Component {
            name: "screen",
            state: match backlight(&self.root) {
                Some((_, value)) => if value > 0 { Observed::On } else { Observed::Off },
                None => Observed::Unknown,
            },
            source: match backlight(&self.root) {
                Some((path, _)) => path,
                None => "not readable: no brightness file under sys/class/backlight".to_string(),
            },
            agentd_can_switch: false,
            needed_by: flight.then(|| "a request is in flight".to_string()),
            operator_command: Some("busctl --user call postmarketOS.Monitor / postmarketOS.Monitor set s b sleep 1"),
            operator_may_switch: true,
        });

        // — Bluetooth: /sys/class/bluetooth/hci0/state says up/down/…
        out.push(Component {
            name: "bluetooth",
            state: bluetooth(&self.root).0,
            source: bluetooth(&self.root).1,
            agentd_can_switch: false,
            needed_by: None,
            operator_command: Some("rfkill block bluetooth"),
            operator_may_switch: true,
        });

        // — Wi-Fi: operstate is read-only and honest.
        out.push(Component {
            name: "wifi",
            state: probe_first(&self.root, &["sys/class/net/wlan0/operstate"])
                .map(|raw| if raw.trim() == "up" { Observed::On } else { Observed::Off })
                .unwrap_or(Observed::Unknown),
            source: probe_note(&self.root, &["sys/class/net/wlan0/operstate"]),
            agentd_can_switch: false,
            needed_by: needed_wifi(&running, flight),
            operator_command: Some("nmcli radio wifi off"),
            operator_may_switch: true,
        });

        // — Local model: is the configured brain port LISTENing? Read from
        //   /proc/net/tcp, which never touches the socket.
        out.push(Component {
            name: "local_model",
            state: self.local_model_state(),
            source: match self.local_model_port {
                Some(port) => format!("/proc/net/tcp, port {port} in LISTEN state"),
                None => "no local_url configured, so there is no local model to watch".to_string(),
            },
            agentd_can_switch: true,
            needed_by: needed_local_model(&running, flight),
            operator_command: None,
            operator_may_switch: true,
        });

        // — Services: agentd never stops a service for power reasons; that is
        //   the user's call through `service_stop`.
        out.push(Component {
            name: "services",
            state: if running.is_empty() { Observed::Off } else { Observed::On },
            source: format!("{} running service(s) in the svcd executor", running.len()),
            agentd_can_switch: false,
            needed_by: if running.is_empty() { None } else { Some(format!("services: {}", running.join(", "))) },
            operator_command: None,
            operator_may_switch: false,
        });

        out
    }

    fn running_services(&self, services: Option<&ServiceHub>) -> Vec<String> {
        let Some(hub) = services else { return Vec::new() };
        let listed = hub.list();
        listed["services"]
            .as_array()
            .map(|rows| {
                rows.iter()
                    .filter(|row| row["status"]["status"].as_str() == Some("running"))
                    .filter_map(|row| row["id"].as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn local_model_state(&self) -> Observed {
        let Some(port) = self.local_model_port else { return Observed::Unknown };
        match std::fs::read_to_string(self.root.path().join("proc/net/tcp")) {
            Ok(text) => {
                if listen_ports(&text).contains(&port) {
                    Observed::On
                } else {
                    Observed::Off
                }
            }
            Err(_) => Observed::Unknown,
        }
    }

    /// The dry-run plan: what should be off, what agentd would do about it, and
    /// the exact command if only an operator may do it.
    pub fn plan(&self, services: Option<&ServiceHub>) -> Value {
        let idle = self.idle_for();
        let idle_secs = idle.as_secs();
        let past_idle = idle >= self.idle_timeout;
        let mut entries = Vec::new();

        for c in self.components(services) {
            let entry = if c.needed_by.is_some() {
                json!({
                    "component": c.name,
                    "state": c.state.as_str(),
                    "action": "keep",
                    "because": c.needed_by,
                    "dry_run": true,
                })
            } else if !past_idle {
                json!({
                    "component": c.name,
                    "state": c.state.as_str(),
                    "action": "wait",
                    "because": format!("idle {idle_secs}s of {}s", self.idle_timeout.as_secs()),
                    "dry_run": true,
                })
            } else if c.state == Observed::Unknown {
                json!({
                    "component": c.name,
                    "state": c.state.as_str(),
                    "action": "cannot_tell",
                    "because": format!("{} — power cannot say it is on, so it cannot propose turning it off", c.source),
                    "dry_run": true,
                })
            } else if !c.operator_may_switch {
                json!({
                    "component": c.name,
                    "state": c.state.as_str(),
                    "action": "leave_alone",
                    "because": "power never stops a service — that is the user's call through `service_stop`",
                    "dry_run": true,
                })
            } else if c.agentd_can_switch {
                json!({
                    "component": c.name,
                    "state": c.state.as_str(),
                    "action": "off_by_agentd",
                    "because": format!("idle {idle_secs}s and no request or service needs it"),
                    "how": "stop the child agentd started for it",
                    "dry_run": false,
                })
            } else {
                json!({
                    "component": c.name,
                    "state": c.state.as_str(),
                    "action": "propose_off",
                    "because": format!("idle {idle_secs}s and no request or service needs it"),
                    "command": c.operator_command,
                    "dry_run": true,
                    "note": "agentd does not touch radios, the screen or Bluetooth — the operator runs this",
                })
            };
            entries.push(entry);
        }

        json!({
            "idle_seconds": idle_secs,
            "idle_timeout_seconds": self.idle_timeout.as_secs(),
            "request_in_flight": self.in_flight(),
            "children_started_by_agentd": self.children.lock().expect("power children poisoned").len(),
            "drain": self.drain_json(),
            "plan": entries,
        })
    }

    /// Advance the manager: stop what agentd may stop once it is idle, and
    /// journal every component transition with its reason.
    pub fn tick(&self, services: Option<&ServiceHub>) {
        let idle = self.idle_for();
        let past_idle = idle >= self.idle_timeout;

        // The only real switch: agentd's own children.
        if past_idle && !self.in_flight() {
            let mut children = self.children.lock().expect("power children poisoned");
            let mut stopped = 0usize;
            for child in children.iter_mut() {
                if child.try_wait().ok().flatten().is_some() {
                    continue;
                }
                let _ = child.kill();
                let _ = child.wait();
                stopped += 1;
            }
            children.clear();
            if stopped > 0 {
                self.journal.push(Payload::Op6(Op6Event::PowerChange {
                    component: "local_model".to_string(),
                    state: PowerState::Off,
                    reason: format!("idle {}s: agentd stopped {} of its own child process(es)", idle.as_secs(), stopped),
                }));
            }
        }

        // Announce transitions of the observed state, once each.
        for c in self.components(services) {
            let proposed = if c.operator_may_switch && !c.agentd_can_switch && c.needed_by.is_none() && past_idle && c.state == Observed::On {
                // Nothing needs it and the idle window passed: powerd's view is
                // "should be off", even though only the operator can make it so.
                Observed::Off
            } else {
                c.state
            };
            let previous = {
                let mut announced = self.announced.lock().expect("power announcements poisoned");
                let previous = announced.get(c.name).copied();
                announced.insert(c.name.to_string(), proposed);
                previous
            };
            if previous == Some(proposed) {
                continue;
            }
            let reason = match (&c.needed_by, past_idle) {
                (Some(why), _) => format!("needed: {why}"),
                (None, true) if !c.operator_may_switch => format!(
                    "idle {}s — services are the user's to stop through `service_stop`, power only reports them",
                    idle.as_secs()
                ),
                (None, true) => format!(
                    "idle {}s and nothing needs it — {}",
                    idle.as_secs(),
                    if c.agentd_can_switch { "agentd switched it" } else { "proposed for the operator" }
                ),
                (None, false) => format!("idle {}s of {}s, still held", idle.as_secs(), self.idle_timeout.as_secs()),
            };
            self.journal.push(Payload::Op6(match proposed {
                Observed::Off => Op6Event::PowerChange { component: c.name.to_string(), state: PowerState::Off, reason },
                Observed::On => Op6Event::PowerChange { component: c.name.to_string(), state: PowerState::On, reason },
                Observed::Unknown => continue,
            }));
        }

        self.sample_battery(services);
    }

    /// Take one battery reading and, when it completes a measured drop,
    /// record the drain rate. A new sample journals exactly one
    /// `power_recommendation`; with no baseline nothing is journaled and
    /// the plan simply carries the no-baseline line. Only the overall
    /// battery rate is measured — sysfs cannot attribute drain to one
    /// component, so none is invented.
    fn sample_battery(&self, services: Option<&ServiceHub>) {
        let Some(sample) = crate::phone::battery_sample(&self.root) else { return };
        let now = self.clock.now_ms();
        {
            let mut last = self.last_battery.lock().expect("battery sample poisoned");
            if let Some((prev_pct, prev_ms, prev_charging)) = *last {
                let elapsed = now.saturating_sub(prev_ms);
                if elapsed > 0 && !sample.charging && !prev_charging && sample.percent < prev_pct {
                    let rate = (prev_pct - sample.percent) as f64 / (elapsed as f64 / 3_600_000.0);
                    if rate > 0.0 {
                        self.policy
                            .lock()
                            .expect("power policy poisoned")
                            .drain
                            .record_at("battery", rate, now);
                    }
                }
            }
            *last = Some((sample.percent, now, sample.charging));
        }

        let (samples, window_ms, pct_per_hour) = {
            let policy = self.policy.lock().expect("power policy poisoned");
            match policy.drain.estimate("battery") {
                Some(e) => (e.samples, e.window_ms, Some(e.pct_per_hour)),
                None => (0, 0, None),
            }
        };
        if samples == 0 {
            return;
        }
        {
            let mut advised = self.advised_samples.lock().expect("advised count poisoned");
            if samples <= *advised {
                return;
            }
            *advised = samples;
        }
        let candidate = self
            .components(services)
            .into_iter()
            .find(|c| c.operator_may_switch && c.needed_by.is_none() && c.state == Observed::On)
            .map(|c| c.name);
        let auto_switch = candidate
            .map(|c| self.preapproved().iter().any(|p| p == c))
            .unwrap_or(false);
        let text = self
            .policy
            .lock()
            .expect("power policy poisoned")
            .battery_advice(candidate);
        self.journal.push(Payload::AgUi(AgUiEvent::Custom {
            name: "power_recommendation".to_string(),
            value: json!({
                "component": "battery",
                "candidate": candidate,
                "samples": samples,
                "window_ms": window_ms,
                "pct_per_hour": pct_per_hour,
                "auto_switch": auto_switch,
                "dry_run": true,
                "text": text,
            }),
        }));
    }

    /// The measured-drain summary both read-only views carry.
    fn drain_json(&self) -> Value {
        let policy = self.policy.lock().expect("power policy poisoned");
        match policy.drain.estimate("battery") {
            Some(e) => json!({
                "component": "battery",
                "samples": e.samples,
                "window_ms": e.window_ms,
                "pct_per_hour": e.pct_per_hour,
                "note": format!("battery at ~{:.1}%/hr over {} sample(s)", e.pct_per_hour, e.samples),
            }),
            None => json!({
                "component": "battery",
                "samples": 0,
                "window_ms": 0,
                "pct_per_hour": null,
                "note": "battery: no measured baseline — no recommendation",
            }),
        }
    }

    /// The read-only snapshot the `power_status` tool and `GET /power` return.
    pub fn status(&self, services: Option<&ServiceHub>) -> Value {
        let components: Vec<Value> = self
            .components(services)
            .into_iter()
            .map(|c| {
                json!({
                    "component": c.name,
                    "state": c.state.as_str(),
                    "source": c.source,
                    "needed_by": c.needed_by,
                    "agentd_can_switch": c.agentd_can_switch,
                })
            })
            .collect();
        json!({
            "read_only": true,
            "idle_seconds": self.idle_for().as_secs(),
            "idle_timeout_seconds": self.idle_timeout.as_secs(),
            "request_in_flight": self.in_flight(),
            "children_started_by_agentd": self.children.lock().expect("power children poisoned").len(),
            "drain": self.drain_json(),
            "components": components,
        })
    }
}

fn needed_wifi(running: &[String], flight: bool) -> Option<String> {
    if flight {
        return Some("a request is in flight".to_string());
    }
    if !running.is_empty() {
        return Some(format!("{} running service(s) may need the network", running.len()));
    }
    None
}

fn needed_local_model(running: &[String], flight: bool) -> Option<String> {
    if flight {
        return Some("a request is in flight".to_string());
    }
    if !running.is_empty() {
        return Some(format!("{} running service(s) may use the local brain", running.len()));
    }
    None
}

/// Read the first probe path that exists.
fn probe_first(root: &ProbeRoot, paths: &[&str]) -> Option<String> {
    paths.iter().find_map(|p| std::fs::read_to_string(root.path().join(p)).ok())
}

fn probe_note(root: &ProbeRoot, paths: &[&str]) -> String {
    for p in paths {
        let full = root.path().join(p);
        if read_ok(&full) {
            return p.to_string();
        }
    }
    format!("not readable: {}", paths.join(", "))
}

fn read_ok(path: &Path) -> bool {
    std::fs::read_to_string(path).is_ok()
}

/// Ports in LISTEN state (`st == 0A`) from a `/proc/net/tcp` table.
fn listen_ports(table: &str) -> Vec<u16> {
    let mut ports = Vec::new();
    for line in table.lines().skip(1) {
        let mut fields = line.split_whitespace();
        let (Some(_), Some(local), Some(_), Some(state)) = (fields.next(), fields.next(), fields.next(), fields.next()) else {
            continue;
        };
        if state != "0A" {
            continue;
        }
        if let Some(hex) = local.split(':').nth(1) {
            if let Ok(port) = u16::from_str_radix(hex, 16) {
                ports.push(port);
            }
        }
    }
    ports
}
