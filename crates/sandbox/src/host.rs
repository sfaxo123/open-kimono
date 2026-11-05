//! The `op6` import module: the only OS access a service has. Every function
//! checks its grant first (a refused call has no side effect), then charges
//! fuel, then touches guest memory.
//!
//! Contract violations (no grant, bad pointer, oversize data, invalid UTF-8)
//! trap and stop the service. Runtime conditions a correct service can meet
//! (key not found, quota full) are negative return codes.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use wasmi::{Caller, Error, Extern, Linker, Memory, TrapCode};

use crate::caps::{Capability, Grants};
use crate::error::HostTrap;
use crate::events::{EventKind, HostEvent, LogLevel};
use crate::kv::KvBackend;
use crate::limits::{Limiter, Limits};

pub const IMPORT_MODULE: &str = "op6";

/// Return codes visible to the guest.
pub const OK: i32 = 0;
pub const ERR_NOT_FOUND: i32 = -1;
pub const ERR_QUOTA: i32 = -2;

/// Fuel charged per host call, on top of one unit per byte moved.
pub const HOST_CALL_FUEL: u64 = 100;

pub(crate) struct Timer {
    pub due_ms: u64,
    pub token: i32,
}

pub(crate) struct HostState {
    pub service: String,
    pub grants: Grants,
    pub limits: Limits,
    pub now_ms: u64,
    pub events: Vec<HostEvent>,
    pub timers: Vec<Timer>,
    pub kv: Box<dyn KvBackend>,
    pub limiter: Limiter,
}

impl HostState {
    pub fn record(&mut self, kind: EventKind) {
        let event = HostEvent { service: self.service.clone(), at_ms: self.now_ms, kind };
        self.events.push(event);
    }
}

fn require(caller: &mut Caller<'_, HostState>, cap: Capability) -> Result<(), Error> {
    let state = caller.data_mut();
    match state.grants.check(cap, state.now_ms) {
        Ok(()) => Ok(()),
        Err(reason) => {
            state.record(EventKind::Denied { capability: cap, reason });
            Err(Error::host(HostTrap::Denied { capability: cap, reason }))
        }
    }
}

fn charge(caller: &mut Caller<'_, HostState>, bytes: usize) -> Result<(), Error> {
    let cost = HOST_CALL_FUEL.saturating_add(bytes as u64);
    let fuel = caller.get_fuel()?;
    if fuel < cost {
        caller.set_fuel(0)?;
        return Err(TrapCode::OutOfFuel.into());
    }
    caller.set_fuel(fuel - cost)
}

fn bad(msg: impl Into<String>) -> Error {
    Error::host(HostTrap::BadCall(msg.into()))
}

fn memory(caller: &Caller<'_, HostState>) -> Result<Memory, Error> {
    caller
        .get_export("memory")
        .and_then(Extern::into_memory)
        .ok_or_else(|| bad("service exports no `memory`"))
}

/// Validates a guest (ptr, len) against `max` *before* allocating.
fn checked_len(len: i32, max: usize, what: &str) -> Result<usize, Error> {
    let len = len as u32 as usize;
    if len > max {
        return Err(bad(alloc::format!("{what} is {len} bytes, limit {max}")));
    }
    Ok(len)
}

fn read(caller: &Caller<'_, HostState>, ptr: i32, len: usize) -> Result<Vec<u8>, Error> {
    let mut buf = vec![0u8; len];
    memory(caller)?
        .read(caller, ptr as u32 as usize, &mut buf)
        .map_err(|_| bad("pointer out of bounds"))?;
    Ok(buf)
}

fn read_str(caller: &Caller<'_, HostState>, ptr: i32, len: usize, what: &str) -> Result<String, Error> {
    String::from_utf8(read(caller, ptr, len)?).map_err(|_| bad(alloc::format!("{what} is not UTF-8")))
}

fn valid_trace_name(name: &str) -> bool {
    !name.is_empty()
        && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b))
}

fn log(mut caller: Caller<'_, HostState>, level: i32, ptr: i32, len: i32) -> Result<(), Error> {
    require(&mut caller, Capability::Log)?;
    let len = checked_len(len, caller.data().limits.max_message_bytes, "log message")?;
    charge(&mut caller, len)?;
    let message = read_str(&caller, ptr, len, "log message")?;
    caller.data_mut().record(EventKind::Log { level: LogLevel::from_i32(level), message });
    Ok(())
}

fn trace_emit(
    mut caller: Caller<'_, HostState>,
    name_ptr: i32,
    name_len: i32,
    payload_ptr: i32,
    payload_len: i32,
) -> Result<i32, Error> {
    require(&mut caller, Capability::TraceEmit)?;
    let limits = caller.data().limits;
    let name_len = checked_len(name_len, limits.max_trace_name_bytes, "trace name")?;
    let payload_len = checked_len(payload_len, limits.max_message_bytes, "trace payload")?;
    charge(&mut caller, name_len + payload_len)?;
    let name = read_str(&caller, name_ptr, name_len, "trace name")?;
    if !valid_trace_name(&name) {
        return Err(bad(alloc::format!("trace name `{name}` must be [a-z0-9._-]+")));
    }
    let payload = read_str(&caller, payload_ptr, payload_len, "trace payload")?;
    caller.data_mut().record(EventKind::Trace { name, payload });
    Ok(OK)
}

fn timer_after(mut caller: Caller<'_, HostState>, ms: i32, token: i32) -> Result<i32, Error> {
    require(&mut caller, Capability::TimerAfter)?;
    charge(&mut caller, 0)?;
    let state = caller.data_mut();
    let ms = ms as u32;
    if ms > state.limits.max_timer_ms {
        return Err(bad(alloc::format!("timer of {ms} ms exceeds {} ms", state.limits.max_timer_ms)));
    }
    if state.timers.len() >= state.limits.max_pending_timers {
        return Ok(ERR_QUOTA);
    }
    let due_ms = state.now_ms.saturating_add(u64::from(ms));
    state.timers.push(Timer { due_ms, token });
    state.record(EventKind::TimerScheduled { token, due_ms });
    Ok(OK)
}

fn kv_get(
    mut caller: Caller<'_, HostState>,
    key_ptr: i32,
    key_len: i32,
    out_ptr: i32,
    out_cap: i32,
) -> Result<i32, Error> {
    require(&mut caller, Capability::KvGet)?;
    let limits = caller.data().limits;
    let key_len = checked_len(key_len, limits.max_kv_key_bytes, "kv key")?;
    let out_cap = out_cap as u32 as usize;
    charge(&mut caller, key_len)?;
    let key = read_str(&caller, key_ptr, key_len, "kv key")?;
    let state = caller.data();
    let Some(value) = state.kv.get(&state.service, &key) else {
        return Ok(ERR_NOT_FOUND);
    };
    let n = value.len().min(out_cap);
    charge(&mut caller, n)?;
    memory(&caller)?
        .write(&mut caller, out_ptr as u32 as usize, &value[..n])
        .map_err(|_| bad("output pointer out of bounds"))?;
    Ok(value.len() as i32)
}

fn kv_set(
    mut caller: Caller<'_, HostState>,
    key_ptr: i32,
    key_len: i32,
    val_ptr: i32,
    val_len: i32,
) -> Result<i32, Error> {
    require(&mut caller, Capability::KvSet)?;
    let limits = caller.data().limits;
    let key_len = checked_len(key_len, limits.max_kv_key_bytes, "kv key")?;
    let val_len = checked_len(val_len, limits.max_kv_value_bytes, "kv value")?;
    if key_len == 0 {
        return Err(bad("kv key is empty"));
    }
    charge(&mut caller, key_len + val_len)?;
    let key = read_str(&caller, key_ptr, key_len, "kv key")?;
    let value = read(&caller, val_ptr, val_len)?;
    let state = caller.data_mut();
    let old = state.kv.get(&state.service, &key).map_or(0, |v| key.len() + v.len());
    let used = state.kv.used_bytes(&state.service) - old;
    if used + key.len() + value.len() > state.limits.kv_quota_bytes {
        return Ok(ERR_QUOTA);
    }
    let service = state.service.clone();
    state.kv.set(&service, &key, &value);
    state.record(EventKind::KvSet { key, value_bytes: value.len() });
    Ok(OK)
}

pub(crate) fn linker(engine: &wasmi::Engine) -> Linker<HostState> {
    let mut linker = Linker::new(engine);
    linker.func_wrap(IMPORT_MODULE, "log", log).expect("fresh linker");
    linker.func_wrap(IMPORT_MODULE, "trace_emit", trace_emit).expect("fresh linker");
    linker.func_wrap(IMPORT_MODULE, "timer_after", timer_after).expect("fresh linker");
    linker.func_wrap(IMPORT_MODULE, "kv_get", kv_get).expect("fresh linker");
    linker.func_wrap(IMPORT_MODULE, "kv_set", kv_set).expect("fresh linker");
    linker
}
