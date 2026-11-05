//! Taps and keystrokes become journal records — and the envelopes they carry
//! are the ones the A2UI spec defines, not something invented here.

use serde_json::{json, Value};
use trace_events::{from_jsonl, to_jsonl, EventRing, Payload, TraceId};

use a2ui_render::doc;
use a2ui_render::events::{self, Emitter, Tap};
use a2ui_render::scene::{self, Scene};
use a2ui_render::surface::Surface;

const W: i32 = 480;
const H: i32 = 800;
const TS: u64 = 1_761_290_130_123_456; // 2025-10-24T07:15:30.123Z

fn state(components: impl IntoIterator<Item = Value>, data: Value) -> Surface {
    let mut msgs = vec![json!({"version": "v0.9", "createSurface": {"surfaceId": "s1", "catalogId": "basic"}})];
    for c in components {
        msgs.push(json!({"version": "v0.9", "updateComponents": {"surfaceId": "s1", "components": [c]}}));
    }
    msgs.push(json!({"version": "v0.9", "updateDataModel": {"surfaceId": "s1", "value": data}}));
    let mut s = Surface::default();
    for m in &msgs {
        let parsed = doc::parse_message(m).unwrap_or_else(|e| panic!("message {m} rejected: {e}"));
        s.apply(&parsed).expect("apply");
    }
    s
}

fn layout(s: &Surface) -> Scene {
    scene::layout(s, W, H)
}

fn center(sc: &Scene, want: impl Fn(&a2ui_render::scene::ItemKind) -> bool) -> (i32, i32) {
    let it = sc.items.iter().find(|it| want(&it.kind)).expect("item");
    (it.rect.x + it.rect.w / 2, it.rect.y + it.rect.h / 2)
}

fn button(sc: &Scene) -> (i32, i32) {
    center(sc, |k| matches!(k, a2ui_render::scene::ItemKind::Button { .. }))
}

fn field(sc: &Scene) -> (i32, i32) {
    center(sc, |k| matches!(k, a2ui_render::scene::ItemKind::Field { .. }))
}

fn ring() -> EventRing {
    EventRing::new(64)
}

#[test]
fn iso8601_matches_the_utc_calendar() {
    // Vectors checked against Python's datetime.
    assert_eq!(events::iso8601(0), "1970-01-01T00:00:00.000Z");
    assert_eq!(events::iso8601(TS), "2025-10-24T07:15:30.123Z");
    assert_eq!(events::iso8601(1_709_208_000_000_000), "2024-02-29T12:00:00.000Z", "leap day");
    assert_eq!(events::iso8601(86_399_999_999), "1970-01-01T23:59:59.999Z", "last ms of the epoch day");
    assert_eq!(events::iso8601(1_761_345_600_500_000), "2025-10-24T22:40:00.500Z");
    // A date before the epoch must not panic either.
    assert_eq!(events::iso8601(1_000_000), "1970-01-01T00:00:01.000Z");
}

#[test]
fn a_tap_on_a_button_sends_the_spec_action_envelope() {
    let s = state(
        vec![json!({
            "id": "root", "component": "Button", "child": "l", "variant": "primary",
            "action": {"event": {"name": "deploy", "context": {"count": {"path": "/n"}, "room": "101"}}}
        }),
        json!({"id": "l", "component": "Text", "text": "Deploy"}),
        ],
        json!({"n": 7}),
    );
    let sc = layout(&s);
    let (x, y) = button(&sc);
    let mut r = ring();
    let tap = Emitter::new("s1").tap(&sc, x, y, TS, &mut r);
    assert_eq!(
        tap,
        Tap::Action { component_id: "root".into(), name: "deploy".into(), context: json!({"count": 7, "room": "101"}) }
    );

    let (name, value) = events::last_custom(&r).expect("an event");
    assert_eq!(name, events::ACTION);
    // Exactly the two keys the schema allows (minProperties/maxProperties = 2).
    let obj = value.as_object().expect("object");
    assert_eq!(obj.keys().collect::<Vec<_>>(), vec!["action", "version"]);
    assert_eq!(obj["version"], json!("v0.9"));
    let a = &obj["action"];
    assert_eq!(
        a.as_object().expect("action object").keys().collect::<Vec<_>>(),
        vec!["context", "name", "sourceComponentId", "surfaceId", "timestamp"]
    );
    assert_eq!(a["name"], json!("deploy"));
    assert_eq!(a["surfaceId"], json!("s1"));
    assert_eq!(a["sourceComponentId"], json!("root"));
    assert_eq!(a["timestamp"], json!("2025-10-24T07:15:30.123Z"));
    // Bindings are resolved before the wire, per the schema description.
    assert_eq!(a["context"], json!({"count": 7, "room": "101"}));
}

#[test]
fn a_tap_on_a_field_focuses_it_and_sends_nothing() {
    let s = state(
        vec![json!({"id": "root", "component": "TextField", "label": "Room", "value": {"path": "/room"}})],
        json!({"room": "101"}),
    );
    let sc = layout(&s);
    let (x, y) = field(&sc);
    let mut r = ring();
    let tap = Emitter::new("s1").tap(&sc, x, y, TS, &mut r);
    assert_eq!(tap, Tap::Focus { component_id: "root".into() });
    assert!(r.is_empty(), "focus is a local concern; the agent is not told");
}

#[test]
fn typing_is_recorded_with_the_model_path_it_wrote() {
    let mut s = state(
        vec![json!({"id": "root", "component": "TextField", "label": "Room", "value": {"path": "/room"}})],
        json!({"room": "101"}),
    );
    let path = s.set_field_value("root", "204").expect("field");
    assert_eq!(path, "/room");
    let sc = layout(&s);
    let mut r = ring();
    let seq = Emitter::new("s1").typed(&sc, "root", "204", TS, &mut r).expect("recorded");
    assert_eq!(seq, 0);
    let (name, value) = events::last_custom(&r).expect("an event");
    assert_eq!(name, events::INPUT);
    assert_eq!(value["journalOnly"], json!(true), "the spec has no keystroke message");
    assert_eq!(value["path"], json!("/room"));
    assert_eq!(value["value"], json!("204"));
    assert_eq!(value["sourceComponentId"], json!("root"));
    // A field that is not in the scene records nothing.
    assert!(Emitter::new("s1").typed(&sc, "nope", "x", TS, &mut r).is_none());
    assert_eq!(r.len(), 1, "and did not add an event");
}

#[test]
fn a_function_call_button_is_recorded_but_not_sent() {
    let s = state(
        vec![json!({
            "id": "root", "component": "Button", "child": "l",
            "action": {"functionCall": {"call": "openUrl", "args": {"url": "https://example.org"}, "returnType": "boolean"}}
        }),
        json!({"id": "l", "component": "Text", "text": "Open"}),
        ],
        Value::Null,
    );
    let sc = layout(&s);
    let (x, y) = button(&sc);
    let mut r = ring();
    let tap = Emitter::new("s1").tap(&sc, x, y, TS, &mut r);
    assert!(matches!(tap, Tap::Function { .. }), "{tap:?}");
    let (name, value) = events::last_custom(&r).expect("an event");
    assert_eq!(name, events::FUNCTION);
    assert!(value.get("action").is_none(), "a client-side call is not an agent action");
    assert_eq!(value["journalOnly"], json!(true));
    assert_eq!(value["functionCall"]["call"], json!("openUrl"));
    assert_eq!(value["functionCall"]["args"]["url"], json!("https://example.org"));
}

#[test]
fn a_button_with_no_action_reports_an_error_instead_of_silence() {
    let s = state(
        vec![json!({"id": "root", "component": "Button", "child": "l"}), json!({"id": "l", "component": "Text", "text": "Soon"})],
        Value::Null,
    );
    let sc = layout(&s);
    let (x, y) = button(&sc);
    let mut r = ring();
    let tap = Emitter::new("s1").tap(&sc, x, y, TS, &mut r);
    assert_eq!(tap, Tap::Inert { component_id: "root".into() });
    let (name, value) = events::last_custom(&r).expect("an event");
    assert_eq!(name, events::ERROR);
    assert_eq!(value["version"], json!("v0.9"));
    // The generic error envelope: code, surfaceId, message.
    assert_eq!(value["error"]["code"], json!("ACTION_UNSPECIFIED"));
    assert_eq!(value["error"]["surfaceId"], json!("s1"));
    assert!(value["error"]["message"].as_str().unwrap().contains("root"));
}

#[test]
fn a_press_on_blank_screen_records_nothing() {
    let s = state(vec![json!({"id": "root", "component": "Text", "text": "just words"})], Value::Null);
    let sc = layout(&s);
    let mut r = ring();
    let tap = Emitter::new("s1").tap(&sc, W - 2, H - 2, TS, &mut r);
    assert_eq!(tap, Tap::Miss);
    assert!(r.is_empty());
}

#[test]
fn events_carry_a_trace_id_and_survive_the_jsonl_round_trip() {
    let s = state(
        vec![json!({
            "id": "root", "component": "Button", "child": "l",
            "action": {"event": {"name": "go", "context": {}}}
        }),
        json!({"id": "l", "component": "Text", "text": "Go"}),
        ],
        Value::Null,
    );
    let sc = layout(&s);
    let (x, y) = button(&sc);
    let mut r = ring();
    let em = Emitter::new("s1").trace(TraceId(0x0123_4567_89ab_cdef_0123_4567_89ab_cdef));
    em.tap(&sc, x, y, TS, &mut r);
    em.error(&mut r, TS + 1_000_000, "ACTION_UNSPECIFIED", "second record");
    let seqs: Vec<u64> = r.iter().map(|e| e.seq).collect();
    assert_eq!(seqs, vec![0, 1], "the journal numbers gaplessly");
    assert!(r.iter().all(|e| e.trace_id == Some(TraceId(0x0123_4567_89ab_cdef_0123_4567_89ab_cdef))));

    let text = to_jsonl(&r.snapshot()).expect("serialise");
    let back = from_jsonl(&text).expect("deserialise");
    assert_eq!(back, r.snapshot(), "AG-UI CUSTOM payloads round-trip through the journal format");
    assert!(matches!(back[0].payload, Payload::AgUi(_)));
    // The wire form is what a consumer actually sees.
    let v: Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
    // The envelope is flat; the vocabulary tag lives inside `payload`.
    assert_eq!(v["payload"]["kind"], json!("ag_ui"));
    assert_eq!(v["payload"]["type"], json!("CUSTOM"));
    assert_eq!(v["payload"]["name"], json!("a2ui.action"));
    assert_eq!(v["payload"]["value"]["action"]["name"], json!("go"));
    assert_eq!(v["trace_id"], json!("0123456789abcdef0123456789abcdef"));
}

#[test]
fn every_event_stamps_the_producer_identity() {
    // The trace-events `service` field (added by B1 after C6 branched) must
    // reach the wire on every record — the same contract voice-bridge upholds.
    let s = state(
        vec![json!({
            "id": "root", "component": "Button", "child": "l",
            "action": {"event": {"name": "go", "context": {}}}
        }),
        json!({"id": "l", "component": "Text", "text": "Go"}),
        ],
        Value::Null,
    );
    let sc = layout(&s);
    let (x, y) = button(&sc);
    let mut r = ring();
    let em = Emitter::new("s1");
    em.tap(&sc, x, y, TS, &mut r);
    em.error(&mut r, TS + 1_000_000, "ACTION_UNSPECIFIED", "second record");
    assert!(r.iter().all(|e| e.service.as_deref() == Some(events::SERVICE_NAME)));

    let text = to_jsonl(&r.snapshot()).expect("serialise");
    assert!(text.lines().all(|l| l.contains(r#""service":"a2ui-render""#)), "{text}");
}
