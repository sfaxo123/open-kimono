//! `svcd` — the deterministic Service IR executor (task B4, docs/PLAN.md B4,
//! spec: docs/design/SERVICE-IR.md).
//!
//! A ServiceIR is compiled once (by the agent, or written by hand), validated
//! here, proven in the virtual-time simulator, and then runs with **no LLM per
//! trigger**: voice templates, timers, sensor thresholds, UI actions and bus
//! messages fire a deterministic action DAG inside a transaction; every step
//! is a `trace-events` journal record.

pub mod error;
mod exec;
pub mod expr;
pub mod host;
pub mod intent;
pub mod ir;
mod ir_misc;
pub mod journal;
pub mod ledger;
pub mod patch;
#[cfg(feature = "sandbox")]
pub mod sandbox_host;
pub mod sensor;
pub mod sim;
pub mod template;
pub mod time;
pub mod validate;

pub use error::{Code, IrError, IrErrors};
pub use exec::{BudgetViolation, BusMessage, Config, InstallError, ServiceRt, ServiceStatus, Surface, Svcd};
pub use host::{ActionHost, ActionKind, ActionOutcome, ActionRequest, CapsBroker, DenyCaps, GrantAll, HostError, SimHost};
pub use ir::ServiceIr;
pub use ledger::{ExecStatus, Ledger};
pub use validate::{load, validate};
