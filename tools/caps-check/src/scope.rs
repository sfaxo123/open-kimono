//! Scope grammar (design §5.1): how a *request* is compared against what a *grant* allows.
//!
//! Two rules from the design drive everything in this file.
//!
//! 1. **Monotonic narrowing.** A grant that allows `state.rw(area="nav")` also allows
//!    `state.rw(area="nav/waypoints")`: a sub-path of an allowed area is inside the area. A
//!    request for `state.rw(area="music/lyrics")` under the same grant is *not* covered.
//! 2. **Exact destinations only.** `network.connect` compares an exact `scheme://host:port`
//!    triple, and anything that is not exactly one of the grant's targets is denied. Prefix or
//!    suffix matching on a host is the hole that AWS-S3-style buckets made famous, so there is
//!    deliberately no wildcard support here: `Scope::Origin` is one literal host, port and
//!    scheme, and `parse_scope` refuses anything shaped like a pattern (`*`, `.internal`) or like
//!    prose (`"cloud"`). R1's "no `*`, and no prose scope" rule is enforced in *code* here, not
//!    only in prose, because a manifest written by a model is exactly where that mistake comes
//!    from.

use crate::clock::Nanos;
use alloc::string::String;
use alloc::vec::Vec;

/// What a request *wants to do*, in the units the budget metering needs. `Stream`/`Connect`
/// carry a quantity because "keep the mic open" and "keep the phone awake" are both measured in
/// on-time, not in calls: a grant that spends power always has a *budget* (R4), and a budget can
/// only be debited in the unit it was written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Want {
    /// Hold a stream open for `dur` nanoseconds (mic capture, a wake window, a surface).
    Stream { dur: Nanos },
    /// Move `bytes` out of the device (network).
    Connect { bytes: u64 },
    /// One non-power action with no unit (a notification, a clock read, a bus message).
    Use,
}

impl Want {
    /// What kind of budget this request would spend, if any.
    pub fn unit(&self) -> Unit {
        match self {
            Want::Stream { .. } => Unit::OnTime,
            Want::Connect { .. } => Unit::Bytes,
            Want::Use => Unit::None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    OnTime,
    Bytes,
    None,
}

/// The `scope` part of a grant: the narrowing arguments *inside* the brackets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    /// No narrowing arguments (`time.now`, `notify.post`), or a scope the model could not check.
    /// This is deliberately *not* a wildcard: see `is_exact`.
    Empty,
    /// A named per-service data area, plus its read-only flag (`state.rw`).
    Area { area: String, read_only: bool },
    /// A single device (`audio.capture(device="wcd9340.mic0")`).
    Device { device: String },
    /// One exact destination (`network.connect(target="…", scheme="https")`).
    Origin { host: String, port: u16, scheme: String },
    /// A place, as a fence or a mode (`sensor.gps(scope="fence=route")` / `("scope=coarse")`).
    Fence { fence: String },
    /// A wake budget: how far ahead of a deadline a timer may wake the phone, and how much
    /// on-time that may cost (`time.wake(advance="250ms", max="5m")`).
    Wake { advance: Nanos, max: Nanos },
    /// A surface a service may draw to (`display.render(surface="owned")`).
    Surface { surface: String },
    /// A bus topic (`bus.subscribe(topic="…")`).
    Topic { topic: String },
    /// Which key/gesture source (`input.read(source="surface")`).
    Keys { source: String },
    /// Which model, for `model.infer`.
    Model { model: String },
}

impl Scope {
    /// True when this scope names something a check can *compare* to. A scope that is neither
    /// `Empty` (nothing to compare) nor exact (`"*"`, `"cloud"`) is a manifest *error*: R1
    /// refuses wildcards, and an over-broad grant is not a grant.
    pub fn is_exact(&self) -> bool {
        !matches!(self, Scope::Empty)
    }

    /// Does *this* (the granted scope) cover `request`? `Scope::Empty` on the grant side is the
    /// widest form of that kind — which is why the design gates it by default (R12's "narrowest
    /// first" is about what we *ask* for) — and every other shape must match *exactly*.
    pub fn covers(&self, request: &Scope) -> bool {
        match (self, request) {
            // R1: no ambient authority, and no wildcard. An unscoped grant covers a scoped
            // request (it is the widest form of its kind), which is exactly why manifest
            // validation refuses to *mint* one for a kind that spends power or money.
            (Scope::Empty, _) => true,
            (
                Scope::Area {
                    area: grant_area,
                    read_only: grant_ro,
                },
                Scope::Area {
                    area: want_area,
                    read_only: want_ro,
                },
            ) => {
                // Sub-path narrowing (rule 1 above): a granted area covers an area *inside* it.
                let inside = valid_area(grant_area)
                    && valid_area(want_area)
                    && (want_area == grant_area
                        || want_area
                            .strip_prefix(grant_area.as_str())
                            .map(|rest| rest.starts_with('/'))
                            .unwrap_or(false));
                // `mode=ro` on the grant is a *ceiling* (R3), so a read-only grant never covers
                // a read-write request.
                inside && (!*grant_ro || *want_ro)
            }
            (Scope::Device { device: g }, Scope::Device { device: w }) => g == w,
            (
                Scope::Origin {
                    host: gh,
                    port: gp,
                    scheme: gs,
                },
                Scope::Origin {
                    host: wh,
                    port: wp,
                    scheme: ws,
                },
            ) => gh == wh && gp == wp && gs == ws,
            (Scope::Fence { fence: g }, Scope::Fence { fence: w }) => g == w,
            (
                Scope::Wake {
                    advance: ga,
                    max: gm,
                },
                Scope::Wake {
                    advance: wa,
                    max: wm,
                },
            ) => ga >= wa && gm >= wm,
            (Scope::Surface { surface: g }, Scope::Surface { surface: w }) => g == w,
            (Scope::Topic { topic: g }, Scope::Topic { topic: w }) => g == w,
            (Scope::Keys { source: g }, Scope::Keys { source: w }) => g == w,
            (Scope::Model { model: g }, Scope::Model { model: w }) => g == w,
            // A grant written for a different *shape* of scope never covers the request: a
            // device grant does not authorise a destination, and a fence does not authorise an
            // area. Shapes are not interchangeable, and neither is a device name with a host.
            _ => false,
        }
    }
}

/// Parse the text inside the brackets of a grant string (`"while-active"`, `"fence=route"`,
/// `"target=\"speech-cache.internal:443\", scheme=\"https\""`).
///
/// Anything the model cannot *check* at call time comes back as `Scope::Empty`, which manifest
/// validation then refuses (R1): a wildcard or a prose scope is not a scope, it is a hole with a
/// name in it.
pub fn parse_scope(text: &str) -> Scope {
    let text = text.trim();
    if text.is_empty() {
        return Scope::Empty;
    }

    // Split on commas that are *outside* brackets, then on `key=value`, trimming quotes.
    let mut parts: Vec<&str> = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    for (i, c) in text.char_indices() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            ',' if depth <= 0 => {
                parts.push(&text[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    if start < text.len() {
        parts.push(&text[start..]);
    }
    let parts: Vec<&str> = parts
        .iter()
        .map(|p| p.trim().trim_matches('"'))
        .filter(|p| !p.is_empty())
        .collect();

    // A scope with no `=` in it is a *window* or a mode, not a set of narrowing arguments:
    // `while-active` alone closes the grant when the activity stops; `fence:route` names a place.
    if parts.len() == 1 && !parts[0].contains('=') {
        let only = parts[0];
        return match only {
            "while-active" | "while-visible" => Scope::Empty,
            "coarse" => Scope::Fence {
                fence: String::from("coarse"),
            },
            "surface" => Scope::Surface {
                surface: String::from("surface"),
            },
            "keys" => Scope::Keys {
                source: String::from("keys"),
            },
            _ if only.starts_with("fence:") => Scope::Fence {
                fence: String::from(only.trim_start_matches("fence:").trim_start_matches('/')),
            },
            // `area:nav/waypoints`, `*`, `cloud`, `*.internal`: nothing a check can compare
            // against, so nothing that can be granted (R1).
            _ => Scope::Empty,
        };
    }

    let mut kv: Vec<(&str, &str)> = Vec::new();
    for part in parts.iter() {
        match part.split_once('=') {
            // A quoted value keeps its quotes in the source text (`target="host:443"`), so strip
            // them here: the *value* is what a check compares, not the punctuation around it.
            Some((k, v)) => kv.push((k.trim(), v.trim().trim_matches('"'))),
            None => kv.push((part, "")),
        }
    }
    let get = |key: &str| -> Option<&str> {
        kv.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
    };

    // `target=` is a destination, and only ever an *exact* one (rule 2 in the module header).
    if let Some(target) = get("target").or_else(|| get("host")) {
        if target.contains('*') || target.starts_with('.') {
            return Scope::Empty;
        }
        let (host, port) = match target.rsplit_once(':') {
            Some((h, p)) => (String::from(h), p.parse::<u16>().ok()),
            None => (String::from(target), None),
        };
        // A destination is a *triple*. A host with no port or no scheme is not checkable, and
        // "we'll assume 443 and https" is how a grant ends up covering plaintext HTTP.
        let (Some(port), Some(scheme)) = (port, get("scheme")) else {
            return Scope::Empty;
        };
        if scheme != "https" && scheme != "wss" {
            // Same ceiling as the `DestinationPolicy` donor: a plaintext hop to a permitted host
            // is still a leak, so a grant that would allow one was written wrong.
            return Scope::Empty;
        }
        return Scope::Origin {
            host,
            port,
            scheme: String::from(scheme),
        };
    }
    if let Some(area) = get("area") {
        if !valid_area(area) {
            return Scope::Empty;
        }
        // C-4 (audit 2026-10-25) / R9: an unknown `mode=` value is an error, not a shrug. Reading
        // the flag as `matches!(mode, Some("ro"))` made every plausible typo (`read-only`,
        // `readonly`, `RO`, `r`) land on read-*write* — the typo bought more authority than the
        // author asked for, and R3's ceiling then let that grant cover write requests. A value the
        // host cannot check yields `Empty`, which manifest validation refuses (R1).
        return match get("mode") {
            None | Some("rw") => Scope::Area {
                area: String::from(area),
                read_only: false,
            },
            Some("ro") => Scope::Area {
                area: String::from(area),
                read_only: true,
            },
            Some(_) => Scope::Empty,
        };
    }
    if let Some(device) = get("device") {
        return Scope::Device {
            device: String::from(device),
        };
    }
    if let Some(fence) = get("fence") {
        return Scope::Fence {
            fence: String::from(fence),
        };
    }
    if let (Some(advance), Some(max)) = (get("advance"), get("max")) {
        if let (Some(a), Some(m)) = (parse_dur(advance), parse_dur(max)) {
            return Scope::Wake {
                advance: a,
                max: m,
            };
        }
        return Scope::Empty; // a wake scope with unparseable durations cannot be metered
    }
    if let Some(model) = get("model") {
        return Scope::Model {
            model: String::from(model),
        };
    }
    if let Some(topic) = get("topic") {
        return Scope::Topic {
            topic: String::from(topic),
        };
    }
    if let Some(surface) = get("surface") {
        return Scope::Surface {
            surface: String::from(surface),
        };
    }
    if let Some(source) = get("source") {
        return Scope::Keys {
            source: String::from(source),
        };
    }
    Scope::Empty
}

fn valid_area(area: &str) -> bool {
    !area.is_empty()
        && !area.starts_with('/')
        && area.split('/').all(|segment| !segment.is_empty() && segment != "." && segment != "..")
}

fn parse_scaled(num: &str, multiplier: f64, max: f64) -> Option<f64> {
    let value = num.trim().parse::<f64>().ok()?;
    let scaled = value * multiplier;
    if !value.is_finite() || value < 0.0 || !scaled.is_finite() || scaled >= max {
        return None;
    }
    Some(scaled)
}

/// `250ms`, `5s`, `2m`, `90s` → nanoseconds. A number with **no unit** is `None`: a budget is
/// debited in the unit it is written in, so a bare `5000000000` cannot be told apart from a byte
/// count, and guessing wrong spends a wake budget on a byte transfer (or the other way round).
pub fn parse_dur(text: &str) -> Option<Nanos> {
    let t = text.trim();
    let (num, mult) = if let Some(v) = t.strip_suffix("ms") {
        (v, 1_000_000i64)
    } else if let Some(v) = t.strip_suffix('s') {
        (v, 1_000_000_000i64)
    } else if let Some(v) = t.strip_suffix('m') {
        (v, 60_000_000_000i64)
    } else if let Some(v) = t.strip_suffix('h') {
        (v, 3_600_000_000_000i64)
    } else {
        return None;
    };
    parse_scaled(num, mult as f64, i64::MAX as f64).map(|v| v as Nanos)
}

/// `4096B`, `512KiB`, `40MiB` → bytes. Same rule as `parse_dur`: the unit is written down, so a
/// volume cap is never mistaken for a duration.
pub fn parse_bytes(text: &str) -> Option<u64> {
    let t = text.trim();
    let (num, mult) = if let Some(v) = t.strip_suffix("KiB") {
        (v, 1_024u64)
    } else if let Some(v) = t.strip_suffix("MiB") {
        (v, 1_048_576u64)
    } else if let Some(v) = t.strip_suffix('B') {
        (v, 1u64)
    } else {
        return None;
    };
    parse_scaled(num, mult as f64, u64::MAX as f64).map(|v| v as u64)
}

/// `33%/1s` → `(33, 1_000_000_000)`: at most 33 % on-time per 1 s window.
pub fn parse_duty(text: &str) -> Option<(u32, Nanos)> {
    let (pct, window) = text.split_once('/')?;
    let pct = pct.trim().strip_suffix('%')?.trim().parse::<u32>().ok()?;
    let window = parse_dur(window.trim())?;
    if pct > 100 || window == 0 {
        return None;
    }
    Some((pct, window))
}
