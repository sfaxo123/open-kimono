//! C5 voice-bridge: the seam between the phone and the homelab's Wyoming
//! speech engines. Streams a 16 kHz mono WAV to a Wyoming STT server and
//! text to a Wyoming TTS server, emitting every step as trace-events JSONL
//! on stdout. See `stt.rs`/`tts.rs` for the journal mapping rationale.

pub mod cli;
pub mod error;
pub mod journal;
pub mod stt;
pub mod tts;
pub mod wav;

pub use error::BridgeError;
pub use journal::Journal;
pub use stt::{run_stt, SttOptions};
pub use tts::{run_tts, TtsOptions, TtsResult};
pub use wav::{chunk_rms_level, read_pcm, write_pcm, Pcm};
