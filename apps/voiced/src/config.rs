//! voiced configuration: one small `key = value` file, no TOML dependency.
//!
//! The engines are selected by name so the same binary runs against a
//! Wyoming server when one exists and against the engines actually installed
//! on the phone when it does not (PLAN A4). A bad value names the key it
//! choked on — a silent default here would hide a misconfigured phone.

use std::collections::BTreeMap;
use std::path::Path;

use crate::error::VoiceError;

#[derive(Clone, Debug, PartialEq)]
pub enum SttEngine {
    /// Wyoming STT at `host:port` (bridged by apps/voice-bridge).
    Wyoming { destination: String },
    /// A command that reads a 16 kHz mono WAV on stdin and prints the
    /// transcript on stdout — the thin adapter to what the phone installs.
    Cmd { cmd: String },
    /// A fixed transcript: for tests and for the operator's quiet trials.
    Stub { text: String },
}

#[derive(Clone, Debug, PartialEq)]
pub enum TtsEngine {
    Wyoming { destination: String },
    /// A command that reads the text on stdin and writes a WAV to argv[1].
    Cmd { cmd: String },
    /// Speak nothing (the answer is still journalled).
    None,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub stt: SttEngine,
    pub tts: TtsEngine,
    /// agentd base URL, e.g. `http://127.0.0.1:8787`.
    pub agentd: String,
    /// Command that plays a WAV given as argv[1]. `None` never plays.
    pub play_cmd: Option<String>,
    /// Quiet mode: synthesise, journal, and refuse to play (phone safety).
    pub quiet: bool,
    /// Mic ownership lock file.
    pub lock: String,
    /// The user's mic switch (`on` / `off`); capture refuses while off.
    pub mic_state_file: String,
    pub language: Option<String>,
    pub voice: Option<String>,
    /// Optional post-cleaner: reads the raw transcript on stdin, prints the
    /// cleaned text on stdout (journalled as `text_cleaned`).
    pub clean_cmd: Option<String>,
    /// Pre-rendered acknowledgment clips (CV0 instant ack). Empty means
    /// the ack is disabled and the turn behaves exactly as pre-CV0.
    pub ack_clips: Vec<String>,
    /// How long after the turn's STT final the ack may fire, in ms.
    pub ack_delay_ms: u64,
    /// Smart Turn judge command (CV2): a shell string in the STT/TTS
    /// `cmd` adapter shape — reads the capture-so-far WAV on stdin,
    /// prints `complete <prob>` / `incomplete <prob>`. `None` keeps
    /// the CV0 fixed-silence endpointing exactly as it was.
    pub smart_turn_cmd: Option<String>,
}

/// The ack deadline when a config names clips but no delay: ~600 ms
/// after STT final, per the CV0 brief.
pub const DEFAULT_ACK_DELAY_MS: u64 = 600;

impl Default for Config {
    fn default() -> Self {
        Config {
            stt: SttEngine::Stub { text: String::new() },
            tts: TtsEngine::None,
            agentd: "http://127.0.0.1:8787".into(),
            play_cmd: None,
            quiet: true,
            lock: "/tmp/voiced.mic.lock".into(),
            mic_state_file: "/tmp/voiced.mic.state".into(),
            language: None,
            voice: None,
            clean_cmd: None,
            ack_clips: Vec::new(),
            ack_delay_ms: DEFAULT_ACK_DELAY_MS,
            smart_turn_cmd: None,
        }
    }
}

fn keys(text: &str) -> Result<BTreeMap<String, String>, VoiceError> {
    let mut out = BTreeMap::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            return Err(VoiceError::Config(format!("line {}: expected key = value, got {line:?}", n + 1)));
        };
        out.insert(k.trim().to_string(), v.trim().to_string());
    }
    Ok(out)
}

/// `host:port` for a Wyoming engine: a host, a numeric port, nothing else.
fn valid_destination(s: &str) -> bool {
    match s.rsplit_once(':') {
        Some((host, port)) => !host.is_empty() && !host.contains('/') && !host.contains(' ') && !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()),
        None => false,
    }
}

/// `stt = wyoming host:port` | `stt = cmd <command>` | `stt = stub <text>`
fn parse_stt(value: &str) -> Result<SttEngine, VoiceError> {
    let (kind, rest) = match value.split_once(' ') {
        Some(p) => p,
        None => (value, ""),
    };
    match kind {
        "wyoming" => {
            let dest = rest.trim();
            if !valid_destination(dest) {
                return Err(VoiceError::Config(format!("stt wyoming needs host:port, got {dest:?}")));
            }
            Ok(SttEngine::Wyoming { destination: dest.into() })
        }
        "cmd" if !rest.trim().is_empty() => Ok(SttEngine::Cmd { cmd: rest.trim().into() }),
        "stub" => Ok(SttEngine::Stub { text: rest.to_string() }),
        other => Err(VoiceError::Config(format!("stt must be 'wyoming host:port', 'cmd <command>' or 'stub <text>', got {other:?}"))),
    }
}

fn parse_tts(value: &str) -> Result<TtsEngine, VoiceError> {
    let (kind, rest) = match value.split_once(' ') {
        Some(p) => p,
        None => (value, ""),
    };
    match kind {
        "wyoming" => {
            let dest = rest.trim();
            if !valid_destination(dest) {
                return Err(VoiceError::Config(format!("tts wyoming needs host:port, got {dest:?}")));
            }
            Ok(TtsEngine::Wyoming { destination: dest.into() })
        }
        "cmd" if !rest.trim().is_empty() => Ok(TtsEngine::Cmd { cmd: rest.trim().into() }),
        "none" => Ok(TtsEngine::None),
        other => Err(VoiceError::Config(format!("tts must be 'wyoming host:port', 'cmd <command>' or 'none', got {other:?}"))),
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Config, VoiceError> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| VoiceError::Config(format!("cannot read {}: {e}", path.display())))?;
        Config::parse(&text)
    }

    pub fn parse(text: &str) -> Result<Config, VoiceError> {
        let map = keys(text)?;
        let mut cfg = Config::default();
        for (k, v) in &map {
            match k.as_str() {
                "stt" => cfg.stt = parse_stt(v)?,
                "tts" => cfg.tts = parse_tts(v)?,
                "agentd" => {
                    if !v.starts_with("http://") {
                        return Err(VoiceError::Config(format!("agentd must be an http:// URL, got {v:?}")));
                    }
                    cfg.agentd = v.trim_end_matches('/').into();
                }
                "play" => cfg.play_cmd = if v == "none" { None } else { Some(v.clone()) },
                "quiet" => cfg.quiet = v == "true",
                "lock" => cfg.lock = v.clone(),
                "mic_state" => cfg.mic_state_file = v.clone(),
                "language" => cfg.language = Some(v.clone()),
                "voice" => cfg.voice = Some(v.clone()),
                "clean" => cfg.clean_cmd = if v == "none" { None } else { Some(v.clone()) },
                "ack_clips" => cfg.ack_clips = v.split_whitespace().map(|s| s.to_string()).collect(),
                "ack_delay_ms" => {
                    cfg.ack_delay_ms = v.parse().map_err(|_| {
                        VoiceError::Config(format!("ack_delay_ms must be a number of milliseconds, got {v:?}"))
                    })?
                }
                "smart_turn" => {
                    let (kind, rest) = match v.split_once(' ') {
                        Some(p) => p,
                        None => (v.as_str(), ""),
                    };
                    match kind {
                        "cmd" if !rest.trim().is_empty() => cfg.smart_turn_cmd = Some(rest.trim().into()),
                        "none" => cfg.smart_turn_cmd = None,
                        other => {
                            return Err(VoiceError::Config(format!(
                                "smart_turn must be 'cmd <command>' or 'none', got {other:?}"
                            )))
                        }
                    }
                }
                other => return Err(VoiceError::Config(format!("unknown key {other:?}"))),
            }
        }
        Ok(cfg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ack_keys_parse() {
        let cfg = Config::parse("ack_clips = /a/got.wav /a/one.wav\nack_delay_ms = 450\n").unwrap();
        assert_eq!(cfg.ack_clips, vec!["/a/got.wav".to_string(), "/a/one.wav".to_string()]);
        assert_eq!(cfg.ack_delay_ms, 450);
    }

    #[test]
    fn ack_defaults_without_keys() {
        let cfg = Config::parse("quiet = true\n").unwrap();
        assert!(cfg.ack_clips.is_empty());
        assert_eq!(cfg.ack_delay_ms, DEFAULT_ACK_DELAY_MS);
    }

    #[test]
    fn smart_turn_key_parses_and_defaults_off() {
        let cfg = Config::parse("smart_turn = cmd /x/judge.sh\n").unwrap();
        assert_eq!(cfg.smart_turn_cmd.as_deref(), Some("/x/judge.sh"));
        let cfg = Config::parse("quiet = true\n").unwrap();
        assert_eq!(cfg.smart_turn_cmd, None);
        let err = Config::parse("smart_turn = magic\n").unwrap_err();
        assert!(err.to_string().contains("smart_turn"), "{err}");
    }

    #[test]
    fn ack_delay_names_its_key_on_error() {
        let err = Config::parse("ack_delay_ms = soon\n").unwrap_err();
        assert!(err.to_string().contains("ack_delay_ms"), "{err}");
    }
}
