//! `svc:simulate` (SERVICE-IR.md §5): run a ServiceIR's embedded tests in the
//! same executor, on a virtual clock, with every effect captured by a
//! [`SimHost`]. Hundreds of virtual seconds cost microseconds; nothing sleeps.
//!
//! After the last event the simulator keeps the clock running for the
//! longest `debounce_ms` of the service, so a crossing in the final sample
//! is judged after its debounce window, as on the device.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use serde_json::{json, Map, Value};
use trace_events::Event;

use crate::exec::{BusMessage, Config, InstallError, ServiceStatus, Surface, Svcd};
use crate::expr::json_subset;
use crate::host::{ActionHost, ActionRequest, CapsBroker, GrantAll, SimHost};
use crate::ir::{ServiceIr, SimEventType, SimTestCase};
use crate::time::parse_iso_utc_us;

#[derive(Clone, Debug, Serialize)]
pub struct SimReport {
    pub service: String,
    pub test: String,
    /// Empty when every assertion held.
    pub failures: Vec<String>,
    pub final_state: Value,
    pub status: Option<ServiceStatus>,
    pub bus: Vec<BusMessage>,
    pub surfaces: Vec<Surface>,
    pub peak_memory_bytes: u64,
    pub network_bytes: u64,
    pub active_schemes: Vec<String>,
    pub host_calls: Vec<ActionRequest>,
    pub journal: Vec<Event>,
    pub end_ms: u64,
}

impl SimReport {
    pub fn passed(&self) -> bool {
        self.failures.is_empty()
    }
}

const MIB: f64 = 1024.0 * 1024.0;

/// Executor configured from a test's `initial_env`.
pub fn svcd_for<H: ActionHost>(test: &SimTestCase, host: H) -> Svcd<H> {
    let env = &test.initial_env;
    let mut sensors = BTreeMap::new();
    let battery = json!({ "scheme": "sys:power/battery", "capacity_pct": env.battery_pct, "charging": env.charging });
    sensors.insert("sys:power/battery".to_string(), battery.as_object().unwrap().clone());
    let cfg = Config {
        wall_anchor_us: parse_iso_utc_us(&env.wall_iso).unwrap_or(0).saturating_sub(env.monotonic_ms * 1000),
        start_ms: env.monotonic_ms,
        session: json!({ "location": env.location, "lang": "en-US", "network_online": env.network_online }),
        sensors,
        ..Config::default()
    };
    Svcd::new(host, cfg)
}

/// Feed a test's events into an executor that already has the service.
pub fn drive<H: ActionHost>(svcd: &mut Svcd<H>, ir: &ServiceIr, test: &SimTestCase) {
    let s = |p: &Map<String, Value>, a: &str, b: &str| p.get(a).or_else(|| p.get(b)).and_then(Value::as_str).map(str::to_string);
    for ev in &test.events {
        svcd.advance_to(ev.at_monotonic_ms);
        let p = &ev.payload;
        match ev.event_type {
            SimEventType::Utterance => {
                svcd.utterance(&s(p, "text", "text").unwrap_or_default(), s(p, "utterance_id", "utteranceId").as_deref());
            }
            SimEventType::SensorSample => svcd.sensor_sample(&s(p, "scheme", "scheme").unwrap_or_default(), p.clone()),
            SimEventType::UiAction => {
                let surface = s(p, "surfaceId", "surface_id").unwrap_or_default();
                let name = s(p, "name", "action_name").unwrap_or_default();
                svcd.ui_action(&surface, &name, s(p, "sourceComponentId", "source_component_id").as_deref());
            }
            SimEventType::BusMessage => svcd.bus_message(&s(p, "msg_type", "type").unwrap_or_default(), p.get("data").cloned().unwrap_or(json!({}))),
            SimEventType::StopSignal => svcd.stop(s(p, "service_id", "service").as_deref(), "stop signal"),
            SimEventType::AdvanceClock => {}
        }
    }
    let last = test.events.last().map_or(test.initial_env.monotonic_ms, |e| e.at_monotonic_ms);
    let settle = ir.triggers.iter().map(|t| t.debounce_ms).max().unwrap_or(0);
    svcd.advance_to(last + settle);
}

/// Run one embedded test with a given host and broker.
pub fn run_test<H: ActionHost>(ir: &ServiceIr, test: &SimTestCase, host: H, broker: impl CapsBroker + 'static) -> (SimReport, Svcd<H>) {
    let mut svcd = svcd_for(test, host).with_broker(broker);
    let mut failures = Vec::new();
    if let Err(e) = svcd.install(ir.clone()) {
        failures.push(format!("install failed: {e}"));
    } else {
        drive(&mut svcd, ir, test);
    }
    let report = report(&svcd, ir, test, failures);
    (report, svcd)
}

/// Run every embedded test with a fresh [`SimHost`] and [`GrantAll`].
pub fn simulate(ir: &ServiceIr) -> Vec<SimReport> {
    ir.tests.iter().map(|t| run_test(ir, t, SimHost::new(), GrantAll).0).collect()
}

fn report<H: ActionHost>(svcd: &Svcd<H>, ir: &ServiceIr, test: &SimTestCase, mut failures: Vec<String>) -> SimReport {
    let rt = svcd.service(&ir.id);
    let final_state = rt.map_or(Value::Null, |r| r.state.clone());
    let surfaces: Vec<Surface> = svcd.surfaces().cloned().collect();
    let active: Vec<String> = rt.map(|r| r.active.iter().cloned().collect()).unwrap_or_default();
    let peak = rt.map_or(0, |r| r.peak_memory_bytes);
    let net = rt.map_or(0, |r| r.network_bytes);
    let a = &test.assertions;

    if !json_subset(&Value::Object(a.final_state_subset.clone()), &final_state) {
        failures.push(format!("final state {final_state} does not contain {}", Value::Object(a.final_state_subset.clone())));
    }
    let mut cursor = svcd.bus().iter();
    for exp in &a.expected_bus_emissions {
        let ty = exp.get("msg_type").and_then(Value::as_str).unwrap_or("");
        let data = exp.get("data").cloned().unwrap_or(json!({}));
        if !cursor.any(|m| m.msg_type == ty && json_subset(&data, &m.data)) {
            let seen: Vec<&str> = svcd.bus().iter().map(|m| m.msg_type.as_str()).collect();
            failures.push(format!("bus emission {} not found in order; bus was {seen:?}", Value::Object(exp.clone())));
            break;
        }
    }
    for exp in &a.expected_ui_surfaces {
        let found = surfaces.iter().any(|s| json_subset(&Value::Object(exp.clone()), &serde_json::to_value(s).unwrap()));
        if !found {
            failures.push(format!("no surface matching {}", Value::Object(exp.clone())));
        }
    }
    let b = &a.budget_assertions;
    if peak as f64 / MIB > b.max_memory_mb_used {
        failures.push(format!("peak memory {peak} bytes exceeds {} MB", b.max_memory_mb_used));
    }
    if net != b.network_bytes_sent {
        failures.push(format!("network bytes {net} != expected {}", b.network_bytes_sent));
    }
    let want: BTreeSet<&String> = b.active_schemes_after_completion.iter().collect();
    let got: BTreeSet<&String> = active.iter().collect();
    if want != got {
        failures.push(format!("active schemes {got:?} != expected {want:?}"));
    }
    let journal = svcd.journal().snapshot();
    if journal.iter().any(|e| matches!(e.payload, trace_events::Payload::GenAi(_))) {
        failures.push("an LLM span was journaled during a deterministic run".into());
    }
    SimReport {
        service: ir.id.clone(),
        test: test.name.clone(),
        failures,
        final_state,
        status: rt.map(|r| r.status.clone()),
        bus: svcd.bus().to_vec(),
        surfaces,
        peak_memory_bytes: peak,
        network_bytes: net,
        active_schemes: active,
        host_calls: Vec::new(),
        journal,
        end_ms: svcd.now_ms(),
    }
}

#[derive(Debug)]
pub enum AdmitError {
    Install(InstallError),
    TestsFailed(Vec<SimReport>),
}

/// The verification gate (SERVICE-IR.md §5): an IR is installed into a live
/// executor only if every one of its embedded tests passes in simulation.
pub fn admit<H: ActionHost>(svcd: &mut Svcd<H>, ir: ServiceIr) -> Result<Vec<SimReport>, AdmitError> {
    crate::validate::validate(&ir).map_err(|e| AdmitError::Install(InstallError::Invalid(e)))?;
    let reports = simulate(&ir);
    if reports.iter().any(|r| !r.passed()) {
        return Err(AdmitError::TestsFailed(reports));
    }
    svcd.install(ir).map_err(AdmitError::Install)?;
    Ok(reports)
}

impl SimReport {
    /// Attach the host's recorded calls (for scripted-host tests).
    pub fn with_calls(mut self, host: &SimHost) -> Self {
        self.host_calls = host.calls.clone();
        self
    }
}
