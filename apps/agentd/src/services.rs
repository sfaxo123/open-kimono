//! The Lego layer: the model composes services, `svcd` runs them.
//!
//! Four tools (`service_create` / `service_list` / `service_describe` /
//! `service_stop`) sit on top of the unmodified `crates/svcd` executor. The
//! gate is svcd's own: an IR is validated, then proven in the virtual-time
//! simulator, and only then admitted into the live executor — a service that
//! fails its own tests never runs on the phone.
//!
//! Two structural facts shape the design:
//!
//! - `Svcd` holds a `Box<dyn CapsBroker>`, which is not `Send`, so it cannot
//!   live behind a mutex that threads share. Instead the executor belongs to
//!   one thread and everyone else talks to it over a channel. That is also how
//!   svcd wants to be used: one deterministic executor, no locks in the middle.
//! - `ActionHost` is the seam where effects would touch the world (spoken
//!   dialog, Wasm logic, compensation). On this phone that seam is a recorder:
//!   every requested effect is journaled as an AG-UI `CUSTOM` record and
//!   answered with `dry_run: true`. Nothing is spoken, no surface is pushed to
//!   the system UI, no Wasm runs, and no radio, screen, DNS or container is
//!   touched. A reminder therefore *is* its journal record — which is exactly
//!   what the kimono view renders.

use std::collections::BTreeMap;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Map, Value};

use svcd::host::{ActionHost, ActionKind, ActionOutcome, ActionRequest, HostError};
use svcd::sim::AdmitError;
use svcd::{Config, InstallError, ServiceIr, ServiceStatus, Svcd};
use trace_events::{AgUiEvent, Payload};

use crate::journal::Journal;
use crate::phone::ProbeRoot;
use crate::smoke::now_us;

/// How many requested effects the host remembers (for `service_describe`).
const RECORDED_MAX: usize = 32;
/// Idle tick: how long the executor thread waits for a command before advancing
/// the service clock. Finer than any service the plan asks for.
const IDLE_TICK: Duration = Duration::from_secs(1);

/// The effect seam, as a recorder. See the module comment for why.
pub struct RecordingHost {
    journal: Journal,
    recorded: Vec<Value>,
}

impl RecordingHost {
    pub fn new(journal: Journal) -> Self {
        Self { journal, recorded: Vec::new() }
    }
}

impl ActionHost for RecordingHost {
    fn invoke(&mut self, req: &ActionRequest) -> Result<ActionOutcome, HostError> {
        let (kind, detail) = match &req.kind {
            ActionKind::Dialog { speech, text, priority } => (
                "dialog",
                json!({ "speech": speech, "text": text, "priority": format!("{priority:?}").to_lowercase() }),
            ),
            ActionKind::Logic { component_ref, wit_interface, function, args, fuel_budget } => (
                "logic",
                json!({ "component": component_ref, "interface": wit_interface, "function": function, "args": args, "fuel_budget": fuel_budget }),
            ),
            ActionKind::Compensate { action_id, compensating_action } => (
                "compensate",
                json!({ "action": action_id, "compensation": compensating_action }),
            ),
        };

        let record = json!({
            "service": req.service_id,
            "action": req.action_id,
            "execution": req.execution_id,
            "kind": kind,
            "detail": detail,
            "performed": false,
            "dry_run": true,
            "why": "agentd records service effects instead of performing them",
        });
        self.journal.push(Payload::AgUi(AgUiEvent::Custom {
            name: "svc.effect_requested".to_string(),
            value: record.clone(),
        }));
        if self.recorded.len() >= RECORDED_MAX {
            self.recorded.remove(0);
        }
        self.recorded.push(record);

        Ok(ActionOutcome {
            output: json!({ "performed": false, "dry_run": true }),
            fuel_used: 0,
            network_bytes: 0,
            // svcd charges this against the action's `timeout.total_ms`; one
            // millisecond says "this really was handled", zero looks faked.
            elapsed_ms: 1,
            wakelock_ms: 0,
        })
    }
}

#[derive(Debug)]
enum Cmd {
    Create(Value),
    List,
    Describe(String),
    Stop(String, String),
    Tick,
    Ping,
}

/// Handle to the service executor thread. Cheap to clone, safe to share.
#[derive(Clone)]
pub struct ServiceHub {
    tx: Arc<Mutex<Sender<(Cmd, Sender<Value>)>>>,
}

impl ServiceHub {
    /// Start the executor thread. `sensors` seeds svcd's view of the phone
    /// (battery, charging) so a sensor-triggered service starts from a real
    /// sample instead of nothing.
    ///
    /// The `Svcd` is built *inside* the thread: it holds a `Box<dyn CapsBroker>`,
    /// which is not `Send`, so it must never be moved across threads — but
    /// nothing here needs to be.
    pub fn start(journal: Journal, sensors: BTreeMap<String, Map<String, Value>>) -> ServiceHub {
        let (tx, rx) = channel::<(Cmd, Sender<Value>)>();
        std::thread::spawn(move || {
            let cfg = Config {
                wall_anchor_us: now_us(),
                start_ms: 0,
                trace_capacity: 4096,
                session: json!({ "lang": "en-US" }),
                sensors,
            };
            let mut svcd = Svcd::new(RecordingHost::new(journal.clone()), cfg);
            executor(&mut svcd, &journal, &rx);
        });
        ServiceHub { tx: Arc::new(Mutex::new(tx)) }
    }

    /// Send one command and wait for the executor's answer. A refusal comes
    /// back as `{"error": "..."}` — never as a panic and never as silence.
    fn ask(&self, cmd: Cmd) -> Value {
        let (reply_tx, reply_rx) = channel();
        let send = {
            let guard = self.tx.lock().expect("service hub sender poisoned");
            guard.clone()
        };
        if send.send((cmd, reply_tx)).is_err() {
            return json!({ "error": "the service layer is not running in this agentd" });
        }
        match reply_rx.recv_timeout(Duration::from_secs(30)) {
            Ok(value) => value,
            Err(_) => json!({ "error": "the service layer did not answer within 30 seconds" }),
        }
    }

    /// validate → simulate → admit. Nothing reaches the live executor unless
    /// every embedded test passed in the simulator first.
    pub fn create(&self, ir: &Value) -> Value {
        self.ask(Cmd::Create(ir.clone()))
    }

    pub fn list(&self) -> Value {
        self.ask(Cmd::List)
    }

    pub fn describe(&self, id: &str) -> Value {
        self.ask(Cmd::Describe(id.to_string()))
    }

    /// `svc:stop` through svcd's own lifecycle ingress: the service runs its
    /// `lifecycle_stop` triggers, then stops. Nothing else is touched.
    pub fn stop(&self, id: &str, reason: &str) -> Value {
        self.ask(Cmd::Stop(id.to_string(), reason.to_string()))
    }

    /// Advance the service clock to now, firing anything that came due.
    pub fn tick(&self) {
        self.ask(Cmd::Tick);
    }

    /// True if the executor thread is alive.
    pub fn alive(&self) -> bool {
        self.ask(Cmd::Ping).get("pong").and_then(Value::as_bool).unwrap_or(false)
    }
}

fn executor(mut svcd: &mut Svcd<RecordingHost>, journal: &Journal, rx: &Receiver<(Cmd, Sender<Value>)>) {
    let epoch = Instant::now();
    // Highest svcd journal seq already copied into the agentd journal.
    let mut mirrored = 0u64;
    loop {
        match rx.recv_timeout(IDLE_TICK) {
            Ok((cmd, reply)) => {
                let answer = match cmd {
                    Cmd::Create(ir) => create(&mut svcd, &ir),
                    Cmd::List => list(&svcd, epoch),
                    Cmd::Describe(id) => describe(&svcd, &id),
                    Cmd::Stop(id, reason) => stop(&mut svcd, &id, &reason),
                    Cmd::Tick => {
                        svcd.advance_to(elapsed_ms(epoch));
                        json!({ "ok": true, "clock_ms": svcd.now_ms() })
                    }
                    Cmd::Ping => json!({ "pong": true, "clock_ms": svcd.now_ms() }),
                };
                mirrored = mirror(&svcd, &journal, mirrored);
                let _ = reply.send(answer);
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                svcd.advance_to(elapsed_ms(epoch));
                mirrored = mirror(&svcd, &journal, mirrored);
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                // One last pass so nothing journaled is lost, then the thread ends.
                mirror(&svcd, &journal, mirrored);
                break;
            }
        }
    }
}

fn elapsed_ms(epoch: Instant) -> u64 {
    epoch.elapsed().as_millis() as u64
}

/// Copy svcd's own records into the agentd journal so the kimono view sees
/// service steps as they happen, with svcd still named as the producer.
fn mirror(svcd: &Svcd<RecordingHost>, journal: &Journal, mut mirrored: u64) -> u64 {
    for event in svcd.journal().since(mirrored) {
        mirrored = event.seq;
        journal.push_event(event.clone());
    }
    mirrored
}

fn create(svcd: &mut Svcd<RecordingHost>, ir_value: &Value) -> Value {
    let ir: ServiceIr = match serde_json::from_value(ir_value.clone()) {
        Ok(ir) => ir,
        Err(e) => return json!({ "error": format!("not a ServiceIR: {e}") }),
    };
    if let Err(errors) = svcd::validate::validate(&ir) {
        return json!({ "error": format!("ServiceIR failed validation: {errors}") });
    }

    let reports = svcd::sim::simulate(&ir);
    let failures: Vec<String> = reports
        .iter()
        .filter(|r| !r.passed())
        .map(|r| format!("{}: {}", r.test, r.failures.join(", ")))
        .collect();
    if !failures.is_empty() {
        return json!({
            "error": format!("refused before install — the service does not pass its own simulation: {}", failures.join("; ")),
            "tests": reports.iter().map(|r| json!({ "name": r.test, "passed": r.passed(), "failures": r.failures })).collect::<Vec<_>>(),
        });
    }

    match svcd::sim::admit(svcd, ir.clone()) {
        Ok(reports) => summarize(&ir, &reports),
        Err(err) => json!({ "error": admit_reason(err) }),
    }
}

fn admit_reason(err: AdmitError) -> String {
    match err {
        AdmitError::Install(e) => format!(
            "svcd refused to install it: {}",
            match e {
                InstallError::Invalid(errors) => errors.to_string(),
                InstallError::AlreadyInstalled(id) => format!("{id} is already installed — stop it first or use a new id"),
                InstallError::Denied { capability, reason } => format!("capability `{capability}` refused: {reason}"),
            }
        ),
        AdmitError::TestsFailed(reports) => format!(
            "refused before install — the service does not pass its own simulation: {}",
            reports
                .iter()
                .filter(|r| !r.passed())
                .map(|r| format!("{}: {}", r.test, r.failures.join(", ")))
                .collect::<Vec<_>>()
                .join("; ")
        ),
    }
}

fn summarize(ir: &ServiceIr, reports: &[svcd::sim::SimReport]) -> Value {
    json!({
        "id": ir.id,
        "title": ir.title,
        "version": ir.version,
        "generation": ir.generation,
        "status": "running",
        "admitted_by": "validate, then simulate, then admit",
        "tests": reports.iter().map(|r| json!({ "name": r.test, "passed": r.passed(), "virtual_end_ms": r.end_ms })).collect::<Vec<_>>(),
        "power": ir.power_budget.power_announcement,
        "triggers": ir.triggers.iter().map(|t| json!({ "id": t.id, "kind": t.kind, "entry_action": t.entry_action })).collect::<Vec<_>>(),
    })
}

fn list(svcd: &Svcd<RecordingHost>, epoch: Instant) -> Value {
    let services: Vec<Value> = svcd
        .list()
        .into_iter()
        .map(|(id, status)| {
            let rt = svcd.service(&id);
            json!({
                "id": id,
                "title": rt.map(|r| r.ir.title.clone()).unwrap_or_default(),
                "version": rt.map(|r| r.ir.version.clone()).unwrap_or_default(),
                "generation": rt.map(|r| r.ir.generation).unwrap_or(0),
                "status": status,
                "triggers": rt.map(|r| r.ir.triggers.len()).unwrap_or(0),
                "actions": rt.map(|r| r.ir.actions.len()).unwrap_or(0),
                "capabilities": rt.map(|r| r.ir.capabilities.len()).unwrap_or(0),
                "power": rt.map(|r| r.ir.power_budget.power_announcement.clone()).unwrap_or_default(),
                "state": rt.map(|r| r.state.clone()).unwrap_or(Value::Null),
            })
        })
        .collect();
    json!({
        "count": services.len(),
        "services": services,
        "clock_ms": elapsed_ms(epoch),
        "pending_timers": svcd.pending_timers(),
    })
}

fn describe(svcd: &Svcd<RecordingHost>, id: &str) -> Value {
    let Some(rt) = svcd.service(id) else {
        return json!({ "error": format!("no service `{id}` is installed") });
    };
    json!({
        "id": id,
        "status": rt.status,
        "generation": rt.generation,
        "state": rt.state,
        "ir": rt.ir.as_ref().clone(),
        "grants": rt.grants,
        "active_schemes": rt.active,
        "network_bytes": rt.network_bytes,
        "peak_memory_bytes": rt.peak_memory_bytes,
        "surfaces": svcd.surfaces().collect::<Vec<_>>(),
    })
}

fn stop(svcd: &mut Svcd<RecordingHost>, id: &str, reason: &str) -> Value {
    if svcd.service(id).is_none() {
        return json!({ "error": format!("no service `{id}` is installed") });
    }
    svcd.stop(Some(id), reason);
    let status = svcd.service(id).map(|rt| rt.status.clone());
    match status {
        Some(status) => json!({ "id": id, "status": status_text(&status), "reason": reason }),
        None => json!({ "id": id, "status": "removed", "reason": reason }),
    }
}

/// A short, honest status line for a service.
pub fn status_text(status: &ServiceStatus) -> String {
    match status {
        ServiceStatus::Running => "running".to_string(),
        ServiceStatus::Stopped { reason, violation } => match violation {
            Some(v) => format!("stopped: {reason} ({} over {})", v.budget, v.limit),
            None => format!("stopped: {reason}"),
        },
    }
}

/// Seed svcd's sensor view from the same read-only probes `phone_status` uses,
/// so a battery-triggered service starts from a real sample instead of nothing.
pub fn seed_sensors(root: &ProbeRoot) -> BTreeMap<String, Map<String, Value>> {
    let status = crate::phone::probe_json(root);
    let mut battery = Map::new();
    if let Some(pct) = status.get("battery_percent").and_then(Value::as_f64) {
        battery.insert("capacity_pct".to_string(), json!(pct));
    }
    if let Some(state) = status.get("battery_status").and_then(Value::as_str) {
        battery.insert("charging".to_string(), json!(state.eq_ignore_ascii_case("charging")));
    }
    let mut sensors = BTreeMap::new();
    if !battery.is_empty() {
        sensors.insert("sys:power/battery".to_string(), battery);
    }
    sensors
}
