//! Our own event vocabulary: what the phone itself does — mic, STT, power,
//! services, grants, hand-offs. Every event carries the *reason* where a
//! reason exists: the open kimono (VISION §3–§5) means the UI can show not
//! just that something happened but why it happened.

use alloc::string::String;
use serde::{Deserialize, Serialize};

/// Mic / STT / power / service / grant / hand-off events. Tagged by `type`
/// (snake_case) inside the journal envelope's `"kind":"op6"` payload.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Op6Event {
    /// Journal retention or a slow reader skipped records.
    JournalDropped { count: u64, reason: String },
    // — Mic: the user always sees whether the mic is live (VISION §3, B10) —
    /// Capture opened on a device.
    MicStarted {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        device: Option<String>,
    },
    /// Capture closed — `reason` is mandatory: closing is never silent.
    MicStopped {
        reason: String,
    },
    /// One capture frame's level meter (drives the waveform UI).
    MicFrame {
        level: f32,
    },

    // — STT: live transcript, then final, then the cleaned text (B1/B2) —
    SttPartial {
        engine: String,
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        confidence: Option<f32>,
    },
    SttFinal {
        engine: String,
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        confidence: Option<f32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        latency_us: Option<u64>,
    },
    /// Raw transcript → post-cleaned text (disambiguation, filler removal).
    TextCleaned {
        raw: String,
        cleaned: String,
    },

    // — Power: the system says what it switched and why (VISION §4, B10) —
    PowerChange {
        component: String,
        state: PowerState,
        reason: String,
    },
    /// A wake lock with an expiry the UI can show (expiring wake locks).
    WakeLockAcquired {
        holder: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        deadline_us: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_us: Option<u64>,
    },
    WakeLockReleased {
        holder: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },

    // — Services: Lego blocks starting/stopping, with generations (B8) —
    ServiceStarted {
        service: String,
        generation: u64,
        reason: String,
    },
    ServiceStopped {
        service: String,
        generation: u64,
        reason: String,
    },

    // — Capability grants: grants are journal events too (B1, B6) —
    GrantGranted {
        service: String,
        capability: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        scope: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expires_us: Option<u64>,
    },
    GrantRevoked {
        service: String,
        capability: String,
        reason: String,
    },

    // — Hand-offs: homelab/cloud work is announced and visible (VISION §5) —
    Handoff {
        destination: String,
        reason: String,
        status: HandoffStatus,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PowerState {
    On,
    Off,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HandoffStatus {
    Started,
    Completed,
    Failed,
}
