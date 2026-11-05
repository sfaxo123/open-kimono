//! The named failure modes of voiced. Every variant carries enough to become
//! an honest journal record: the open kimono means the phone can always say
//! *what* failed and *where*, never just "error".

use std::fmt;

#[derive(Debug)]
pub enum VoiceError {
    /// The config file is unusable (names the key).
    Config(String),
    /// Another live process holds the microphone — voiced owns it, the browser never does.
    MicBusy { holder: String, lock: String },
    /// The user turned the mic off; voiced refuses capture until it is on.
    MicOff { state_file: String },
    /// Capture produced no usable audio.
    Capture(String),
    /// The STT engine could not be reached or refused the audio.
    SttUnavailable { engine: String, detail: String },
    /// The STT engine heard nothing: sending an empty request would be a lie.
    EmptyTranscript { engine: String },
    /// agentd is not answering.
    AgentUnreachable { endpoint: String, detail: String },
    /// agentd answered but refused the request.
    AgentRefused { endpoint: String, status: u16, detail: String },
    /// The run finished without an assistant message to speak.
    NoAnswer { endpoint: String, detail: String },
    /// TTS failed or produced a file that is not playable audio.
    TtsFailed { engine: String, detail: String },
    /// Playback was refused on purpose (quiet test, or no output device).
    PlaybackRefused { reason: String },
    /// Anything else I/O shaped, with the operation named.
    Io { op: String, detail: String },
}

impl fmt::Display for VoiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VoiceError::Config(m) => write!(f, "config: {m}"),
            VoiceError::MicBusy { holder, lock } => write!(f, "microphone busy: held by {holder} (lock {lock})"),
            VoiceError::MicOff { state_file } => write!(f, "mic off by user: capture refused — {state_file} says off; turn the mic on to use voice"),
            VoiceError::Capture(m) => write!(f, "capture: {m}"),
            VoiceError::SttUnavailable { engine, detail } => write!(f, "stt {engine}: {detail}"),
            VoiceError::EmptyTranscript { engine } => write!(f, "stt {engine}: transcript is empty (silence)"),
            VoiceError::AgentUnreachable { endpoint, detail } => write!(f, "agentd {endpoint}: unreachable: {detail}"),
            VoiceError::AgentRefused { endpoint, status, detail } => write!(f, "agentd {endpoint}: refused with HTTP {status}: {detail}"),
            VoiceError::NoAnswer { endpoint, detail } => write!(f, "agentd {endpoint}: no answer to speak: {detail}"),
            VoiceError::TtsFailed { engine, detail } => write!(f, "tts {engine}: {detail}"),
            VoiceError::PlaybackRefused { reason } => write!(f, "playback refused: {reason}"),
            VoiceError::Io { op, detail } => write!(f, "{op}: {detail}"),
        }
    }
}

impl From<std::io::Error> for VoiceError {
    fn from(e: std::io::Error) -> Self {
        VoiceError::Io { op: "io".into(), detail: e.to_string() }
    }
}
