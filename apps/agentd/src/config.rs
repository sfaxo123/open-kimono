//! Runtime configuration, read from a file **outside the repo** (on the phone:
//! `~/agentic/agentd.toml`). No key ever lives in the repository: the config
//! names a *key file*, and the key is read at runtime.
//!
//! The format is a documented TOML subset — flat `key = value` lines, `#`
//! comments, strings in double quotes. Hand-parsing that subset keeps the
//! static musl build free of a TOML dependency; anything outside the subset is
//! a hard error naming the offending line, never a silent ignore.

use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;

/// One brain in the fallback chain: an OpenAI-compatible chat endpoint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Brain {
    /// Journal-visible identity, e.g. `homelab-gateway`.
    pub name: String,
    /// Full chat-completions URL, e.g. `http://MESH_HOST:4000/v1/chat/completions`.
    pub url: String,
    /// Model/alias to request (`auto`, `5090-nothink`, `gemma3-1b`).
    pub model: String,
    /// Whether this endpoint needs a bearer token from `key_file`.
    pub needs_key: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    /// Bind address. Safety rule: loopback or the phone's Meshnet IP only.
    pub listen: String,
    /// Ordered fallback chain. The phone runs gateway → on-device llama.cpp.
    pub brains: Vec<Brain>,
    /// File holding the gateway bearer token (single line). Never in the repo.
    pub key_file: PathBuf,
    /// Per-attempt wall-clock timeout for a model call.
    pub model_timeout_sec: u64,
    /// How many model round-trips one request may take before we stop.
    pub max_tool_rounds: usize,
    /// Idle timeout before the power manager may switch components off (A5).
    pub idle_timeout_sec: u64,
    /// Completion tokens one model call may use. A reasoning model spends these
    /// on its thinking before it can emit a tool call, so a small budget here
    /// looks like "the model returned nothing" on the phone.
    pub max_completion_tokens: u64,
    /// Optional directory of static files served at `/` (the kimono view).
    pub static_dir: Option<PathBuf>,
    /// Components the user pre-approved for power auto-switch advice
    /// (`power_preapproved = "wifi, bluetooth"`). Anything not listed is
    /// only ever proposed, never marked for switching.
    pub power_preapproved: Vec<String>,
    /// CV2 repair thresholds on the wire's `asr_confidence` (0..=1):
    /// at or above `repair_high` is HIGH (act as today), below
    /// `repair_low` is LOW (one targeted question, no brain, no
    /// tools), between is MEDIUM (proceed, restating the heard
    /// span). Defaults match the live proxy's two values — 0.8 is
    /// HIGH, 0.4 is LOW — with a band in between for any finer
    /// signal the wire later carries.
    pub repair_high: f32,
    pub repair_low: f32,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            listen: "127.0.0.1:8770".to_string(),
            brains: Vec::new(),
            key_file: PathBuf::from("agentic/litellm.key"),
            model_timeout_sec: 30,
            max_tool_rounds: 6,
            idle_timeout_sec: 120,
            max_completion_tokens: 2048,
            static_dir: None,
            power_preapproved: Vec::new(),
            repair_high: 0.7,
            repair_low: 0.55,
        }
    }
}

impl Config {
    /// The port of the on-device brain, if one is configured. The power manager
    /// uses it to tell whether the local model is up by reading
    /// `/proc/net/tcp` — no connection, no wake-up.
    pub fn local_model_port(&self) -> Option<u16> {
        let brain = self.brains.iter().find(|b| b.name == "on-device-llama")?;
        let after_scheme = brain.url.split("://").nth(1)?;
        let authority = after_scheme.split('/').next()?;
        authority.rsplit(':').next()?.parse::<u16>().ok()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigError {
    pub line: usize,
    pub reason: String,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "config line {}: {}", self.line, self.reason)
    }
}

impl std::error::Error for ConfigError {}

/// Parse the flat `key = value` subset.
fn parse_pairs(text: &str) -> Result<BTreeMap<String, (String, usize)>, ConfigError> {
    let mut pairs = BTreeMap::new();
    for (i, raw) in text.lines().enumerate() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            return Err(ConfigError { line: i + 1, reason: format!("expected `key = value`, got `{line}`") });
        };
        let key = key.trim().to_string();
        let value = value.trim().to_string();
        if key.is_empty() {
            return Err(ConfigError { line: i + 1, reason: "empty key".to_string() });
        }
        let value = unquote(&value).ok_or_else(|| ConfigError {
            line: i + 1,
            reason: format!("value for `{key}` must be a quoted string, a bare number or true/false"),
        })?;
        pairs.insert(key, (value, i + 1));
    }
    Ok(pairs)
}

fn unquote(v: &str) -> Option<String> {
    if v.is_empty() {
        return None;
    }
    if v.starts_with('"') && v.ends_with('"') && v.len() >= 2 {
        return Some(v[1..v.len() - 1].to_string());
    }
    // Bare tokens are allowed for numbers and booleans so paths/ports stay readable.
    if v.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_' | b'/')) {
        return Some(v.to_string());
    }
    None
}

fn get(pairs: &BTreeMap<String, (String, usize)>, key: &str) -> Option<(String, usize)> {
    pairs.get(key).cloned()
}

/// Load and validate a config. Unknown keys are an error: a typo in a phone
/// config must fail loudly, not quietly change the endpoint.
pub fn load(text: &str) -> Result<Config, ConfigError> {
    let pairs = parse_pairs(text)?;
    const KNOWN: &[&str] = &[
        "listen", "gateway_url", "gateway_model", "local_url", "local_model", "key_file",
        "model_timeout_sec", "max_tool_rounds", "idle_timeout_sec", "max_completion_tokens", "static_dir",
        "power_preapproved", "repair_high", "repair_low",
    ];
    if let Some(unknown) = pairs.keys().find(|k| !KNOWN.contains(&k.as_str())) {
        let (_, line) = pairs.get(unknown).unwrap().clone();
        return Err(ConfigError { line, reason: format!("unknown key `{unknown}`") });
    }

    let mut cfg = Config::default();
    if let Some((v, _)) = get(&pairs, "listen") {
        cfg.listen = v;
    }
    if let Some((v, _)) = get(&pairs, "key_file") {
        cfg.key_file = PathBuf::from(v);
    }
    for (key, field) in [
        ("model_timeout_sec", "model_timeout_sec"),
        ("max_tool_rounds", "max_tool_rounds"),
        ("idle_timeout_sec", "idle_timeout_sec"),
        ("max_completion_tokens", "max_completion_tokens"),
    ] {
        if let Some((v, line)) = get(&pairs, key) {
            let parsed = v.parse::<u64>().map_err(|_| ConfigError { line, reason: format!("`{key}` must be a non-negative integer, got `{v}`") })?;
            match field {
                "model_timeout_sec" => cfg.model_timeout_sec = parsed,
                "max_tool_rounds" => cfg.max_tool_rounds = parsed as usize,
                "idle_timeout_sec" => cfg.idle_timeout_sec = parsed,
                _ => cfg.max_completion_tokens = parsed,
            }
        }
    }
    if let Some((v, _)) = get(&pairs, "static_dir") {
        cfg.static_dir = Some(PathBuf::from(v));
    }
    for key in ["repair_high", "repair_low"] {
        if let Some((v, line)) = get(&pairs, key) {
            let parsed = v.parse::<f32>().map_err(|_| ConfigError {
                line,
                reason: format!("`{key}` must be a number in [0, 1], got `{v}`"),
            })?;
            if !(0.0..=1.0).contains(&parsed) {
                return Err(ConfigError { line, reason: format!("`{key}` must be a number in [0, 1], got `{v}`") });
            }
            match key {
                "repair_high" => cfg.repair_high = parsed,
                _ => cfg.repair_low = parsed,
            }
        }
    }
    if cfg.repair_low >= cfg.repair_high {
        return Err(ConfigError {
            line: 0,
            reason: format!(
                "`repair_low` ({}) must be below `repair_high` ({}) or there is no medium band",
                cfg.repair_low, cfg.repair_high
            ),
        });
    }
    if let Some((v, _)) = get(&pairs, "power_preapproved") {
        cfg.power_preapproved = v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
    }

    // The chain: gateway first, then the on-device brain if configured.
    let gw_url = get(&pairs, "gateway_url");
    let gw_model = get(&pairs, "gateway_model");
    if let (Some((url, _)), Some((model, _))) = (gw_url.clone(), gw_model.clone()) {
        cfg.brains.push(Brain { name: "homelab-gateway".into(), url, model, needs_key: true });
    } else if gw_url.is_some() || gw_model.is_some() {
        return Err(ConfigError { line: 0, reason: "`gateway_url` and `gateway_model` must both be set".to_string() });
    }
    let loc_url = get(&pairs, "local_url");
    let loc_model = get(&pairs, "local_model");
    if let (Some((url, _)), Some((model, _))) = (loc_url.clone(), loc_model.clone()) {
        cfg.brains.push(Brain { name: "on-device-llama".into(), url, model, needs_key: false });
    } else if loc_url.is_some() || loc_model.is_some() {
        return Err(ConfigError { line: 0, reason: "`local_url` and `local_model` must both be set".to_string() });
    }
    if cfg.brains.is_empty() {
        return Err(ConfigError { line: 0, reason: "no brain configured: set gateway_url/gateway_model or local_url/local_model".to_string() });
    }
    for brain in &cfg.brains {
        check_url(&brain.url).map_err(|r| ConfigError { line: 0, reason: format!("`{}`: {r}", brain.name) })?;
    }
    check_listen(&cfg.listen)?;
    Ok(cfg)
}

fn check_url(url: &str) -> Result<(), String> {
    let rest = url.strip_prefix("http://").ok_or("only plain http:// is supported (Meshnet, no TLS)")?;
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    if authority.is_empty() {
        return Err("URL has no host".to_string());
    }
    if !path.starts_with("v1/") {
        return Err(format!("URL path must start with `v1/` (chat completions), got `/{path}`"));
    }
    Ok(())
}

fn check_listen(listen: &str) -> Result<(), ConfigError> {
    let (host, port) = listen
        .rsplit_once(':')
        .ok_or_else(|| ConfigError { line: 0, reason: format!("`listen` must be `host:port`, got `{listen}`") })?;
    if host.is_empty() {
        return Err(ConfigError { line: 0, reason: "`listen` has no host".to_string() });
    }
    let port: u16 = port
        .parse()
        .map_err(|_| ConfigError { line: 0, reason: format!("`listen` port is not a number: `{port}`") })?;
    if port == 0 {
        return Err(ConfigError { line: 0, reason: "`listen` port must be non-zero".to_string() });
    }
    Ok(())
}

/// Read the bearer token. The file must exist and be a single line; the token
/// itself is never logged or journaled.
pub fn read_key(path: &std::path::Path) -> Result<String, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("key file {}: {e}", path.display()))?;
    let line = text.lines().map(str::trim).find(|l| !l.is_empty() && !l.starts_with('#'));
    match line {
        Some(l) => Ok(l.to_string()),
        None => Err(format!("key file {}: no key line found", path.display())),
    }
}
