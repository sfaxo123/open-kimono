//! The non-graph half of the IR: deps, capabilities, state, UI, power budget,
//! retry and simulator test cases (SERVICE-IR.md §2.1 `$defs`).

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::ir::{
    CapLifetime, CheckType, CpuCluster, Durability, Lifecycle, SchemeAccess, SimEventType, UiMode,
};

fn yes() -> bool {
    true
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dependencies {
    pub schemes: Vec<SchemeDep>,
    pub wasm_components: Vec<WasmComponent>,
    pub sub_services: Vec<SubService>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SchemeDep {
    pub scheme: String,
    pub access: SchemeAccess,
    pub lifecycle: Lifecycle,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WasmComponent {
    pub id: String,
    pub digest: String,
    pub wit_world: WitWorld,
    pub pool_policy: PoolPolicy,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WitWorld {
    pub imports: Vec<String>,
    pub exports: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PoolPolicy {
    pub pool_size: u64,
    pub max_invocations: u64,
    pub max_concurrency: u64,
    pub reclaim_window_seconds: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubService {
    pub name: String,
    pub service_id: String,
    #[serde(default)]
    pub min_version: Option<String>,
    #[serde(default)]
    pub values: Option<Map<String, Value>>,
    #[serde(default = "yes")]
    pub sequential_when_repeated: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityRequest {
    pub cap_id: String,
    /// One of [`crate::ir::CAPABILITIES`] (checked by the validator so the
    /// error names the allowed set).
    pub capability: String,
    pub scope: Map<String, Value>,
    pub lifetime: CapLifetime,
    pub max_duration_sec: u64,
    #[serde(default = "yes")]
    pub revocable: bool,
    pub audit_reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StateSpec {
    pub namespace: String,
    pub durability: Durability,
    #[serde(default)]
    pub wal_limit_bytes: Option<u64>,
    #[serde(default)]
    pub retention_ttl_sec: Option<u64>,
    pub schema: Map<String, Value>,
    pub initial_state: Map<String, Value>,
    #[serde(default)]
    pub migrations: Vec<Migration>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Migration {
    pub from_version: String,
    pub to_version: String,
    pub json_patch: Vec<Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiSpec {
    pub mode: UiMode,
    #[serde(default)]
    pub surface_id: Option<String>,
    #[serde(default)]
    pub catalog_id: Option<String>,
    #[serde(default = "yes")]
    pub send_data_model: bool,
    #[serde(default)]
    pub initial_messages: Vec<Map<String, Value>>,
    #[serde(default)]
    pub openui_statements: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PowerBudget {
    pub cpu_cluster_preference: CpuCluster,
    pub max_cpu_cores: u64,
    pub cpu_quota_pct: u64,
    pub memory_limit_mb: u64,
    pub wasm_fuel_per_invocation: u64,
    pub max_wakeups_per_minute: f64,
    pub max_wakelock_sec: u64,
    pub network_egress_bytes_max: u64,
    pub allowed_hosts: Vec<String>,
    pub allowed_ip_name_lookups: Vec<String>,
    pub allowed_host_loopback_ports: Vec<i64>,
    pub auto_stop_idle_ms: u64,
    pub power_announcement: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetryConfig {
    pub max_retries: u64,
    pub timeout_seconds: u64,
    #[serde(default)]
    pub on_failure_action: Option<String>,
    #[serde(default)]
    pub on_failure_timeout_seconds: Option<u64>,
    pub checks: Vec<RetryCheck>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RetryCheck {
    #[serde(rename = "type")]
    pub ty: CheckType,
    pub expr: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SimTestCase {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub initial_env: InitialEnv,
    pub events: Vec<SimEvent>,
    pub assertions: Assertions,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InitialEnv {
    pub monotonic_ms: u64,
    pub wall_iso: String,
    pub battery_pct: f64,
    pub charging: bool,
    pub network_online: bool,
    pub location: Location,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Location {
    pub lat: f64,
    pub lon: f64,
    pub tz: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SimEvent {
    pub at_monotonic_ms: u64,
    pub event_type: SimEventType,
    pub payload: Map<String, Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Assertions {
    pub final_state_subset: Map<String, Value>,
    pub expected_bus_emissions: Vec<Map<String, Value>>,
    pub expected_ui_surfaces: Vec<Map<String, Value>>,
    pub budget_assertions: BudgetAssertions,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BudgetAssertions {
    pub max_memory_mb_used: f64,
    pub network_bytes_sent: u64,
    pub active_schemes_after_completion: Vec<String>,
}

impl crate::ir::Lifecycle {
    pub fn is_demand(self) -> bool {
        self == crate::ir::Lifecycle::DemandPerAction
    }
}
