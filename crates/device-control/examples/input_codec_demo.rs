//! DC-401 input codec demo: encodes a scripted press-tap-release batch,
//! prints the hex bytes and the held set. Output is deterministic and
//! byte-identical across runs — the held set depends only on the event
//! sequence, never on any id.

use device_control::input_record::{
    InputBatch, InputEvent, PointerEvent, TapEvent, RECORD_EVENT_LEN,
};

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn held_line(held: &device_control::input_record::HeldSet) -> String {
    let pointers: Vec<String> = (0..held.pointers.len())
        .filter(|&i| held.pointers[i])
        .map(|i| format!("slot {i}"))
        .collect();
    let keys: Vec<String> = (0..768u16)
        .filter(|&c| held.key_down(c))
        .map(|c| format!("key {c}"))
        .collect();
    if pointers.is_empty() && keys.is_empty() {
        "held: (empty)".to_string()
    } else {
        format!(
            "held: {}{}",
            pointers.join(", "),
            if keys.is_empty() {
                String::new()
            } else {
                format!(
                    "{}keys: {}",
                    if pointers.is_empty() { "" } else { "; " },
                    keys.join(", ")
                )
            }
        )
    }
}

fn main() {
    // Scripted batch: press (slot 0 @ 100,200), tap (slot 1 @ 540,1200),
    // release (slot 0 @ 100,200).
    let events = [
        InputEvent::Pointer(PointerEvent::press(0, 100, 200)),
        InputEvent::Tap(TapEvent::new(1, 540, 1200)),
        InputEvent::Pointer(PointerEvent::release(0, 100, 200)),
    ];

    let mut body = [0u8; 5 + 3 * RECORD_EVENT_LEN];
    let n = InputBatch::encode_body(&events, &mut body).expect("scripted batch must encode");
    println!("batch: {} wire records, {n} body bytes", events.len());
    println!("body bytes: {}", hex(&body[..n]));

    let batch = InputBatch::decode(&body[..n]).expect("encoded batch must decode");
    println!("decoded: {batch}");
    println!("{}", held_line(&batch.held_set()));

    // Prefix view: after press + tap (a two-record batch), slot 0 is held
    // and the tap nets empty.
    let prefix_events = &events[..2];
    let mut prefix_body = [0u8; 5 + 2 * RECORD_EVENT_LEN];
    let pn = InputBatch::encode_body(prefix_events, &mut prefix_body).expect("prefix encode");
    let prefix = InputBatch::decode(&prefix_body[..pn]).expect("prefix batch must decode");
    println!("after press+tap -> {}", held_line(&prefix.held_set()));
}
