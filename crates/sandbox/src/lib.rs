//! Runs a user-built service as a WebAssembly module on wasmi.
//!
//! The `op6` host functions are the service's only access to the OS, and each
//! one is gated by a [`Grant`]: `log`, `trace.emit`, `timer.after`, `kv.get`,
//! `kv.set` (KV is scoped to the service's own namespace). Every entry-point
//! call gets a fixed fuel budget and the instance has a memory cap, so a
//! runaway or greedy service is stopped and the reason is recorded
//! (VISION §3 open kimono, §5 honest, §8 minimal).
//!
//! `no_std` + `alloc`; the `std` feature only forwards to wasmi.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

mod caps;
mod error;
mod events;
mod host;
mod kv;
mod limits;
mod service;

pub use caps::{Capability, DenyReason, Grant, Grants};
pub use error::SandboxError;
pub use events::{EventKind, HostEvent, LogLevel, StopReason};
pub use host::{ERR_NOT_FOUND, ERR_QUOTA, HOST_CALL_FUEL, IMPORT_MODULE, OK};
pub use kv::{KvBackend, MemKv};
pub use limits::{Limits, ResourceDenial, WASM_PAGE_BYTES};
pub use service::{CallReport, Sandbox, Service, ServiceSpec};
