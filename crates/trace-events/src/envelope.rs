//! The journal envelope every `trace:` scheme line carries.
//!
//! One flat record per event: a monotonic `seq` (assigned by [`crate::EventRing`]),
//! a wall-clock `ts_us` in microseconds since the Unix epoch, optional W3C-style
//! trace/span ids for correlation (OTel-shaped), and the [`Payload`] tag that
//! routes to the three vocabularies: `gen_ai` (OTel GenAI spans), `ag_ui`
//! (AG-UI protocol events) and `op6` (our mic/STT/power/service events).

use alloc::string::String;
use serde::de::{Deserializer, Visitor};
use serde::{Deserialize, Serialize, Serializer};

/// Version of this event schema. Bump on any breaking JSON change.
pub const SCHEMA_VERSION: u32 = 1;

/// W3C trace id (PLAN B1 / OTel-shaped correlation). Serialized as exactly 32
/// lowercase hex characters, like every other W3C/OTel tooling.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TraceId(pub u128);

impl TraceId {
    pub fn to_hex(self) -> String {
        alloc::format!("{:032x}", self.0)
    }

    /// Parse exactly 32 hex characters (no `0x` prefix).
    pub fn from_hex(s: &str) -> Option<Self> {
        if s.len() != 32 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let mut v: u128 = 0;
        for b in s.bytes() {
            v = (v << 4) | u128::from((b as char).to_digit(16)?);
        }
        Some(TraceId(v))
    }
}

/// OTel span id. Serialized as exactly 16 lowercase hex characters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SpanId(pub u64);

impl SpanId {
    pub fn to_hex(self) -> String {
        alloc::format!("{:016x}", self.0)
    }

    /// Parse exactly 16 hex characters (no `0x` prefix).
    pub fn from_hex(s: &str) -> Option<Self> {
        if s.len() != 16 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let mut v: u64 = 0;
        for b in s.bytes() {
            v = (v << 4) | u64::from((b as char).to_digit(16)?);
        }
        Some(SpanId(v))
    }
}

fn trace_id_from_hex<'de, E: serde::de::Error>(s: &str) -> Result<TraceId, E> {
    TraceId::from_hex(s).ok_or_else(|| E::invalid_value(serde::de::Unexpected::Str(s), &"32 hex chars"))
}

fn span_id_from_hex<'de, E: serde::de::Error>(s: &str) -> Result<SpanId, E> {
    SpanId::from_hex(s).ok_or_else(|| E::invalid_value(serde::de::Unexpected::Str(s), &"16 hex chars"))
}

impl Serialize for TraceId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for TraceId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct V;
        impl Visitor<'_> for V {
            type Value = TraceId;
            fn expecting(&self, f: &mut alloc::fmt::Formatter) -> alloc::fmt::Result {
                f.write_str("a 32-character hex trace id")
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<TraceId, E> {
                trace_id_from_hex(v)
            }
        }
        deserializer.deserialize_str(V)
    }
}

impl Serialize for SpanId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for SpanId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct V;
        impl Visitor<'_> for V {
            type Value = SpanId;
            fn expecting(&self, f: &mut alloc::fmt::Formatter) -> alloc::fmt::Result {
                f.write_str("a 16-character hex span id")
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<SpanId, E> {
                span_id_from_hex(v)
            }
        }
        deserializer.deserialize_str(V)
    }
}

/// One journal record. `seq` is assigned by the journal (ring buffer), not by
/// the producer, so consumers can tail with [`crate::EventRing::since`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Event {
    /// Monotonic journal sequence number (gapless while the journal lives).
    pub seq: u64,
    /// Wall clock, microseconds since the Unix epoch.
    pub ts_us: u64,
    /// Producer identity for service filtering and per-service rate limits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<TraceId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span_id: Option<SpanId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_span_id: Option<SpanId>,
    pub payload: Payload,
}

/// The vocabularies of the open-kimono journal (docs/PLAN.md B1): everything
/// the phone itself does (mic, STT, power, services) as op6; model calls as
/// OTel GenAI spans and agent↔UI protocol events as AG-UI are added on top.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Payload {
    GenAi(crate::genai::GenAiEvent),
    AgUi(crate::agui::AgUiEvent),
    Op6(crate::op6::Op6Event),
}
