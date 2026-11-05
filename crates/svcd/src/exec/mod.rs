//! The deterministic executor (SERVICE-IR.md §3). One `Svcd` holds every
//! installed service, a virtual monotonic clock, the timer heap, the ledger
//! and the journal. Time only moves when the caller says so ([`Svcd::advance_to`]),
//! which is what makes the simulator and the daemon the same code.

mod effects;
mod ingress;
mod power;
mod run;
mod ui;

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;

use serde::Serialize;
use serde_json::{json, Map, Value};
use trace_events::{AgUiEvent, Op6Event};

use crate::error::IrErrors;
use crate::host::{ActionHost, CapsBroker, GrantAll};
use crate::ir::{CapLifetime, ServiceIr, TimerMode, TriggerKind};
use crate::journal::Journal;
use crate::ledger::Ledger;
use crate::sensor::SensorTrack;

pub use ui::Surface;

/// One OVOS-MSG-1 bus message `{type, data, context}`.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct BusMessage {
    #[serde(rename = "type")]
    pub msg_type: String,
    pub data: Value,
    pub context: Value,
    pub at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct BudgetViolation {
    /// The `power_budget` field that was exceeded.
    pub budget: String,
    pub limit: String,
    pub observed: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ServiceStatus {
    Running,
    Stopped { reason: String, violation: Option<BudgetViolation> },
}

pub struct ServiceRt {
    pub ir: Arc<ServiceIr>,
    pub state: Value,
    /// Bumped by every commit that changed state.
    pub generation: u64,
    pub status: ServiceStatus,
    pub(crate) sensors: BTreeMap<String, SensorTrack>,
    pub(crate) last_fire: BTreeMap<String, u64>,
    pub(crate) wakeups: VecDeque<u64>,
    pub network_bytes: u64,
    pub peak_memory_bytes: u64,
    /// Schemes currently powered for this service.
    pub active: BTreeSet<String>,
    /// Live non-per-invocation grants by `cap_id`.
    pub grants: BTreeSet<String>,
    /// Pending countdown/interval timer per trigger id.
    pub(crate) timers: BTreeMap<String, (u64, u64)>,
    /// `deduplicate_calls` cache: key → output, valid for one state generation.
    pub(crate) dedup: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum TimerEntry {
    Schedule { service: String, trigger: String },
    Debounce { service: String, trigger: String },
}

#[derive(Clone, Debug)]
pub struct Config {
    /// Wall clock (µs since epoch) at monotonic 0; only used for trace stamps.
    pub wall_anchor_us: u64,
    pub start_ms: u64,
    pub trace_capacity: usize,
    /// OVOS session: `{ "location": {lat, lon, tz}, "lang": ... }`.
    pub session: Value,
    /// Latest known sample per scheme (e.g. battery at boot).
    pub sensors: BTreeMap<String, Map<String, Value>>,
}

impl Default for Config {
    fn default() -> Self {
        Self { wall_anchor_us: 0, start_ms: 0, trace_capacity: 4096, session: json!({}), sensors: BTreeMap::new() }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum InstallError {
    Invalid(IrErrors),
    AlreadyInstalled(String),
    Denied { capability: String, reason: String },
}

impl std::fmt::Display for InstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InstallError::Invalid(e) => write!(f, "{e}"),
            InstallError::AlreadyInstalled(id) => write!(f, "{id} is already installed"),
            InstallError::Denied { capability, reason } => write!(f, "capability `{capability}` refused: {reason}"),
        }
    }
}

pub struct Svcd<H: ActionHost> {
    pub(crate) now_ms: u64,
    pub(crate) services: BTreeMap<String, ServiceRt>,
    pub(crate) order: Vec<String>,
    pub(crate) timers: BTreeMap<(u64, u64), TimerEntry>,
    pub(crate) seq: u64,
    pub(crate) host: H,
    pub(crate) broker: Box<dyn CapsBroker>,
    pub(crate) ledger: Ledger,
    pub(crate) journal: Journal,
    pub(crate) bus: Vec<BusMessage>,
    pub(crate) surfaces: BTreeMap<String, Surface>,
    pub(crate) sensors: BTreeMap<String, Map<String, Value>>,
    pub(crate) session: Value,
}

impl<H: ActionHost> Svcd<H> {
    pub fn new(host: H, cfg: Config) -> Self {
        Self {
            now_ms: cfg.start_ms,
            services: BTreeMap::new(),
            order: Vec::new(),
            timers: BTreeMap::new(),
            seq: 0,
            host,
            broker: Box::new(GrantAll),
            ledger: Ledger::new(),
            journal: Journal::new(cfg.trace_capacity, cfg.wall_anchor_us),
            bus: Vec::new(),
            surfaces: BTreeMap::new(),
            sensors: cfg.sensors,
            session: cfg.session,
        }
    }

    pub fn with_broker(mut self, broker: impl CapsBroker + 'static) -> Self {
        self.broker = Box::new(broker);
        self
    }

    /// Restore a ledger persisted by an earlier run (restart path).
    pub fn with_ledger(mut self, ledger: Ledger) -> Self {
        self.ledger = ledger;
        self
    }

    pub fn now_ms(&self) -> u64 {
        self.now_ms
    }
    pub fn host(&self) -> &H {
        &self.host
    }
    pub fn host_mut(&mut self) -> &mut H {
        &mut self.host
    }
    pub fn ledger(&self) -> &Ledger {
        &self.ledger
    }
    pub fn journal(&self) -> &trace_events::EventRing {
        self.journal.ring()
    }
    pub fn bus(&self) -> &[BusMessage] {
        &self.bus
    }
    pub fn surfaces(&self) -> impl Iterator<Item = &Surface> {
        self.surfaces.values()
    }
    pub fn service(&self, id: &str) -> Option<&ServiceRt> {
        self.services.get(id)
    }
    pub fn state(&self, id: &str) -> Option<&Value> {
        self.services.get(id).map(|s| &s.state)
    }
    /// `svc:list` — installed ids with status, install order.
    pub fn list(&self) -> Vec<(String, ServiceStatus)> {
        self.order.iter().map(|id| (id.clone(), self.services[id].status.clone())).collect()
    }
    /// Pending timer deadlines (ms), earliest first.
    pub fn pending_timers(&self) -> Vec<u64> {
        self.timers.keys().map(|(due, _)| *due).collect()
    }

    /// `svc:start` — install an already-validated IR (re-validated here).
    /// Use [`crate::sim::admit`] to require its simulator tests first.
    pub fn install(&mut self, ir: ServiceIr) -> Result<(), InstallError> {
        crate::validate::validate(&ir).map_err(InstallError::Invalid)?;
        if self.services.contains_key(&ir.id) {
            return Err(InstallError::AlreadyInstalled(ir.id.clone()));
        }
        let ir = Arc::new(ir);
        let id = ir.id.clone();
        for c in ir.capabilities.iter().filter(|c| c.lifetime == CapLifetime::StandingWatch) {
            self.broker
                .grant(&id, c, self.now_ms)
                .map_err(|reason| InstallError::Denied { capability: c.capability.clone(), reason })?;
        }
        let mut sensors = BTreeMap::new();
        for t in ir.triggers.iter().filter(|t| t.kind == TriggerKind::Sensor) {
            let st = t.sensor_threshold.as_ref().expect("validated");
            let initial = self.sensors.get(&st.scheme_path).and_then(|s| s.get(&st.metric)).and_then(Value::as_f64);
            sensors.insert(t.id.clone(), SensorTrack::new(st, initial));
        }
        let state = Value::Object(ir.state_schema.initial_state.clone());
        let peak = serde_json::to_vec(&state).map_or(0, |v| v.len() as u64);
        self.services.insert(
            id.clone(),
            ServiceRt {
                ir: ir.clone(),
                state: state.clone(),
                generation: 0,
                status: ServiceStatus::Running,
                sensors,
                last_fire: BTreeMap::new(),
                wakeups: VecDeque::new(),
                network_bytes: 0,
                peak_memory_bytes: peak,
                active: BTreeSet::new(),
                grants: BTreeSet::new(),
                timers: BTreeMap::new(),
                dedup: BTreeMap::new(),
            },
        );
        self.order.push(id.clone());
        let reason = format!("installed, {}: {}", ir.priority_class.as_str(), ir.power_budget.power_announcement);
        self.journal.op6(self.now_ms, Op6Event::ServiceStarted { service: id.clone(), generation: ir.generation, reason });
        self.journal.agui(self.now_ms, AgUiEvent::StateSnapshot { snapshot: json!({ "service": id, "state": state }) });
        for c in ir.capabilities.iter().filter(|c| c.lifetime == CapLifetime::StandingWatch) {
            self.note_grant(&id, c);
        }
        for t in ir.triggers.iter().filter(|t| t.timer_schedule.as_ref().is_some_and(|ts| ts.mode == TimerMode::Interval)) {
            self.schedule_trigger(&id, &t.id);
        }
        self.after_execution(&id);
        Ok(())
    }

    /// Advance the virtual clock, firing every due timer in deadline order
    /// (the clock reads each deadline as it fires). Never sleeps.
    pub fn advance_to(&mut self, t_ms: u64) {
        while let Some((&(due, seq), _)) = self.timers.first_key_value() {
            if due > t_ms {
                break;
            }
            let entry = self.timers.remove(&(due, seq)).expect("present");
            self.now_ms = self.now_ms.max(due);
            match entry {
                TimerEntry::Schedule { service, trigger } => self.timer_fired(&service, &trigger, due),
                TimerEntry::Debounce { service, trigger } => self.debounce_fired(&service, &trigger),
            }
        }
        self.now_ms = self.now_ms.max(t_ms);
    }

    pub(crate) fn add_timer(&mut self, due: u64, entry: TimerEntry) -> (u64, u64) {
        self.seq += 1;
        let key = (due, self.seq);
        self.timers.insert(key, entry);
        key
    }

    pub(crate) fn next_seq(&mut self) -> u64 {
        self.seq += 1;
        self.seq
    }
}
