//! Host-side events: everything a service does through the sandbox, and every
//! refusal, is recorded so the UI can show it (VISION §3 open kimono, §5 honest).
//! Mapping to the `trace-events` journal schema is the integrator's job.

use alloc::string::String;

use crate::caps::{Capability, DenyReason};
use crate::limits::ResourceDenial;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
}

impl LogLevel {
    pub(crate) fn from_i32(v: i32) -> Self {
        match v {
            0 => LogLevel::Error,
            1 => LogLevel::Warn,
            2 => LogLevel::Info,
            _ => LogLevel::Debug,
        }
    }
}

/// Why a service was stopped. After a stop the instance is not reused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StopReason {
    OutOfFuel { fuel_per_call: u64 },
    Denied { capability: Capability, reason: DenyReason },
    BadHostCall(String),
    Trap(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EventKind {
    Loaded { requested: alloc::vec::Vec<Capability> },
    Log { level: LogLevel, message: String },
    Trace { name: String, payload: String },
    TimerScheduled { token: i32, due_ms: u64 },
    TimerFired { token: i32 },
    KvSet { key: String, value_bytes: usize },
    Denied { capability: Capability, reason: DenyReason },
    ResourceDenied(ResourceDenial),
    Stopped(StopReason),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostEvent {
    pub service: String,
    pub at_ms: u64,
    pub kind: EventKind,
}
