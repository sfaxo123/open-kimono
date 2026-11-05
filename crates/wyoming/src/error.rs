//! Error types for Wyoming framing and event conversion.

use std::fmt;

/// Errors produced while decoding Wyoming frames or converting events into
/// typed events.
#[derive(Debug)]
pub enum Error {
    /// Underlying I/O error while reading or writing a stream.
    Io(std::io::Error),
    /// The header line was not valid JSON, or not a JSON object.
    InvalidHeader(String),
    /// The header line was a JSON object but had no string `"type"` field.
    MissingType,
    /// The additional-data block (after `data_length` bytes) was not valid
    /// JSON or not a JSON object.
    InvalidData(String),
    /// The stream ended before a full frame (line, data block or payload
    /// block) could be read.
    UnexpectedEof,
    /// A required field was missing from an event's `data` object.
    MissingField(String),
    /// A field in an event's `data` object had the wrong type or value.
    InvalidField(String),
    /// A client state-machine action was attempted in a state that does not
    /// allow it (e.g. `audio-chunk` before `audio-start`, or any action on a
    /// closed connection).
    InvalidState(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(err) => write!(f, "io error: {err}"),
            Error::InvalidHeader(msg) => write!(f, "invalid header line: {msg}"),
            Error::MissingType => write!(f, "header line has no string \"type\" field"),
            Error::InvalidData(msg) => write!(f, "invalid additional data block: {msg}"),
            Error::UnexpectedEof => write!(f, "stream ended mid-frame"),
            Error::MissingField(name) => write!(f, "missing required field: {name}"),
            Error::InvalidField(name) => write!(f, "invalid field: {name}"),
            Error::InvalidState(msg) => write!(f, "invalid state: {msg}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(err) => Some(err),
            _ => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        Error::Io(err)
    }
}

impl From<serde_json::Error> for Error {
    fn from(err: serde_json::Error) -> Self {
        Error::InvalidHeader(err.to_string())
    }
}

// `std::io::Error` is neither `Clone` nor `PartialEq`, so these are
// implemented by hand: cloning an `Io` variant preserves its error kind and
// message, and comparing two `Io` variants compares kind + message.
impl Clone for Error {
    fn clone(&self) -> Self {
        match self {
            Error::Io(err) => match err.raw_os_error() {
                Some(code) => Error::Io(std::io::Error::from_raw_os_error(code)),
                None => Error::Io(std::io::Error::new(err.kind(), err.to_string())),
            },
            Error::InvalidHeader(msg) => Error::InvalidHeader(msg.clone()),
            Error::MissingType => Error::MissingType,
            Error::InvalidData(msg) => Error::InvalidData(msg.clone()),
            Error::UnexpectedEof => Error::UnexpectedEof,
            Error::MissingField(name) => Error::MissingField(name.clone()),
            Error::InvalidField(name) => Error::InvalidField(name.clone()),
            Error::InvalidState(msg) => Error::InvalidState(msg.clone()),
        }
    }
}

impl PartialEq for Error {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Error::Io(a), Error::Io(b)) => a.kind() == b.kind() && a.to_string() == b.to_string(),
            (Error::InvalidHeader(a), Error::InvalidHeader(b)) => a == b,
            (Error::MissingType, Error::MissingType) => true,
            (Error::InvalidData(a), Error::InvalidData(b)) => a == b,
            (Error::UnexpectedEof, Error::UnexpectedEof) => true,
            (Error::MissingField(a), Error::MissingField(b)) => a == b,
            (Error::InvalidField(a), Error::InvalidField(b)) => a == b,
            (Error::InvalidState(a), Error::InvalidState(b)) => a == b,
            _ => false,
        }
    }
}
