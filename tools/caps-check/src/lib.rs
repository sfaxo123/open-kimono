//! Reference model for the `caps:` capability broker + Wasm sandbox design.
//!
//! This crate is the *design oracle* for `docs/design/CAPS-SANDBOX.md` (D2): the parts of that
//! design that have to be exact (grant scope matching, expiry, revocation reaching in-flight
//! work, budget accounting, and the "one authority, one budget" rule that keeps the host from
//! having to wake a daemon to answer `check` on a hot path). It is not production code and it is
//! not a device model: a driver's register behaviour is replayed from a recorded oracle trace
//! (that is `crates/mmio-replay`, task C1); this model only pins the *authority* semantics, so a
//! rule written in the design can be shown to be self-consistent before any of it runs on a
//! phone.
//!
//! Two facts make it host-only for now: Redox has no Wasm engine today, and neither wasmi nor
//! tinywasm can parse a Wasm *component*, so the design's host-function layer is a host-side shim
//! over core modules. Everything here therefore runs as plain host code against a fake monotonic
//! clock, which is what makes the grant model testable at all.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod audit;
pub mod clock;
pub mod grant;
pub mod manifest;
pub mod power;
pub mod scope;

pub use audit::{Event, EventKind, Journal};
pub use clock::{Clock, Monotonic, Nanos};
pub use grant::{Budget, Decision, Denied, Grant, GrantId, GrantState, GrantTable, PowerReq, Request, NO_GRANT};
pub use manifest::{DepDecl, Manifest, ServiceKind, UseDecl, CATALOGUE, SPENDS_POWER};
pub use power::{Domain, Hold, PowerState, WakeLedger};
pub use scope::{Scope, Unit, Want};
