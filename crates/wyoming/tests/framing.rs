//! Framing tests ported from the Python reference's `tests/test_event.py`
//! (MIT, commit 08a0c25) plus edge cases.

use std::io::Cursor;

use serde_json::{json, Map, Value};
use wyoming::{decode, encode, Event};

const PAYLOAD: &[u8] = b"test\npayload";
const VERSION: &str = wyoming::WYOMING_VERSION;

fn data_map() -> Map<String, Value> {
    let mut data = Map::new();
    data.insert("test".to_string(), json!("data"));
    data
}

fn data_bytes() -> Vec<u8> {
    // Exactly what Python's json.dumps({"test": "data"}, ensure_ascii=False)
    // produces — default ", " / ": " separators.
    br#"{"test": "data"}"#.to_vec()
}

// ---------------------------------------------------------------------------
// write (encode) — mirrors test_write_event / test_write_event_no_payload
// ---------------------------------------------------------------------------

#[test]
fn test_write_event() {
    let event = Event {
        type_: "test-event".to_string(),
        data: data_map(),
        payload: Some(PAYLOAD.to_vec()),
    };
    let bytes = encode(&event);
    let mut reader = Cursor::new(&bytes);

    // First line is JSON with event type and data/payload lengths.
    let mut line = Vec::new();
    std::io::BufRead::read_until(&mut reader, b'\n', &mut line).unwrap();
    line.pop();
    let header: Value = serde_json::from_slice(&line).unwrap();
    assert_eq!(
        header,
        json!({
            "type": "test-event",
            "version": VERSION,
            "data_length": data_bytes().len(),
            "payload_length": PAYLOAD.len(),
        })
    );

    // Data dict comes next, encoded as UTF-8 JSON.
    let mut got = vec![0u8; data_bytes().len()];
    std::io::Read::read_exact(&mut reader, &mut got).unwrap();
    assert_eq!(got, data_bytes());

    // Payload comes last.
    let mut payload = vec![0u8; PAYLOAD.len()];
    std::io::Read::read_exact(&mut reader, &mut payload).unwrap();
    assert_eq!(payload, PAYLOAD);

    // No more data.
    let mut rest = Vec::new();
    std::io::Read::read_to_end(&mut reader, &mut rest).unwrap();
    assert!(rest.is_empty());
}

#[test]
fn test_write_event_no_payload() {
    let event = Event {
        type_: "test-event".to_string(),
        data: data_map(),
        payload: None,
    };
    let bytes = encode(&event);
    let mut reader = Cursor::new(&bytes);

    let mut line = Vec::new();
    std::io::BufRead::read_until(&mut reader, b'\n', &mut line).unwrap();
    line.pop();
    let header: Value = serde_json::from_slice(&line).unwrap();
    assert_eq!(
        header,
        json!({
            "type": "test-event",
            "version": VERSION,
            "data_length": data_bytes().len(),
        })
    );

    let mut got = vec![0u8; data_bytes().len()];
    std::io::Read::read_exact(&mut reader, &mut got).unwrap();
    assert_eq!(got, data_bytes());

    // No payload.
    let mut rest = Vec::new();
    std::io::Read::read_to_end(&mut reader, &mut rest).unwrap();
    assert!(rest.is_empty());
}

// ---------------------------------------------------------------------------
// read (decode) — mirrors test_read_event / test_read_event_no_data
// ---------------------------------------------------------------------------

#[test]
fn test_read_event() {
    // Header with inline data that the additional data block overwrites.
    let header = json!({
        "type": "test-event",
        "version": VERSION,
        "data_length": data_bytes().len(),
        "payload_length": PAYLOAD.len(),
        "data": {
            "test": "this will be overwritten by DATA",
            "test2": "this will not"
        }
    });

    let mut bytes = serde_json::to_vec(&header).unwrap();
    bytes.push(b'\n');
    bytes.extend(data_bytes());
    bytes.extend(PAYLOAD);

    let mut reader = Cursor::new(bytes);
    let event = decode(&mut reader).unwrap().expect("event");
    assert_eq!(
        event,
        Event {
            type_: "test-event".to_string(),
            // inline data was overwritten
            data: json!({"test": "data", "test2": "this will not"}).as_object().unwrap().clone(),
            payload: Some(PAYLOAD.to_vec()),
        }
    );
}

#[test]
fn test_read_event_no_data() {
    let header = json!({"type": "test-event", "version": VERSION});
    let mut bytes = serde_json::to_vec(&header).unwrap();
    bytes.push(b'\n');

    let mut reader = Cursor::new(bytes);
    let event = decode(&mut reader).unwrap().expect("event");
    assert_eq!(event, Event::new("test-event"));
}

// ---------------------------------------------------------------------------
// round-trips and edge cases
// ---------------------------------------------------------------------------

#[test]
fn test_roundtrip_empty_event() {
    let event = Event::new("describe");
    let bytes = encode(&event);
    let mut reader = Cursor::new(bytes);
    assert_eq!(decode(&mut reader).unwrap(), Some(event));
    assert_eq!(decode(&mut reader).unwrap(), None); // clean EOF
}

#[test]
fn test_roundtrip_multiple_events_in_one_stream() {
    let events = vec![
        Event::new("audio-start").with_data(
            json!({"rate": 16000, "width": 2, "channels": 1}).as_object().unwrap().clone(),
        ),
        Event::new("audio-chunk")
            .with_data(json!({"rate": 16000, "width": 2, "channels": 1}).as_object().unwrap().clone())
            .with_payload(vec![0u8, 1, 2, 3, 254, 255]),
        Event::new("audio-stop"),
    ];
    let mut bytes = Vec::new();
    for event in &events {
        bytes.extend(encode(event));
    }
    let mut reader = Cursor::new(bytes);
    for event in &events {
        assert_eq!(&decode(&mut reader).unwrap().expect("event"), event);
    }
    assert_eq!(decode(&mut reader).unwrap(), None);
}

#[test]
fn test_roundtrip_unicode() {
    let mut data = Map::new();
    data.insert("text".to_string(), json!("Καλημέρα κόσμε 🎙️"));
    let event = Event::new("transcript").with_data(data);
    let bytes = encode(&event);
    // ensure_ascii=False upstream: raw UTF-8, no \u escapes
    assert!(bytes.windows("Καλημέρα".as_bytes().len()).any(|w| w == "Καλημέρα".as_bytes()));
    let mut reader = Cursor::new(bytes);
    assert_eq!(decode(&mut reader).unwrap(), Some(event));
}

#[test]
fn test_zero_length_fields_are_treated_as_absent() {
    // Upstream: `data_length is not None and data_length > 0`.
    let header = json!({
        "type": "test-event",
        "data_length": 0,
        "payload_length": 0
    });
    let mut bytes = serde_json::to_vec(&header).unwrap();
    bytes.push(b'\n');
    let mut reader = Cursor::new(bytes);
    assert_eq!(decode(&mut reader).unwrap(), Some(Event::new("test-event")));
}

#[test]
fn test_empty_payload_is_omitted_on_encode() {
    let event = Event::new("test-event").with_payload(Vec::new());
    let bytes = encode(&event);
    let header_line = bytes.split(|b| *b == b'\n').next().unwrap();
    let header: Value = serde_json::from_slice(header_line).unwrap();
    assert!(header.get("payload_length").is_none());
    // Decodes back with no payload.
    let mut reader = Cursor::new(bytes);
    assert_eq!(decode(&mut reader).unwrap(), Some(Event::new("test-event")));
}

#[test]
fn test_truncated_payload_is_error() {
    let header = json!({"type": "audio-chunk", "payload_length": 10});
    let mut bytes = serde_json::to_vec(&header).unwrap();
    bytes.push(b'\n');
    bytes.extend(b"short");
    let mut reader = Cursor::new(bytes);
    let err = decode(&mut reader).unwrap_err();
    assert!(matches!(err, wyoming::Error::UnexpectedEof), "{err}");
}

#[test]
fn test_truncated_line_is_error() {
    let mut reader = Cursor::new(b"{\"type\": \"x\"");
    let err = decode(&mut reader).unwrap_err();
    assert!(matches!(err, wyoming::Error::UnexpectedEof), "{err}");
}

#[test]
fn test_malformed_json_is_error() {
    let mut reader = Cursor::new(b"not json\n");
    let err = decode(&mut reader).unwrap_err();
    assert!(matches!(err, wyoming::Error::InvalidHeader(_)), "{err}");
}

#[test]
fn test_missing_type_is_error() {
    let mut reader = Cursor::new(b"{\"version\": \"1.11.0\"}\n");
    let err = decode(&mut reader).unwrap_err();
    assert!(matches!(err, wyoming::Error::MissingType), "{err}");
}

#[test]
fn test_non_object_additional_data_is_error() {
    let header = json!({"type": "test-event", "data_length": 5});
    let mut bytes = serde_json::to_vec(&header).unwrap();
    bytes.push(b'\n');
    bytes.extend(b"[1,2]");
    let mut reader = Cursor::new(bytes);
    let err = decode(&mut reader).unwrap_err();
    assert!(matches!(err, wyoming::Error::InvalidData(_)), "{err}");
}

#[test]
fn test_large_payload_roundtrip() {
    // 16 kHz, 16-bit mono, 0.5 s of audio.
    let audio: Vec<u8> = (0..16000).flat_map(|i| (i as i16).to_le_bytes()).collect();
    let mut data = Map::new();
    data.insert("rate".to_string(), json!(16000));
    data.insert("width".to_string(), json!(2));
    data.insert("channels".to_string(), json!(1));
    let event = Event::new("audio-chunk").with_data(data).with_payload(audio);
    let bytes = encode(&event);
    let mut reader = Cursor::new(bytes);
    assert_eq!(decode(&mut reader).unwrap(), Some(event));
}

// ---------------------------------------------------------------------------
// byte-exact encoder parity with Python json.dumps(ensure_ascii=False)
// ---------------------------------------------------------------------------

#[test]
fn encode_is_byte_identical_to_python_write_event() {
    // Exact bytes captured from the Python reference (08a0c25) write_event:
    // json.dumps defaults use ", " / ": " separators, not compact JSON.
    let mut data = Map::new();
    data.insert("rate".to_string(), json!(16000));
    data.insert("width".to_string(), json!(2));
    data.insert("channels".to_string(), json!(1));
    data.insert("timestamp".to_string(), Value::Null);
    let event = Event::new("audio-start").with_data(data);
    assert_eq!(
        encode(&event),
        b"{\"type\": \"audio-start\", \"version\": \"1.11.0\", \"data_length\": 61}\n\
          {\"rate\": 16000, \"width\": 2, \"channels\": 1, \"timestamp\": null}"
            .to_vec()
    );
}

#[test]
fn encode_writes_unicode_raw_like_python() {
    // ensure_ascii=False: non-ASCII goes out as raw UTF-8, not \uXXXX.
    let mut data = Map::new();
    data.insert("text".to_string(), json!("café — ünïcode"));
    let event = Event::new("transcript").with_data(data);
    let block = "{\"text\": \"café — ünïcode\"}";
    let header = format!(
        "{{\"type\": \"transcript\", \"version\": \"{VERSION}\", \"data_length\": {}}}\n",
        block.as_bytes().len()
    );
    let expected = [header.as_bytes(), block.as_bytes()].concat();
    assert_eq!(encode(&event), expected);
}

#[test]
fn encode_escapes_control_chars_like_python() {
    let mut data = Map::new();
    data.insert("text".to_string(), json!("a\u{1}\n\t\"b\\"));
    let event = Event::new("transcript").with_data(data);
    let block = "{\"text\": \"a\\u0001\\n\\t\\\"b\\\\\"}";
    let header = format!(
        "{{\"type\": \"transcript\", \"version\": \"{VERSION}\", \"data_length\": {}}}\n",
        block.as_bytes().len()
    );
    let expected = [header.as_bytes(), block.as_bytes()].concat();
    assert_eq!(encode(&event), expected);
}
