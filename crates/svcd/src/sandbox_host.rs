//! `logic` actions on real Wasm: an [`ActionHost`] that runs each call in
//! `crates/sandbox` (wasmi). Feature `sandbox`.
//!
//! - One fresh instance per call (`ephemeral_component`: linear memory is
//!   freed when the call returns — SERVICE-IR.md §7 rule 6).
//! - The call's fuel budget is the IR's `wasm_fuel_per_invocation`.
//! - The sandbox ABI's entry points take no parameters, so the rendered
//!   `args_mapping` is placed in the service's KV namespace under `args` and
//!   the guest's result is read back from `output` (both need `kv.*` grants).
//! - Running out of fuel is reported as having used more than the budget, so
//!   svcd's budget check stops the service and says why.
//!
//! Dialogs and compensations go to the wrapped `inner` host.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use sandbox::{EventKind, Grants, KvBackend, Limits, MemKv, Sandbox, SandboxError, ServiceSpec};
use serde_json::{json, Value};

use crate::host::{ActionHost, ActionKind, ActionOutcome, ActionRequest, HostError};

#[derive(Clone)]
pub struct Component {
    pub wasm: Vec<u8>,
    pub grants: Grants,
    /// Fuel is overridden per call from the IR's power budget.
    pub limits: Limits,
}

pub struct SandboxHost<H: ActionHost> {
    sandbox: Sandbox,
    components: BTreeMap<String, Component>,
    pub inner: H,
}

impl<H: ActionHost> SandboxHost<H> {
    pub fn new(inner: H) -> Self {
        Self { sandbox: Sandbox::new(), components: BTreeMap::new(), inner }
    }

    /// Register the Wasm bytes for an IR `deps.wasm_components[].id`.
    pub fn component(mut self, id: &str, c: Component) -> Self {
        self.components.insert(id.to_string(), c);
        self
    }
}

impl<H: ActionHost> ActionHost for SandboxHost<H> {
    fn invoke(&mut self, req: &ActionRequest) -> Result<ActionOutcome, HostError> {
        let ActionKind::Logic { component_ref, function, args, fuel_budget, .. } = &req.kind else {
            return self.inner.invoke(req);
        };
        let c = self.components.get(component_ref).ok_or_else(|| HostError(format!("no Wasm registered for component `{component_ref}`")))?;
        let kv = Rc::new(RefCell::new(MemKv::new()));
        kv.borrow_mut().set(&req.service_id, "args", args.to_string().as_bytes());
        let limits = Limits { fuel_per_call: *fuel_budget, ..c.limits };
        let spec = ServiceSpec::new(req.service_id.clone(), c.grants.clone()).with_limits(limits);
        let mut svc = self.sandbox.load(&c.wasm, spec, kv.clone()).map_err(|e| HostError(format!("load `{component_ref}`: {e}")))?;
        svc.set_now(req.now_ms);
        let report = match svc.invoke(function) {
            Ok(r) => r,
            Err(SandboxError::OutOfFuel { fuel_per_call }) => {
                let output = json!({ "error": format!("out of fuel after {fuel_per_call}") });
                return Ok(ActionOutcome { output, fuel_used: fuel_per_call + 1, ..Default::default() });
            }
            Err(e) => return Err(HostError(format!("{component_ref}.{function}: {e}"))),
        };
        let trace: Vec<Value> = svc
            .drain_events()
            .into_iter()
            .filter_map(|e| match e.kind {
                EventKind::Trace { name, payload } => Some(json!({ "name": name, "payload": payload })),
                _ => None,
            })
            .collect();
        let result = kv.borrow().get(&req.service_id, "output").map(|b| {
            let text = String::from_utf8_lossy(&b).into_owned();
            serde_json::from_str(&text).unwrap_or(Value::String(text))
        });
        Ok(ActionOutcome { output: json!({ "result": result, "trace": trace }), fuel_used: report.fuel_used, ..Default::default() })
    }
}
