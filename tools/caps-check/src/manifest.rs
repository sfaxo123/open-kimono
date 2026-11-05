//! Manifest parsing + validation (design §5.5, R9/R10): the machine-checkable part.
//!
//! A manifest is what a service *declares it needs*, and the whole point of making it
//! machine-checkable (Fuchsia's strongest transferable property) is that a model-written manifest
//! can be *refused before it runs*. The checks here are the ones that catch a real mistake
//! rather than a typo:
//!
//! * an undeclared capability **cannot be used** (R10) — that is what "no ambient authority"
//!   means in a system where nobody read the generated code;
//! * no wildcard scope and no prose scope (R1): `target="*"` and `target="cloud"` are both
//!   refused, because neither can be *compared* to anything at check time;
//! * a capability that spends power or money **must** carry a budget (R4);
//! * `kind = "remote"` (the service runs off-device, so it costs money *and* power) is refused by
//!   default, and the only exception has to name an exact host (VISION §5 keeps the off-device
//!   hand-off visible, R12 keeps the transport layer separate from the authority layer).

use crate::clock::Nanos;
use crate::grant::{Budget, Grant, GrantState, PowerReq};
use crate::grant::GrantId;
use crate::scope::{parse_bytes, parse_dur, parse_duty, parse_scope, Scope};
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// What the *platform* exposes and a service may therefore ask for. Anything outside this list is
/// a manifest error (R10): the catalogue is closed on purpose, because "everything, eventually" is
/// where a permission list goes to rot, and a model that invents `mic.selftest` or `shell.exec` is
/// describing a capability that would need a driver nobody has written.
pub const CATALOGUE: &[&str] = &[
    "audio.capture",
    "audio.play",
    "network.connect",
    "network.mdns",
    "power.request-wake",
    "sensor.gps",
    "sensor.imu",
    "display.render",
    "time.wake",
    "time.now",
    "time.alarm",
    "state.rw",
    "state.erase",
    "notify.post",
    "model.infer",
    "input.read",
    "bus.subscribe",
    "device.selftest",
    "log.write",
];

/// Kinds whose *use* spends power or money: these are the ones R4 says must never be unlimited,
/// because "unlimited" here means "the phone is warm in an hour and the bill is larger".
pub const SPENDS_POWER: &[&str] = &[
    "audio.capture",
    "audio.play",
    "network.connect",
    "network.mdns",
    "power.request-wake",
    "sensor.gps",
    "sensor.imu",
    "display.render",
    "time.wake",
    "time.alarm",
    "model.infer",
];

/// Kinds whose grant is *meaningless* without narrowing arguments: `state.rw` with no area is a
/// grant over all of a service's data, `audio.capture` with no device is "any microphone", and
/// `network.connect` with no destination is a tunnel. R1's "no `*`, and no prose scope" rule is
/// enforced through this list: a scope that cannot be *compared* at check time is not a scope.
pub const NEEDS_SCOPE: &[&str] = &[
    "audio.capture",
    "audio.play",
    "network.connect",
    "network.mdns",
    "power.request-wake",
    "sensor.gps",
    "sensor.imu",
    "display.render",
    "time.wake",
    "state.rw",
    "state.erase",
    "model.infer",
    "input.read",
    "bus.subscribe",
];

/// Kinds where one holder excludes another (R12's conflict check): a second live grant on the same
/// exclusive resource is refused instead of queued, because "both may be held at once" makes the
/// grant *not* an authority over that resource.
pub const EXCLUSIVE: &[&str] = &["audio.capture", "audio.play", "sensor.gps", "display.render", "input.read"];

/// One declared need, as written in a manifest (`uses` block).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UseDecl {
    pub service: String,
    pub cap: String,
    /// The scope as *written*, kept so a refusal can quote it (R5: the reason is part of the
    /// event, and "scope `cloud` names nothing checkable" is a reason).
    pub scope_text: String,
    pub scope: Scope,
    pub window: Option<String>,
    pub budget: Budget,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ServiceKind {
    /// Runs on the phone (Wasm). The default, because everything else is a bigger trust problem.
    #[default]
    Wasm,
    /// Runs on the phone as a *native* daemon: outside the sandbox, so no journal guarantee and no
    /// budget ceiling (R5/R12) — refused by default.
    Native,
    /// Runs off-device (homelab/cloud): costs money and battery, so it is never invisible.
    Remote,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DepDecl {
    pub from: String,
    pub to: String,
    pub cap: String,
    /// R1: a *required* route that cannot be resolved leaves the service **refusing to start**,
    /// rather than starting with a hole where the capability should have been.
    pub critical: bool,
    /// R7: when this provider stops or its grant dies, its dependents stop too.
    pub critical_chain: bool,
}

/// What a service declares it *needs* (the `iid.uses` shape D1's IR will carry).
#[derive(Debug, Clone, Default)]
pub struct Manifest {
    pub service: String,
    pub gen: u32,
    pub engine: String,
    pub kind: ServiceKind,
    pub cores: u32,
    pub ram_kb: u32,
    pub wake: Option<String>,
    pub on_time: Option<String>,
    pub relay: Option<String>,
    pub uses: Vec<UseDecl>,
    pub routes: Vec<DepDecl>,
    pub check_needs_broker: bool,
}

impl Manifest {
    pub fn new(service: &str) -> Self {
        Manifest {
            service: service.to_string(),
            ..Default::default()
        }
    }
    pub fn uses(&self, cap: &str) -> Option<&UseDecl> {
        self.uses.iter().find(|u| u.cap == cap)
    }
    /// R10: the manifest is the ceiling. A call for something not listed is refused *here*, before
    /// the grant model is even consulted.
    pub fn declares(&self, cap: &str) -> bool {
        self.uses.iter().any(|u| u.cap == cap)
    }
}

/// A tiny JSON reader, just enough for a manifest: objects, arrays, strings, numbers, `true`/
/// `false`/`null`. Hand-rolled because this model stays dependency-free (and offline-buildable)
/// and because a permissive parser silently tolerates exactly the typlevel mistakes R9 exists to
/// catch.
#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(pairs) => pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s.as_str()),
            _ => None,
        }
    }
    pub fn as_arr(&self) -> Option<&[Json]> {
        match self {
            Json::Arr(v) => Some(v.as_slice()),
            _ => None,
        }
    }
    pub fn as_num(&self) -> Option<f64> {
        match self {
            Json::Num(n) => Some(*n),
            _ => None,
        }
    }
}

/// Parse JSON, returning `Err(reason)` for anything malformed (R9: an unknown *shape* is an error,
/// never a shrug).
pub fn json(text: &str) -> Result<Json, String> {
    let bytes: Vec<char> = text.chars().collect();
    let mut i = 0usize;
    let out = value(&bytes, &mut i)?;
    skip_ws(&bytes, &mut i);
    if i != bytes.len() {
        return Err(alloc::format!("trailing characters at {}", i));
    }
    Ok(out)
}

fn skip_ws(bytes: &[char], i: &mut usize) {
    while *i < bytes.len() && bytes[*i].is_whitespace() {
        *i += 1;
    }
}

fn value(bytes: &[char], i: &mut usize) -> Result<Json, String> {
    skip_ws(bytes, i);
    if *i >= bytes.len() {
        return Err("unexpected end of input".into());
    }
    match bytes[*i] {
        '{' => {
            *i += 1;
            let mut pairs = Vec::new();
            loop {
                skip_ws(bytes, i);
                if *i >= bytes.len() {
                    return Err("unterminated object".into());
                }
                if bytes[*i] == '}' {
                    *i += 1;
                    break;
                }
                let key = string(bytes, i)?;
                skip_ws(bytes, i);
                if *i >= bytes.len() || bytes[*i] != ':' {
                    return Err(alloc::format!("expected `:` after key `{}`", key));
                }
                *i += 1;
                pairs.push((key, value(bytes, i)?));
                skip_ws(bytes, i);
                if *i < bytes.len() && bytes[*i] == ',' {
                    *i += 1;
                }
            }
            Ok(Json::Obj(pairs))
        }
        '[' => {
            *i += 1;
            let mut items = Vec::new();
            loop {
                skip_ws(bytes, i);
                if *i >= bytes.len() {
                    return Err("unterminated array".into());
                }
                if bytes[*i] == ']' {
                    *i += 1;
                    break;
                }
                items.push(value(bytes, i)?);
                skip_ws(bytes, i);
                if *i < bytes.len() && bytes[*i] == ',' {
                    *i += 1;
                }
            }
            Ok(Json::Arr(items))
        }
        '"' => Ok(Json::Str(string(bytes, i)?)),
        't' => {
            if bytes[*i..].starts_with(&['t', 'r', 'u', 'e'][..]) {
                *i += 4;
                Ok(Json::Bool(true))
            } else {
                Err(alloc::format!("bad literal at {}", i))
            }
        }
        'f' => {
            if bytes[*i..].starts_with(&['f', 'a', 'l', 's', 'e'][..]) {
                *i += 5;
                Ok(Json::Bool(false))
            } else {
                Err(alloc::format!("bad literal at {}", i))
            }
        }
        'n' => {
            if bytes[*i..].starts_with(&['n', 'u', 'l', 'l'][..]) {
                *i += 4;
                Ok(Json::Null)
            } else {
                Err(alloc::format!("bad literal at {}", i))
            }
        }
        c if c == '-' || c.is_ascii_digit() => {
            let start = *i;
            let mut j = *i + 1;
            while j < bytes.len() && (bytes[j].is_ascii_digit() || matches!(bytes[j], '.' | '-' | '+' | 'e' | 'E')) {
                j += 1;
            }
            let text: String = bytes[start..j].iter().collect();
            match text.parse::<f64>() {
                Ok(n) => {
                    *i = j;
                    Ok(Json::Num(n))
                }
                Err(_) => Err(alloc::format!("bad number `{}`", text)),
            }
        }
        c => Err(alloc::format!("unexpected character `{}` at {}", c, *i)),
    }
}

fn string(bytes: &[char], i: &mut usize) -> Result<String, String> {
    if *i >= bytes.len() || bytes[*i] != '"' {
        return Err(alloc::format!("expected a string at {}", *i));
    }
    *i += 1;
    let mut out = String::new();
    while *i < bytes.len() {
        let c = bytes[*i];
        *i += 1;
        if c == '"' {
            return Ok(out);
        }
        if c == '\\' {
            // An escape sequence contributes the *escaped* character: `\"` is a quote inside the
            // string, which is how a scope writes an exact destination.
            if *i < bytes.len() {
                let escaped = bytes[*i];
                *i += 1;
                out.push(escaped);
            }
            continue;
        }
        out.push(c);
    }
    Err("unterminated string".into())
}

/// The keys a manifest may use at the top level. Everything else is refused: R9's "unknown keys
/// are errors, not ignored" is not a style rule, it is the reason a manifest that *silently* drops
/// a typo'd key (like `power.wake` misspelled) cannot build and then quietly not work.
const KEYS: &[&str] = &[
    "service",
    "gen",
    "engine",
    "kind",
    "cores",
    "ram_kb",
    "wake",
    "on_time",
    "relay",
    "uses",
    "routes",
    "check_needs_broker",
];

/// Read a manifest document. `Err` means the *shape* is wrong (R9), which is a build error before
/// it is ever a runtime question.
pub fn manifest(text: &str) -> Result<Manifest, String> {
    let doc = json(text)?;
    if !matches!(doc, Json::Obj(_)) {
        return Err("a manifest must be a JSON object".into());
    }

    let mut m = Manifest::new(doc.get("service").and_then(Json::as_str).unwrap_or("?"));
    for (key, _) in match &doc {
        Json::Obj(pairs) => pairs.clone(),
        _ => Vec::new(),
    } {
        if !KEYS.contains(&key.as_str()) {
            return Err(alloc::format!("unknown manifest key `{}`", key));
        }
    }

    m.gen = doc.get("gen").and_then(Json::as_num).map(|n| n as u32).unwrap_or(1);
    m.engine = String::from(doc.get("engine").and_then(Json::as_str).unwrap_or("wasmi"));
    m.kind = match doc.get("kind").and_then(Json::as_str) {
        Some("native") => ServiceKind::Native,
        Some("remote") => ServiceKind::Remote,
        _ => ServiceKind::Wasm,
    };
    m.cores = doc.get("cores").and_then(Json::as_num).map(|n| n as u32).unwrap_or(1);
    m.ram_kb = doc.get("ram_kb").and_then(Json::as_num).map(|n| n as u32).unwrap_or(8192);
    m.wake = doc.get("wake").and_then(Json::as_str).map(String::from);
    m.on_time = doc.get("on_time").and_then(Json::as_str).map(String::from);
    m.relay = doc.get("relay").and_then(Json::as_str).map(String::from);
    m.check_needs_broker = matches!(doc.get("check_needs_broker"), Some(Json::Bool(true)));

    if let Some(Json::Arr(items)) = doc.get("uses") {
        for item in items {
            let service = item
                .get("service")
                .and_then(Json::as_str)
                .unwrap_or("self")
                .to_string();
            let Some(cap) = item.get("cap").and_then(Json::as_str) else {
                return Err("a `uses` entry needs a `cap`".into());
            };
            let scope_text = item
                .get("scope")
                .and_then(Json::as_str)
                .unwrap_or("")
                .to_string();
            let budget = match item.get("budget").and_then(Json::as_str) {
                Some(text) => {
                    if let Some((pct, window)) = parse_duty(text) {
                        let Some(total) = (pct as Nanos).checked_mul(window) else {
                            return Err(alloc::format!("`{}`: duty budget overflows its time unit", cap));
                        };
                        Budget::OnTime {
                            total: total / 100,
                            window: Some(window),
                            max_open: None,
                        }
                    } else if let Some(bytes) = parse_bytes(text) {
                        Budget::Bytes(bytes)
                    } else if let Some(dur) = parse_dur(text) {
                        Budget::OnTime {
                            total: dur,
                            window: None,
                            max_open: Some(dur),
                        }
                    } else {
                        // R1/R9: an unparseable budget is *not* silently unlimited. A unit-less
                        // number lands here too, and that is deliberate.
                        return Err(alloc::format!(
                            "`{}`: budget `{}` is not a duration (250ms/5s/2m/90s), a volume \
                             (4096B/512KiB/40MiB) or a duty cycle (33%/1s)",
                            cap, text
                        ));
                    }
                }
                None => Budget::Unlimited,
            };
            m.uses.push(UseDecl {
                service,
                cap: cap.to_string(),
                scope: parse_scope(&scope_text),
                scope_text,
                window: item.get("window").and_then(Json::as_str).map(String::from),
                budget,
            });
        }
    }

    if let Some(Json::Arr(items)) = doc.get("routes") {
        for item in items {
            let from = item
                .get("from")
                .and_then(Json::as_str)
                .unwrap_or("self")
                .to_string();
            let to = match item.get("to").and_then(Json::as_str) {
                Some(t) => String::from(t.strip_prefix('#').unwrap_or(t)),
                None => return Err("a `routes` entry needs a `to`".into()),
            };
            let cap = match item.get("cap").and_then(Json::as_str) {
                Some(c) => c.to_string(),
                None => return Err("a `routes` entry needs a `cap`".into()),
            };
            m.routes.push(DepDecl {
                from,
                to,
                cap,
                critical: matches!(item.get("critical"), Some(Json::Bool(true))),
                critical_chain: matches!(item.get("criticalChain"), Some(Json::Bool(true))),
            });
        }
    }
    Ok(m)
}

/// Every rule the model can check about a manifest. An empty result means "nothing here is
/// *structurally* wrong", not "this is safe": the scope still has to be narrow, and narrowness is
/// where R12 sends the arguments.
pub fn validate(m: &Manifest) -> Vec<String> {
    let mut errors = Vec::new();
    // C-2 (audit 2026-10-25): `mint` mints one grant per `uses` entry and `check` falls over to the
    // next affordable grant when the first is spent, so two entries with the same `(cap, scope)`
    // are a *doubled* budget written in plain manifest syntax — while `Manifest::uses()` returns
    // only the first, so the reviewer sees half the authority. R1's "one authority, one budget"
    // is only a rule if the validator counts declarations.
    let mut seen: Vec<(&str, &Scope)> = Vec::new();

    for u in &m.uses {
        if seen.iter().any(|(cap, scope)| *cap == u.cap && *scope == &u.scope) {
            errors.push(alloc::format!(
                "`{}` is declared twice with the same scope `{}`: one authority, one budget — a \
                 duplicate entry doubles the ceiling that `check` enforces, and `uses()` only ever \
                 shows the first, so the reviewer sees half of it",
                u.cap, u.scope_text
            ));
        } else {
            seen.push((u.cap.as_str(), &u.scope));
        }
        // R10: an undeclared capability cannot be used, and a *made-up* one is a manifest error.
        if !CATALOGUE.contains(&u.cap.as_str()) {
            errors.push(alloc::format!(
                "`{}` is not a capability in the catalogue: the catalogue is closed, so this is \
                 either a typo or an escalation attempt",
                u.cap
            ));
        }
        if let Budget::OnTime { total, window, max_open } = u.budget {
            if total < 0 || window.is_some_and(|value| value <= 0)
                || max_open.is_some_and(|value| value < 0)
            {
                errors.push(alloc::format!("`{}` has an invalid on-time budget", u.cap));
            }
        }
        // R4: an activity label is not a deadline. Rolling duty cycles need a finite lifetime;
        // otherwise only a correctly-unitized cumulative budget can bound total authority.
        if SPENDS_POWER.contains(&u.cap.as_str()) {
            let deadline = u.window.as_deref().and_then(parse_dur);
            let valid_activity = matches!(u.window.as_deref(), Some("while-active" | "while-visible"));
            let invalid_window = u.window.is_some() && deadline.is_none() && !valid_activity;
            if invalid_window {
                errors.push(alloc::format!(
                    "`{}` has an unsupported window; use a finite duration or a supported activity window",
                    u.cap
                ));
            }
            let unit_valid = match u.cap.as_str() {
                "network.connect" | "network.mdns" => matches!(u.budget, Budget::Bytes(_)),
                "model.infer" | "time.alarm" => matches!(u.budget, Budget::Uses(_))
                    || (deadline.is_some() && u.budget == Budget::Unlimited && u.cap == "time.alarm"),
                "audio.capture" | "audio.play" | "power.request-wake" | "sensor.gps"
                | "sensor.imu" | "display.render" | "time.wake" => {
                    matches!(u.budget, Budget::OnTime { .. })
                        || (deadline.is_some() && u.budget == Budget::Unlimited)
                }
                _ => true,
            };
            if !unit_valid {
                errors.push(alloc::format!("`{}` needs a budget in its operation's units", u.cap));
            }
            if deadline.is_none() && !u.budget.bounds_power_for(&u.cap) {
                errors.push(alloc::format!(
                    "`{}` spends power or money without an enforced finite deadline or a cumulative \
                     budget in its operation's units: R4 refuses unlimited authority",
                    u.cap
                ));
            }
            if let (Budget::OnTime { total, window: Some(window), .. }, Some(duration)) =
                (u.budget, deadline)
            {
                if total >= 0 && duration >= 0 && window > 0 {
                    let lifetime = (total as u128)
                        .checked_mul(duration as u128)
                        .map(|value| value / window as u128);
                    if lifetime.map_or(true, |value| value > Nanos::MAX as u128) {
                        errors.push(alloc::format!("`{}` duty budget lifetime overflows", u.cap));
                    }
                }
            }
        }
        // R1: a scope that cannot be *checked* cannot be *granted* either. This catches `*`, a
        // wildcard domain, and prose like "cloud" — the three shapes a model writes when it has
        // not thought about what the check would compare against.
        if NEEDS_SCOPE.contains(&u.cap.as_str()) && u.scope == Scope::Empty {
            errors.push(alloc::format!(
                "`{}`: scope `{}` names nothing the host can check, so it grants everything",
                u.cap, u.scope_text
            ));
        }
        // C-4 (audit 2026-10-25) / R9: name the actual mistake, not only its consequence. `mode=`
        // takes exactly `ro` or `rw`; anything else is a typo that used to read as read-write.
        if let Some(rest) = u.scope_text.split_once("mode=").map(|(_, v)| v) {
            let value = rest.trim_start_matches('"').split(['"', ',', ' ', ')']).next().unwrap_or("");
            if !matches!(value, "ro" | "rw") {
                errors.push(alloc::format!(
                    "`{}`: scope `{}` has `mode={}` — the only modes are `ro` and `rw`, and an \
                     unknown mode is refused rather than read as read-write (R9)",
                    u.cap, u.scope_text, value
                ));
            }
        }
        // R1: `*` is not a scope. A wildcard matches more than one thing, and a scope that
        // matches more than one thing cannot be *compared* to a request — which is the only thing
        // a check knows how to do.
        if u.scope_text.contains('*') {
            errors.push(alloc::format!(
                "`{}`: scope `{}` is a wildcard: R1 refuses wildcards, because a broadened grant \
                 is not a grant",
                u.cap, u.scope_text
            ));
        }
    }

    // R12, second clause: start-on-demand. A service that keeps a wake window permanently open
    // needs a budget on it, or the phone never sleeps: `wake = "keep"` with no `on_time` is
    // exactly "a cap that costs power, unlimited".
    if m.wake.as_deref() == Some("keep") && m.on_time.is_none() {
        errors.push(
            "wake = \"keep\" with no `on_time` budget: a permanent wake window is a dead battery, \
             not a feature (R4), and a service that runs forever is not one that runs when needed (R12)"
                .into(),
        );
    }

    // R1 + R12: an off-device service is never invisible, and it needs an exact destination.
    // `relay` alone is not authority: it needs a `network.connect` grant behind it too.
    if m.kind == ServiceKind::Remote {
        let has_relay = m.relay.is_some();
        let has_connect = m.uses.iter().any(|u| u.cap == "network.connect");
        if !has_relay || !has_connect {
            errors.push(
                "kind = \"remote\" needs an exact `relay` host *and* a `network.connect` grant: a \
                 cloud hand-off has to stay visible (VISION §5) and has to cost something (R12)"
                    .into(),
            );
        }
    }

    // R1/R10: no ambient authority, so a *route* to something undeclared is a hole with a name in
    // it, and a required route that cannot be resolved stops the service from starting at all.
    for r in &m.routes {
        if !CATALOGUE.contains(&r.cap.as_str()) {
            errors.push(alloc::format!(
                "route `{} -> {}` names `{}`, which is not a capability: a dangling `critical` \
                 route means the service must refuse to start (R1)",
                r.from, r.to, r.cap
            ));
        }
    }

    // R5/R12: a native daemon that needs the broker on the hot path has neither a journal
    // guarantee nor a budget ceiling, which is why the host answers from cached grants instead.
    if m.kind == ServiceKind::Native && m.check_needs_broker {
        errors.push(
            "a native service that needs the broker on the hot path has no journal guarantee and \
             no budget ceiling (R5/R12): put it behind a Wasm host function instead"
                .into(),
        );
    }
    if m.cores == 0 {
        errors.push("cores = 0: nothing could ever run, so this is not a budget (R12)".into());
    }
    if m.cores > 4 {
        errors.push("a budget of 8 CPU cores on a 4-core phone is not a budget (R12)".into());
    }
    errors
}

/// Mint the grants a manifest declares, into a table `check` can answer from. `id` is the next
/// free grant number (numbers are never reused, so the journal can keep pointing at them).
///
/// A manifest with errors mints **nothing**: R4's "a grant that spends power always has a budget"
/// is enforced here as well as in `validate`, because the alternative is a grant that spends power
/// with no ceiling and no journal entry explaining why it was allowed.
pub fn mint(m: &Manifest, now: Nanos, id: GrantId) -> Result<(GrantId, Vec<Grant>), Vec<String>> {
    let errors = validate(m);
    if !errors.is_empty() {
        return Err(errors);
    }
    let mut id = id;
    let mut out = Vec::new();
    for u in &m.uses {
        id += 1;
        let duration = parse_dur(u.window.as_deref().unwrap_or(""));
        let mut budget = u.budget;
        if budget == Budget::Unlimited {
            if let Some(duration) = duration {
                budget = match u.cap.as_str() {
                    "audio.capture" | "audio.play" | "power.request-wake" | "sensor.gps"
                    | "sensor.imu" | "display.render" | "time.wake" => Budget::OnTime {
                        total: duration,
                        window: None,
                        max_open: Some(duration),
                    },
                    "time.alarm" => Budget::Uses(1),
                    _ => budget,
                };
            }
        }
        let lifetime_budget = match (budget, duration) {
            (Budget::OnTime { total, window: Some(window), .. }, Some(duration)) => {
                Some(((total as u128) * (duration as u128) / (window as u128)).min(duration as u128) as Nanos)
            }
            (Budget::OnTime { total, window: None, max_open }, Some(duration)) => {
                let total = total.min(duration);
                let max_open = max_open.map(|value| value.min(duration));
                budget = Budget::OnTime { total, window: None, max_open };
                Some(total)
            }
            _ => None,
        };
        out.push(Grant {
            id,
            template: u.cap.clone(),
            kind: u.cap.clone(),
            scope: u.scope.clone(),
            service: m.service.clone(),
            instance: m.gen,
            state: GrantState::Live,
            exclusive: EXCLUSIVE.contains(&u.cap.as_str()),
            // R4: expiry measured in monotonic time, and only a *bounded* window mints a deadline.
            deadline: duration.map(|dur| now.saturating_add(dur)),
            lifetime_budget,
            lifetime_spent: 0,
            idle: None,
            last_use: None,
            budget,
            spent: 0,
            window_start: Some(now),
            in_flight: Vec::new(),
            // R11: the power effect is derived from the kind, never hand-written.
            power: PowerReq::for_kind(&u.cap),
            needs_revalidation: m.check_needs_broker,
            attract_id: 0,
        });
    }
    Ok((id, out))
}
