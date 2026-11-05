//! Audio input/output events (upstream `wyoming/audio.py`).

use serde_json::{json, Map, Value};

use crate::error::Error;
use crate::event::Event;
use crate::events::{from_data, to_map, Eventable};

/// Audio format fields shared by audio events (upstream `AudioFormat`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AudioFormat {
    /// Sample rate in hertz.
    pub rate: u32,
    /// Sample width in bytes.
    pub width: u8,
    /// Number of channels (mono = 1).
    pub channels: u8,
}

/// Chunk of raw PCM audio (`audio-chunk`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioChunk {
    /// Sample rate in hertz.
    pub rate: u32,
    /// Sample width in bytes.
    pub width: u8,
    /// Number of channels.
    pub channels: u8,
    /// Raw audio bytes (carried in the frame payload).
    pub audio: Vec<u8>,
    /// Milliseconds (optional).
    pub timestamp: Option<u64>,
}

impl Eventable for AudioChunk {
    const TYPE: &'static str = "audio-chunk";

    fn to_event(&self) -> Event {
        let mut data = Map::new();
        data.insert("rate".to_string(), json!(self.rate));
        data.insert("width".to_string(), json!(self.width));
        data.insert("channels".to_string(), json!(self.channels));
        data.insert(
            "timestamp".to_string(),
            self.timestamp.map(Value::from).unwrap_or(Value::Null),
        );
        Event {
            type_: Self::TYPE.to_string(),
            data,
            payload: Some(self.audio.clone()),
        }
    }

    fn from_event(event: &Event) -> Result<Self, Error> {
        if event.type_ != Self::TYPE {
            return Err(Error::InvalidField(format!(
                "expected event type \"{}\", got \"{}\"",
                Self::TYPE,
                event.type_
            )));
        }
        Ok(AudioChunk {
            rate: super::req_u64(&event.data, "rate")? as u32,
            width: super::req_u64(&event.data, "width")? as u8,
            channels: super::req_u64(&event.data, "channels")? as u8,
            audio: event.payload.clone().unwrap_or_default(),
            timestamp: super::opt_u64(&event.data, "timestamp"),
        })
    }
}

impl AudioChunk {
    /// Number of samples in the chunk.
    pub fn samples(&self) -> usize {
        self.audio.len() / (self.width as usize * self.channels as usize)
    }

    /// Duration in seconds.
    pub fn seconds(&self) -> f64 {
        self.samples() as f64 / self.rate as f64
    }

    /// Duration in milliseconds.
    pub fn milliseconds(&self) -> u64 {
        (self.seconds() * 1000.0) as u64
    }
}

/// Audio stream has started (`audio-start`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AudioStart {
    /// Sample rate in hertz.
    pub rate: u32,
    /// Sample width in bytes.
    pub width: u8,
    /// Number of channels.
    pub channels: u8,
    /// Milliseconds (optional).
    pub timestamp: Option<u64>,
}

impl Eventable for AudioStart {
    const TYPE: &'static str = "audio-start";

    fn to_event(&self) -> Event {
        Event {
            type_: Self::TYPE.to_string(),
            data: to_map(self),
            payload: None,
        }
    }

    fn from_event(event: &Event) -> Result<Self, Error> {
        from_data(event, Self::TYPE)
    }
}

/// Audio stream has stopped (`audio-stop`).
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct AudioStop {
    /// Milliseconds (optional).
    pub timestamp: Option<u64>,
}

impl Eventable for AudioStop {
    const TYPE: &'static str = "audio-stop";

    fn to_event(&self) -> Event {
        Event {
            type_: Self::TYPE.to_string(),
            data: to_map(self),
            payload: None,
        }
    }

    fn from_event(event: &Event) -> Result<Self, Error> {
        from_data(event, Self::TYPE)
    }
}
