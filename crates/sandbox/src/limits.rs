//! Resource limits. Defaults are deliberately small (VISION §8): a service
//! gets what a tiny routine needs and asks for more explicitly.

use wasmi::errors::{MemoryError, TableError};
use wasmi::ResourceLimiter;
use wasmi_core::LimiterError;

pub const WASM_PAGE_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Fuel available to each entry-point call (≈ one unit per instruction,
    /// plus host-call costs). Refilled at the start of every call.
    pub fuel_per_call: u64,
    /// Upper bound on linear memory, in bytes (rounded down to whole pages).
    pub max_memory_bytes: usize,
    /// Upper bound on table elements (function pointers).
    pub max_table_elements: usize,
    /// Longest `log` message / `trace_emit` payload accepted, in bytes.
    pub max_message_bytes: usize,
    /// Longest trace event name, in bytes.
    pub max_trace_name_bytes: usize,
    pub max_kv_key_bytes: usize,
    pub max_kv_value_bytes: usize,
    /// Total bytes (keys + values) the service may keep in its KV namespace.
    pub kv_quota_bytes: usize,
    pub max_pending_timers: usize,
    /// Longest delay `timer_after` accepts, in ms.
    pub max_timer_ms: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            fuel_per_call: 1_000_000,
            max_memory_bytes: 16 * WASM_PAGE_BYTES,
            max_table_elements: 1024,
            max_message_bytes: 1024,
            max_trace_name_bytes: 64,
            max_kv_key_bytes: 128,
            max_kv_value_bytes: 4096,
            kv_quota_bytes: 64 * 1024,
            max_pending_timers: 8,
            max_timer_ms: 24 * 60 * 60 * 1000,
        }
    }
}

/// A growth the limiter refused (kept so the host can say why).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceDenial {
    Memory { requested_bytes: usize, cap_bytes: usize },
    Table { requested_elements: usize, cap_elements: usize },
}

/// wasmi limiter: one instance, one memory, one table, all capped.
#[derive(Debug)]
pub(crate) struct Limiter {
    max_memory_bytes: usize,
    max_table_elements: usize,
    pub(crate) denials: alloc::vec::Vec<ResourceDenial>,
}

impl Limiter {
    pub(crate) fn new(limits: &Limits) -> Self {
        Self {
            max_memory_bytes: limits.max_memory_bytes,
            max_table_elements: limits.max_table_elements,
            denials: alloc::vec::Vec::new(),
        }
    }
}

impl ResourceLimiter for Limiter {
    fn memory_growing(
        &mut self,
        _current: usize,
        desired: usize,
        maximum: Option<usize>,
    ) -> Result<bool, LimiterError> {
        if desired > self.max_memory_bytes {
            self.denials.push(ResourceDenial::Memory {
                requested_bytes: desired,
                cap_bytes: self.max_memory_bytes,
            });
            return Ok(false);
        }
        Ok(maximum.is_none_or(|max| desired <= max))
    }

    fn table_growing(
        &mut self,
        _current: usize,
        desired: usize,
        maximum: Option<usize>,
    ) -> Result<bool, LimiterError> {
        if desired > self.max_table_elements {
            self.denials.push(ResourceDenial::Table {
                requested_elements: desired,
                cap_elements: self.max_table_elements,
            });
            return Ok(false);
        }
        Ok(maximum.is_none_or(|max| desired <= max))
    }

    fn memory_grow_failed(&mut self, _error: &MemoryError) -> Result<(), LimiterError> {
        Ok(())
    }

    fn table_grow_failed(&mut self, _error: &TableError) -> Result<(), LimiterError> {
        Ok(())
    }

    fn instances(&self) -> usize {
        1
    }

    fn tables(&self) -> usize {
        1
    }

    fn memories(&self) -> usize {
        1
    }
}
