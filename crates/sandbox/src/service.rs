//! Loading and running one service: compile, check imports against the ABI,
//! instantiate under limits, then run entry points with a fresh fuel budget.

use alloc::boxed::Box;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use wasmi::{CompilationMode, Config, Engine, ExternType, Instance, Linker, Module, Store, TrapCode, WasmParams};

use crate::caps::{Capability, Grant, Grants};
use crate::error::{HostTrap, SandboxError};
use crate::events::{EventKind, HostEvent, StopReason};
use crate::host::{self, HostState, IMPORT_MODULE};
use crate::kv::KvBackend;
use crate::limits::{Limiter, Limits};

/// What the broker decided a service may do.
#[derive(Clone, Debug)]
pub struct ServiceSpec {
    pub id: String,
    pub grants: Grants,
    pub limits: Limits,
}

impl ServiceSpec {
    pub fn new(id: impl Into<String>, grants: Grants) -> Self {
        Self { id: id.into(), grants, limits: Limits::default() }
    }

    pub fn with_limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallReport {
    pub export: String,
    pub fuel_used: u64,
}

/// Shared engine + linker; cheap to keep one per device.
pub struct Sandbox {
    engine: Engine,
    linker: Linker<HostState>,
}

impl Default for Sandbox {
    fn default() -> Self {
        Self::new()
    }
}

impl Sandbox {
    pub fn new() -> Self {
        let mut config = Config::default();
        config.consume_fuel(true);
        // Lazy translation bills compile work to the first call's fuel budget.
        config.compilation_mode(CompilationMode::Eager);
        let engine = Engine::new(&config);
        let linker = host::linker(&engine);
        Self { engine, linker }
    }

    pub fn load(
        &self,
        wasm: &[u8],
        spec: ServiceSpec,
        kv: impl KvBackend + 'static,
    ) -> Result<Service, SandboxError> {
        let module = Module::new(&self.engine, wasm).map_err(|e| SandboxError::Compile(e.to_string()))?;
        let requested = requested_capabilities(&module)?;
        if has_start_function(wasm) {
            return Err(SandboxError::StartFunction);
        }
        let state = HostState {
            service: spec.id,
            grants: spec.grants,
            limits: spec.limits,
            now_ms: 0,
            events: Vec::new(),
            timers: Vec::new(),
            kv: Box::new(kv),
            limiter: Limiter::new(&spec.limits),
        };
        let mut store = Store::new(&self.engine, state);
        store.limiter(|s| &mut s.limiter);
        store.set_fuel(spec.limits.fuel_per_call).expect("fuel enabled");
        let instance = match self.linker.instantiate_and_start(&mut store, &module) {
            Ok(instance) => instance,
            Err(e) => {
                return Err(match store.data().limiter.denials.first() {
                    Some(&denial) => SandboxError::ResourceCap(denial),
                    None => SandboxError::Instantiate(e.to_string()),
                })
            }
        };
        store.data_mut().record(EventKind::Loaded { requested: requested.clone() });
        Ok(Service { store, instance, requested, stopped: None })
    }
}

/// Every import must be an `op6` function from the ABI. Returns the
/// capabilities the module could ask for, so the UI can show them up front.
fn requested_capabilities(module: &Module) -> Result<Vec<Capability>, SandboxError> {
    let mut caps = Vec::new();
    for import in module.imports() {
        let cap = match import.ty() {
            ExternType::Func(_) if import.module() == IMPORT_MODULE => Capability::from_import(import.name()),
            _ => None,
        };
        let Some(cap) = cap else {
            return Err(SandboxError::UnknownImport {
                module: import.module().to_string(),
                name: import.name().to_string(),
            });
        };
        if !caps.contains(&cap) {
            caps.push(cap);
        }
    }
    caps.sort();
    Ok(caps)
}

/// Scans the section headers for a start section (id 8). Called only after
/// `Module::new` validated the bytes, so the framing is well-formed.
fn has_start_function(wasm: &[u8]) -> bool {
    let mut pos = 8;
    while pos < wasm.len() {
        let id = wasm[pos];
        pos += 1;
        let (size, used) = leb_u32(&wasm[pos..]);
        pos += used + size as usize;
        if id == 8 {
            return true;
        }
    }
    false
}

fn leb_u32(bytes: &[u8]) -> (u32, usize) {
    let mut value = 0u32;
    for (i, &b) in bytes.iter().take(5).enumerate() {
        value |= u32::from(b & 0x7f) << (7 * i);
        if b & 0x80 == 0 {
            return (value, i + 1);
        }
    }
    (value, bytes.len().min(5))
}

/// One loaded service instance.
pub struct Service {
    store: Store<HostState>,
    instance: Instance,
    requested: Vec<Capability>,
    stopped: Option<StopReason>,
}

impl Service {
    pub fn id(&self) -> &str {
        &self.store.data().service
    }

    pub fn requested_capabilities(&self) -> &[Capability] {
        &self.requested
    }

    pub fn grant(&mut self, grant: Grant) {
        self.store.data_mut().grants.grant(grant);
    }

    pub fn revoke(&mut self, capability: Capability) -> bool {
        self.store.data_mut().grants.revoke(capability)
    }

    pub fn grants(&self) -> &Grants {
        &self.store.data().grants
    }

    /// Advances the sandbox clock (ms). Time never goes backwards.
    pub fn set_now(&mut self, now_ms: u64) {
        let state = self.store.data_mut();
        state.now_ms = state.now_ms.max(now_ms);
    }

    pub fn now_ms(&self) -> u64 {
        self.store.data().now_ms
    }

    pub fn memory_bytes(&self) -> usize {
        self.instance
            .get_memory(&self.store, "memory")
            .map_or(0, |m| m.data(&self.store).len())
    }

    /// Value of an exported `i32` global, for inspection.
    pub fn global_i32(&self, name: &str) -> Option<i32> {
        self.instance.get_global(&self.store, name)?.get(&self.store).i32()
    }

    /// `(due_ms, token)` for each pending timer.
    pub fn pending_timers(&self) -> Vec<(u64, i32)> {
        self.store.data().timers.iter().map(|t| (t.due_ms, t.token)).collect()
    }

    pub fn events(&self) -> &[HostEvent] {
        &self.store.data().events
    }

    pub fn drain_events(&mut self) -> Vec<HostEvent> {
        core::mem::take(&mut self.store.data_mut().events)
    }

    pub fn stop_reason(&self) -> Option<&StopReason> {
        self.stopped.as_ref()
    }

    /// Runs an exported `() -> ()` entry point with a fresh fuel budget.
    pub fn invoke(&mut self, export: &str) -> Result<CallReport, SandboxError> {
        self.call(export, ())
    }

    /// Advances the clock to `now_ms` and calls `on_timer(token)` for every
    /// due timer, earliest first. Stops at the first failure.
    ///
    /// A due timer re-enters the guest, so it is grant-gated like every host
    /// use: with `timer.after` not live (not granted, expired, or revoked) no
    /// callback is delivered — the firing is refused, recorded, and stops the
    /// service before any guest code runs (S-1).
    pub fn fire_due_timers(&mut self, now_ms: u64) -> Result<Vec<CallReport>, SandboxError> {
        self.set_now(now_ms);
        let now = self.now_ms();
        // Only a due timer re-enters the guest, so only then is `timer.after`
        // checked; ticking a service with nothing due is a no-op (REVIEW-1 #2).
        let any_due = self.store.data().timers.iter().any(|t| t.due_ms <= now);
        if any_due && self.stopped.is_none() {
            if let Some(reason) = self.store.data().grants.check(Capability::TimerAfter, now).err()
            {
                let denied = SandboxError::Denied { capability: Capability::TimerAfter, reason };
                let stop = StopReason::Denied { capability: Capability::TimerAfter, reason };
                self.store.data_mut().record(EventKind::Denied {
                    capability: Capability::TimerAfter,
                    reason,
                });
                self.store.data_mut().record(EventKind::Stopped(stop.clone()));
                self.stopped = Some(stop);
                return Err(denied);
            }
        }
        let timers = &mut self.store.data_mut().timers;
        let mut due: Vec<(u64, i32)> =
            timers.iter().filter(|t| t.due_ms <= now).map(|t| (t.due_ms, t.token)).collect();
        timers.retain(|t| t.due_ms > now);
        due.sort_by_key(|&(due_ms, _)| due_ms);
        let mut reports = Vec::new();
        for (_, token) in due {
            self.store.data_mut().record(EventKind::TimerFired { token });
            reports.push(self.call("on_timer", token)?);
        }
        Ok(reports)
    }

    fn call<P: WasmParams>(&mut self, export: &str, params: P) -> Result<CallReport, SandboxError> {
        if self.stopped.is_some() {
            return Err(SandboxError::Stopped);
        }
        let func = self
            .instance
            .get_typed_func::<P, ()>(&self.store, export)
            .map_err(|_| SandboxError::MissingExport(export.to_string()))?;
        let budget = self.store.data().limits.fuel_per_call;
        self.store.set_fuel(budget).expect("fuel enabled");
        let result = func.call(&mut self.store, params);
        let fuel_used = budget - self.store.get_fuel().expect("fuel enabled");
        self.flush_denials();
        match result {
            Ok(()) => Ok(CallReport { export: export.to_string(), fuel_used }),
            Err(e) => {
                let (error, reason) = classify(&e, budget);
                self.store.data_mut().record(EventKind::Stopped(reason.clone()));
                self.stopped = Some(reason);
                Err(error)
            }
        }
    }

    fn flush_denials(&mut self) {
        let state = self.store.data_mut();
        for denial in core::mem::take(&mut state.limiter.denials) {
            state.record(EventKind::ResourceDenied(denial));
        }
    }
}

fn classify(e: &wasmi::Error, fuel_per_call: u64) -> (SandboxError, StopReason) {
    match e.downcast_ref::<HostTrap>() {
        Some(HostTrap::Denied { capability, reason }) => (
            SandboxError::Denied { capability: *capability, reason: *reason },
            StopReason::Denied { capability: *capability, reason: *reason },
        ),
        Some(HostTrap::BadCall(msg)) => {
            (SandboxError::BadHostCall(msg.clone()), StopReason::BadHostCall(msg.clone()))
        }
        None if e.as_trap_code() == Some(TrapCode::OutOfFuel) => {
            (SandboxError::OutOfFuel { fuel_per_call }, StopReason::OutOfFuel { fuel_per_call })
        }
        None => (SandboxError::Trap(e.to_string()), StopReason::Trap(e.to_string())),
    }
}
