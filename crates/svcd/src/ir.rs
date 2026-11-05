//! Typed `ServiceIR` (docs/design/SERVICE-IR.md §2.1, schema v1.0.0).
//!
//! Field names and enums follow the D1 JSON Schema. `deny_unknown_fields` is
//! set exactly where the schema says `additionalProperties: false`, so serde
//! rejects the same unknown keys the schema does. Ranges, patterns and
//! cross-references are checked in [`crate::validate`].

use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};

fn one() -> u64 {
    1
}
fn yes() -> bool {
    true
}
/// Keeps an explicit JSON `null` distinct from an absent field.
fn present<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Value>, D::Error> {
    Value::deserialize(d).map(Some)
}

macro_rules! str_enum {
    ($name:ident { $($variant:ident = $text:literal),+ $(,)? }) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        pub enum $name { $(#[serde(rename = $text)] $variant),+ }
        impl $name {
            pub fn as_str(self) -> &'static str { match self { $($name::$variant => $text),+ } }
        }
    };
}

str_enum!(ExecutionMode { Ephemeral = "ephemeral_component", WarmPooled = "warm_pooled_component", Singleton = "singleton_service" });
str_enum!(PriorityClass {
    P0 = "P0_SAFETY_THERMAL", P1 = "P1_USER_ABORT_STOP", P2 = "P2_INTERACTIVE_VOICE_UI",
    P3 = "P3_ACTIVE_FOREGROUND_NAV", P4 = "P4_BACKGROUND_MONITOR",
});
str_enum!(Provenance { UserVoice = "user_voice", UserText = "user_text", SystemBuiltin = "system_builtin", Community = "community_registry", Llm = "llm_synthesized" });
str_enum!(InputType { String = "string", Number = "number", Boolean = "boolean", Date = "date", DurationSec = "duration_sec", Select = "select" });
str_enum!(Requirement { Required = "required", Optional = "optional", UserPrompt = "user_prompt" });
str_enum!(TriggerKind { Voice = "voice_intent", Bus = "bus_message", Timer = "timer_schedule", Sensor = "sensor_threshold", Ui = "ui_action", LifecycleStop = "lifecycle_stop" });
str_enum!(IntentMethod { Template = "template", Keyword = "keyword", Regex = "regex" });
str_enum!(SlotType { String = "string", Integer = "integer", Float = "float", Duration = "duration", Enum = "enum" });
str_enum!(TimerMode { Countdown = "countdown_monotonic", Interval = "interval_monotonic", CronWall = "cron_wall", OneshotIso = "oneshot_iso" });
str_enum!(CatchupPolicy { AdvanceBeforeDispatch = "advance_before_dispatch", DropMissed = "drop_missed", FireOnceWithinGrace = "fire_once_within_grace" });
str_enum!(CmpOp { Lt = "lt", Lte = "lte", Gt = "gt", Gte = "gte", Eq = "eq" });
str_enum!(PollOrIrq { IrqEvent = "irq_event", SpmiComparator = "spmi_comparator", PollSlow = "poll_slow" });
str_enum!(ActionType { Logic = "logic", Dialog = "dialog", UiPatch = "ui_patch", BusEmit = "bus_emit", StateMutate = "state_mutate", SubService = "sub_service" });
str_enum!(EffectClass { Pure = "pure", ReversibleStaged = "reversible_staged", ReversibleCompensated = "reversible_compensated", CommittedIrreversible = "committed_irreversible" });
str_enum!(Concurrency { Serial = "serial", Parallel = "parallel" });
str_enum!(PatchOp { Add = "add", Replace = "replace", Remove = "remove", Test = "test" });
str_enum!(DialogPriority { Normal = "normal", Alert = "alert", Critical = "critical" });
str_enum!(UiOperation { CreateSurface = "create_surface", UpdateComponents = "update_components", UpdateDataModel = "update_data_model", DeleteSurface = "delete_surface", OpenuiStatements = "openui_statements" });
str_enum!(Derivation { Forward = "forward", Reply = "reply", Response = "response", Root = "root" });
str_enum!(SchemeAccess { Read = "read", Write = "write", ReadWrite = "read_write", EventSubscribe = "event_subscribe" });
str_enum!(Lifecycle { DemandPerAction = "demand_per_action", WhileSurfaceVisible = "while_surface_visible", WhileGrantAlive = "while_grant_alive", AlwaysResident = "always_resident_kernel" });
str_enum!(CapLifetime { PerInvocation = "per_invocation", WhileVisible = "while_visible", UntilDeadline = "until_deadline", StandingWatch = "standing_trigger_watch" });
str_enum!(Durability { Ephemeral = "ephemeral_session", SqliteWal = "sqlite_wal_durable" });
str_enum!(UiMode { None = "none", A2ui = "a2ui_v1", Openui = "openui_stream" });
str_enum!(CpuCluster { SilverOnly = "silver_only", SilverPreferred = "silver_preferred", GoldBurst = "gold_burst_allowed" });
str_enum!(CheckType { StateAssertion = "state_assertion", SchemeHealth = "scheme_health" });
str_enum!(SimEventType { Utterance = "utterance", BusMessage = "bus_message", AdvanceClock = "advance_clock", SensorSample = "sensor_sample", UiAction = "ui_action", StopSignal = "stop_signal" });

// The enum comes from `str_enum!`, which cannot carry `#[default]`.
#[allow(clippy::derivable_impls)]
impl Default for PriorityClass {
    fn default() -> Self {
        PriorityClass::P2
    }
}

/// The capability vocabulary of the `caps:` broker (schema enum).
pub const CAPABILITIES: [&str; 11] = [
    "audio.capture", "audio.playback.tts", "audio.playback.alarm", "location.gnss.read", "storage.maps.readonly",
    "state.namespace.readwrite", "power.battery.read", "power.request_wake", "timer.monotonic.schedule",
    "ui.surface.render", "network.connect",
];

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceIr {
    pub ir_version: String,
    pub id: String,
    pub version: String,
    #[serde(default = "one")]
    pub generation: u64,
    #[serde(default)]
    pub content_hash: Option<String>,
    pub title: String,
    pub description: String,
    #[serde(default)]
    pub author: Option<Author>,
    pub execution_mode: ExecutionMode,
    #[serde(default)]
    pub priority_class: PriorityClass,
    #[serde(default)]
    pub parameters: Vec<Parameter>,
    pub triggers: Vec<Trigger>,
    #[serde(default)]
    pub workflow: Option<Vec<String>>,
    pub actions: BTreeMap<String, ActionNode>,
    pub deps: Dependencies,
    pub capabilities: Vec<CapabilityRequest>,
    pub state_schema: StateSpec,
    pub ui: UiSpec,
    pub power_budget: PowerBudget,
    #[serde(default)]
    pub retry: Option<RetryConfig>,
    pub tests: Vec<SimTestCase>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Author {
    pub name: String,
    #[serde(default)]
    pub contact: Option<String>,
    pub provenance: Provenance,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Parameter {
    pub key: String,
    pub input_type: InputType,
    pub requirement: Requirement,
    pub description: String,
    #[serde(default, deserialize_with = "present")]
    pub default: Option<Value>,
    #[serde(default)]
    pub options: Option<Vec<String>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Trigger {
    pub id: String,
    pub kind: TriggerKind,
    pub entry_action: String,
    #[serde(default)]
    pub condition_cel: Option<String>,
    #[serde(default)]
    pub debounce_ms: u64,
    #[serde(default)]
    pub cooldown_ms: u64,
    #[serde(default)]
    pub voice_intent: Option<VoiceIntent>,
    #[serde(default)]
    pub bus_message: Option<BusMatch>,
    #[serde(default)]
    pub timer_schedule: Option<TimerSchedule>,
    #[serde(default)]
    pub sensor_threshold: Option<SensorThreshold>,
    #[serde(default)]
    pub ui_action: Option<UiActionMatch>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoiceIntent {
    pub domain: String,
    pub intent_name: String,
    pub lang: String,
    #[serde(default = "IntentMethod::default_method")]
    pub method: IntentMethod,
    #[serde(default = "yes")]
    pub speech_to_phrase: bool,
    pub sentences: Vec<String>,
    #[serde(default)]
    pub slots: BTreeMap<String, SlotSpec>,
}

impl IntentMethod {
    fn default_method() -> Self {
        IntentMethod::Template
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SlotSpec {
    #[serde(rename = "type")]
    pub ty: SlotType,
    pub required: bool,
    #[serde(default, deserialize_with = "present")]
    pub default: Option<Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BusMatch {
    pub msg_type: String,
    #[serde(default)]
    pub data_filter: Option<Map<String, Value>>,
    #[serde(default = "yes")]
    pub require_session: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimerSchedule {
    pub mode: TimerMode,
    #[serde(default)]
    pub duration_sec_expr: Option<String>,
    #[serde(default)]
    pub cron_expr: Option<String>,
    #[serde(default = "CatchupPolicy::default_policy")]
    pub catchup_policy: CatchupPolicy,
    #[serde(default = "TimerSchedule::default_grace")]
    pub grace_window_sec: u64,
    #[serde(default)]
    pub wake_from_suspend: bool,
}

impl CatchupPolicy {
    fn default_policy() -> Self {
        CatchupPolicy::AdvanceBeforeDispatch
    }
}
impl TimerSchedule {
    fn default_grace() -> u64 {
        120
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SensorThreshold {
    pub scheme_path: String,
    pub metric: String,
    pub operator: CmpOp,
    pub threshold: f64,
    pub hysteresis_reset: f64,
    #[serde(default = "yes")]
    pub edge_only: bool,
    #[serde(default)]
    pub poll_or_irq: Option<PollOrIrq>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiActionMatch {
    pub surface_id: String,
    pub action_name: String,
    #[serde(default)]
    pub source_component_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionNode {
    #[serde(rename = "type")]
    pub ty: ActionType,
    pub description: String,
    pub effect_class: EffectClass,
    #[serde(default)]
    pub idempotency_key_template: Option<String>,
    pub read_only: bool,
    #[serde(default = "yes")]
    pub deduplicate_calls: bool,
    #[serde(default = "Concurrency::default_serial")]
    pub concurrency: Concurrency,
    pub timeout: Timeout,
    #[serde(default)]
    pub wasm_call: Option<WasmCall>,
    #[serde(default)]
    pub state_patch: Option<Vec<PatchOpSpec>>,
    #[serde(default)]
    pub dialog: Option<Dialog>,
    #[serde(default)]
    pub ui_patch: Option<UiPatch>,
    #[serde(default)]
    pub bus_emit: Option<BusEmit>,
    #[serde(default)]
    pub compensating_action: Option<String>,
    #[serde(default)]
    pub next: Vec<Edge>,
}

impl Concurrency {
    fn default_serial() -> Self {
        Concurrency::Serial
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Timeout {
    pub idle_ms: u64,
    pub total_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WasmCall {
    pub component_ref: String,
    pub wit_interface: String,
    pub function: String,
    pub args_mapping: Map<String, Value>,
    #[serde(default)]
    pub output_state_pointer: Option<String>,
    #[serde(default)]
    pub post_hook_jq: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PatchOpSpec {
    pub op: PatchOp,
    pub path: String,
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dialog {
    pub speech: String,
    pub text: String,
    #[serde(default = "DialogPriority::default_normal")]
    pub priority: DialogPriority,
}

impl DialogPriority {
    fn default_normal() -> Self {
        DialogPriority::Normal
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiPatch {
    pub surface_id: String,
    pub operation: UiOperation,
    #[serde(default)]
    pub data_model_path: Option<String>,
    #[serde(default, deserialize_with = "present")]
    pub data_model_value: Option<Value>,
    #[serde(default)]
    pub openui_statements: Option<Vec<String>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BusEmit {
    pub derivation: Derivation,
    pub msg_type: String,
    pub data: Map<String, Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Edge {
    #[serde(default)]
    pub when_cel: Option<String>,
    pub target: String,
}

pub use crate::ir_misc::*;
