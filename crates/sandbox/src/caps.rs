//! Capabilities and grants. A host function runs only if the service holds a
//! live grant for its capability; grants can expire or be revoked while the
//! service runs.

use alloc::vec::Vec;
use core::fmt;

/// One unit of OS access a service can be granted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Capability {
    /// `op6.log` — write a line to the service log.
    Log,
    /// `op6.trace_emit` — emit an open-kimono trace event.
    TraceEmit,
    /// `op6.timer_after` — ask to be called back (`on_timer`) after N ms.
    TimerAfter,
    /// `op6.kv_get` — read from the service's own key/value namespace.
    KvGet,
    /// `op6.kv_set` — write to the service's own key/value namespace.
    KvSet,
}

impl Capability {
    pub const ALL: [Capability; 5] = [
        Capability::Log,
        Capability::TraceEmit,
        Capability::TimerAfter,
        Capability::KvGet,
        Capability::KvSet,
    ];

    /// Dotted name used in manifests, UI and audit events.
    pub fn name(self) -> &'static str {
        match self {
            Capability::Log => "log",
            Capability::TraceEmit => "trace.emit",
            Capability::TimerAfter => "timer.after",
            Capability::KvGet => "kv.get",
            Capability::KvSet => "kv.set",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.name() == name)
    }

    /// The `op6` import that this capability gates.
    pub fn import_name(self) -> &'static str {
        match self {
            Capability::Log => "log",
            Capability::TraceEmit => "trace_emit",
            Capability::TimerAfter => "timer_after",
            Capability::KvGet => "kv_get",
            Capability::KvSet => "kv_set",
        }
    }

    pub fn from_import(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.import_name() == name)
    }
}

impl fmt::Display for Capability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// A grant of one capability, optionally expiring at `expires_at_ms`
/// (sandbox clock, exclusive).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Grant {
    pub capability: Capability,
    pub expires_at_ms: Option<u64>,
}

impl Grant {
    pub fn new(capability: Capability) -> Self {
        Self { capability, expires_at_ms: None }
    }

    pub fn until(capability: Capability, expires_at_ms: u64) -> Self {
        Self { capability, expires_at_ms: Some(expires_at_ms) }
    }
}

/// Why a host call was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DenyReason {
    NotGranted,
    Expired,
    Revoked,
}

impl DenyReason {
    pub fn as_str(self) -> &'static str {
        match self {
            DenyReason::NotGranted => "not granted",
            DenyReason::Expired => "grant expired",
            DenyReason::Revoked => "grant revoked",
        }
    }
}

/// The set of grants a service holds. Default is empty: nothing is allowed.
#[derive(Clone, Debug, Default)]
pub struct Grants {
    grants: Vec<Grant>,
    revoked: Vec<Capability>,
}

impl Grants {
    pub fn none() -> Self {
        Self::default()
    }

    pub fn with(mut self, grant: Grant) -> Self {
        self.grant(grant);
        self
    }

    /// Convenience: non-expiring grants for each listed capability.
    pub fn of(caps: &[Capability]) -> Self {
        caps.iter().fold(Self::none(), |g, &c| g.with(Grant::new(c)))
    }

    /// Adds or replaces the grant for `grant.capability`; clears a revocation.
    pub fn grant(&mut self, grant: Grant) {
        self.grants.retain(|g| g.capability != grant.capability);
        self.revoked.retain(|&c| c != grant.capability);
        self.grants.push(grant);
    }

    /// Removes the grant; later calls are refused with [`DenyReason::Revoked`].
    pub fn revoke(&mut self, capability: Capability) -> bool {
        let before = self.grants.len();
        self.grants.retain(|g| g.capability != capability);
        let had = self.grants.len() != before;
        if had && !self.revoked.contains(&capability) {
            self.revoked.push(capability);
        }
        had
    }

    pub fn check(&self, capability: Capability, now_ms: u64) -> Result<(), DenyReason> {
        match self.grants.iter().find(|g| g.capability == capability) {
            Some(Grant { expires_at_ms: Some(t), .. }) if now_ms >= *t => Err(DenyReason::Expired),
            Some(_) => Ok(()),
            None if self.revoked.contains(&capability) => Err(DenyReason::Revoked),
            None => Err(DenyReason::NotGranted),
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = &Grant> {
        self.grants.iter()
    }
}
