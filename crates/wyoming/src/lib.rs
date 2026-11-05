//! Rust port of the [Wyoming protocol](https://github.com/rhasspy/wyoming)
//! (MIT), the framed JSONL + binary-payload protocol used by Home Assistant
//! voice services (satellites, STT, TTS, wake word, intent handling).
//!
//! Adapted from the Python reference at commit 08a0c25 (wyoming 1.11.0).
//! See `docs/PROVENANCE.md` in the redox-oneplus6 repository.
//!
//! # Framing
//!
//! A frame is a single `\n`-terminated JSON header line, optionally followed
//! by a `data_length`-sized JSON object block (merged on top of the inline
//! `data`) and a `payload_length`-sized binary block:
//!
//! ```text
//! { "type": "...", "version": "...", "data_length": ..., "payload_length": ... }\n
//! <data_length bytes (optional)>
//! <payload_length bytes (optional)>
//! ```
//!
//! Use [`encode`]/[`decode`] for byte buffers and streams, and the typed
//! events in [`events`] for the protocol vocabulary.

pub mod client;
pub mod error;
pub mod event;
pub mod events;
pub mod transport;

pub use client::{Client, ClientState, Transition, TransitionKind};
pub use error::Error;
pub use event::{decode, encode, encode_into, Event, WYOMING_VERSION};
pub use transport::{connect, Framed, Sink};
