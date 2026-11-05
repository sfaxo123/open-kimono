//! Interactions on the rendered scene become journal records.
//!
//! A tap on a button carries the A2UI client→server envelope verbatim
//! (`specification/v0_9/json/client_to_server.json`: `version` plus an
//! `action` with `name`, `surfaceId`, `sourceComponentId`, `timestamp`,
//! `context`) inside an AG-UI `CUSTOM` event. So a consumer that speaks AG-UI
//! sees a familiar envelope, and one that speaks A2UI can lift the spec's own
//! JSON straight out of `value`.
//!
//! Two names are **journal-only**, and say so in their payload
//! (`"journalOnly": true`): the spec sends nothing to the server for a
//! keystroke — a text field edits the local data model, and the server learns
//! the value when the next action's `context` resolves it — and a
//! `functionCall` button is executed by the client, not reported. Recording
//! both is what makes the phone's own behaviour auditable (VISION §5).

use serde_json::{json, Value};
use trace_events::{AgUiEvent, Event, EventRing, Payload, TraceId};

use crate::scene::{Action, ItemKind, Scene};

/// The A2UI protocol version this client speaks.
pub const VERSION: &str = "v0.9";

/// AG-UI `CUSTOM` names this client emits.
pub const ACTION: &str = "a2ui.action";
pub const INPUT: &str = "a2ui.input";
pub const FUNCTION: &str = "a2ui.function";
pub const ERROR: &str = "a2ui.error";

/// Producer identity stamped on every event (the trace-events `service`
/// field), so a journal reader knows which process produced a record —
/// the same contract voice-bridge (C5) fulfils.
pub const SERVICE_NAME: &str = "a2ui-render";

/// What a tap did, so the caller can react (focus a field, redraw, ignore).
#[derive(Clone, Debug, PartialEq)]
pub enum Tap {
    /// An action was sent to the agent.
    Action { component_id: String, name: String, context: Value },
    /// A client-side function call was recorded (not sent).
    Function { component_id: String, call: Value },
    /// A text field is now where the keyboard goes.
    Focus { component_id: String },
    /// A button the document gave no action: reported, never swallowed.
    Inert { component_id: String },
    /// Nothing interactive was under the finger.
    Miss,
}

/// Emits this surface's events into an [`EventRing`].
#[derive(Clone, Debug)]
pub struct Emitter {
    surface_id: String,
    trace_id: Option<TraceId>,
}

impl Emitter {
    pub fn new(surface_id: impl Into<String>) -> Self {
        Self { surface_id: surface_id.into(), trace_id: None }
    }

    /// Tag every event with a W3C trace id, so a render and the agent run
    /// that produced it correlate.
    pub fn trace(mut self, id: TraceId) -> Self {
        self.trace_id = Some(id);
        self
    }

    pub fn surface_id(&self) -> &str {
        &self.surface_id
    }

    /// Handle a press at (x, y): hit-test the scene and emit what it means.
    pub fn tap(&self, scene: &Scene, x: i32, y: i32, ts_us: u64, ring: &mut EventRing) -> Tap {
        let Some(item) = scene.hit_test(x, y) else {
            return Tap::Miss;
        };
        match &item.kind {
            ItemKind::Button { id, action, .. } => match action {
                Action::Event { name, context } => {
                    let envelope =
                        action_envelope(&self.surface_id, name, id, ts_us, context.clone());
                    self.send(ring, ts_us, ACTION, envelope);
                    Tap::Action { component_id: id.clone(), name: name.clone(), context: context.clone() }
                }
                Action::Function { call } => {
                    let value = json!({
                        "journalOnly": true,
                        "version": VERSION,
                        "surfaceId": self.surface_id,
                        "sourceComponentId": id,
                        "timestamp": iso8601(ts_us),
                        "functionCall": call,
                    });
                    self.send(ring, ts_us, FUNCTION, value);
                    Tap::Function { component_id: id.clone(), call: call.clone() }
                }
                Action::None => {
                    self.error(ring, ts_us, "ACTION_UNSPECIFIED", &format!("button `{id}` has no action"));
                    Tap::Inert { component_id: id.clone() }
                }
            },
            ItemKind::Field { id, .. } => Tap::Focus { component_id: id.clone() },
            // hit_test only returns buttons and fields; anything else is a miss.
            _ => Tap::Miss,
        }
    }

    /// Record what was typed into a field. The path is the model location the
    /// value was written to, so a reader can follow the data.
    pub fn typed(
        &self,
        scene: &Scene,
        component_id: &str,
        value: &str,
        ts_us: u64,
        ring: &mut EventRing,
    ) -> Option<u64> {
        let field = scene.items.iter().find_map(|it| match &it.kind {
            ItemKind::Field { id, path, .. } if id == component_id => Some((id, path)),
            _ => None,
        })?;
        let value = json!({
            "journalOnly": true,
            "version": VERSION,
            "surfaceId": self.surface_id,
            "sourceComponentId": field.0,
            "path": field.1,
            "value": value,
            "timestamp": iso8601(ts_us),
        });
        Some(self.send(ring, ts_us, INPUT, value))
    }

    /// A client-side error, in the spec's generic error envelope.
    pub fn error(&self, ring: &mut EventRing, ts_us: u64, code: &str, message: &str) -> u64 {
        self.send(ring, ts_us, ERROR, error_envelope(&self.surface_id, code, message))
    }

    fn send(&self, ring: &mut EventRing, ts_us: u64, name: &str, value: Value) -> u64 {
        ring.push_event(Event {
            seq: 0, // assigned by the journal
            ts_us,
            service: Some(SERVICE_NAME.to_string()),
            trace_id: self.trace_id,
            span_id: None,
            parent_span_id: None,
            payload: Payload::AgUi(AgUiEvent::Custom { name: name.to_string(), value }),
        })
    }
}

/// The spec's client→server action envelope.
pub fn action_envelope(surface_id: &str, name: &str, source: &str, ts_us: u64, context: Value) -> Value {
    json!({
        "version": VERSION,
        "action": {
            "name": name,
            "surfaceId": surface_id,
            "sourceComponentId": source,
            "timestamp": iso8601(ts_us),
            "context": context,
        },
    })
}

/// The spec's generic client→server error envelope.
pub fn error_envelope(surface_id: &str, code: &str, message: &str) -> Value {
    json!({"version": VERSION, "error": {"code": code, "surfaceId": surface_id, "message": message}})
}

/// Microseconds since the Unix epoch → ISO 8601 UTC with milliseconds, the
/// `date-time` format the A2UI timestamp field asks for.
pub fn iso8601(ts_us: u64) -> String {
    let secs = (ts_us / 1_000_000) as i64;
    let millis = ((ts_us % 1_000_000) / 1_000) as u32;
    let days = secs.div_euclid(86_400);
    let secs_of_day = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        secs_of_day / 3600,
        (secs_of_day % 3600) / 60,
        secs_of_day % 60
    )
}

/// Days since 1970-01-01 → proleptic Gregorian (Howard Hinnant's algorithm).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// The payload of the last event in the journal, if it is one of ours.
pub fn last_custom(ring: &EventRing) -> Option<(String, Value)> {
    ring.last().and_then(|e| match &e.payload {
        Payload::AgUi(AgUiEvent::Custom { name, value }) => Some((name.clone(), value.clone())),
        _ => None,
    })
}
