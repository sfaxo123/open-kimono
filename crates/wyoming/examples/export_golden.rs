//! Re-encodes the golden fixtures (`tests/fixtures/golden.json`) with the
//! Rust encoder to a file given as argv[1], for the Python cross-check
//! (`tests/check_from_rust.py`).

use std::io::Write;
use std::path::PathBuf;

use serde::Deserialize;
use serde_json::Value;
use wyoming::{encode, Event};

#[derive(Deserialize)]
struct Expected {
    #[serde(rename = "type")]
    type_: String,
    data: Value,
    payload_hex: Option<String>,
}

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

fn main() {
    let out_path = std::env::args().nth(1).expect("usage: export_golden <out.wyoming>");
    let json_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/golden.json");
    let expected: Vec<Expected> = serde_json::from_slice(&std::fs::read(json_path).unwrap())
        .expect("golden.json parses");

    let mut out = std::fs::File::create(out_path).unwrap();
    for want in &expected {
        let data = match want.data.as_object() {
            Some(map) => map.clone(),
            None => serde_json::Map::new(),
        };
        let event = Event {
            type_: want.type_.clone(),
            data,
            payload: want.payload_hex.as_deref().map(unhex),
        };
        out.write_all(&encode(&event)).unwrap();
    }
    out.flush().unwrap();
    eprintln!("wrote {} events", expected.len());
}
