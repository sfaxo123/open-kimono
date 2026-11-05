//! Bridge error type. Every failure that reaches the caller is also
//! journaled (RunError + failed Handoff) — the bridge never fails silently.

use std::fmt;

#[derive(Debug)]
pub enum BridgeError {
    /// Socket / file IO failure.
    Io(std::io::Error),
    /// Wyoming framing failure (bad header, truncated frame).
    Wyoming(wyoming::Error),
    /// WAV input rejected (format, truncation).
    Wav(String),
    /// The Wyoming server sent an `error` event.
    Server { message: String, code: Option<String> },
    /// The server closed the stream at a point where more was required,
    /// or sent events out of protocol order.
    Protocol(String),
    /// Bad command line.
    Usage(String),
}

impl fmt::Display for BridgeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BridgeError::Io(e) => write!(f, "io: {e}"),
            BridgeError::Wyoming(e) => write!(f, "wyoming framing: {e}"),
            BridgeError::Wav(m) => write!(f, "wav: {m}"),
            BridgeError::Server { message, code } => match code {
                Some(c) => write!(f, "server error [{c}]: {message}"),
                None => write!(f, "server error: {message}"),
            },
            BridgeError::Protocol(m) => write!(f, "protocol: {m}"),
            BridgeError::Usage(m) => write!(f, "usage: {m}"),
        }
    }
}

impl std::error::Error for BridgeError {}

impl From<std::io::Error> for BridgeError {
    fn from(e: std::io::Error) -> Self {
        BridgeError::Io(e)
    }
}

impl From<wyoming::Error> for BridgeError {
    fn from(e: wyoming::Error) -> Self {
        BridgeError::Wyoming(e)
    }
}

impl From<String> for BridgeError {
    fn from(m: String) -> Self {
        BridgeError::Protocol(m)
    }
}
