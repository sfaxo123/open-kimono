//! DC-401 input-batch codec contract tests.
//!
//! Covers: literal-byte round-trip per event kind, coordinate and batch
//! bound rejections (before any allocation), the exact held set after a
//! scripted press-tap-release batch, and ID-independence of both the held
//! set and the encoded event payload bytes.

use device_control::input_record::{
    InputBatch, InputError, InputEvent, InputReject, KeyEvent, KeyPhase, PointerEvent, SwipeEvent,
    TapEvent, TextEvent, MAX_BATCH, MAX_X, MAX_Y, RECORD_EVENT_LEN,
};
use device_control::{Frame, Op, Policy};

/// One event record as literal bytes: tag, action, slot, x, y.
const fn rec(tag: u8, action: u8, slot: u8, x: u32, y: u32) -> [u8; 16] {
    let mut b = [0u8; 16];
    b[0] = tag;
    b[1] = action;
    b[2] = slot;
    b[3] = x as u8;
    b[4] = (x >> 8) as u8;
    b[5] = (x >> 16) as u8;
    b[6] = (x >> 24) as u8;
    b[7] = y as u8;
    b[8] = (y >> 8) as u8;
    b[9] = (y >> 16) as u8;
    b[10] = (y >> 24) as u8;
    b
}

const fn header(count: u16) -> [u8; 5] {
    [count as u8, (count >> 8) as u8, 0, 0, 0]
}

/// 1. Literal-byte fixture round-trip for every event kind: the checked-in
/// expected bytes decode to the expected typed events, and re-encoding the
/// decoded batch reproduces the fixture byte-for-byte.
#[test]
fn literal_byte_roundtrip_per_kind() {
    let tap = rec(0, 0, 0, 540, 1200);
    let press = rec(2, 1, 0, 100, 200);
    let move_r = rec(3, 2, 0, 300, 400);
    let release = rec(4, 3, 0, 300, 400);
    let key_press = [5u8, 0, 0, 65, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    let key_release = [5u8, 1, 0, 65, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    let text = [6u8, 0, 2, b'o', b'k', 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    // swipe: slot 0, (100,512) -> (900,1536); y1=1536=6*256 exactly.
    let swipe = [1u8, 0, 0, 100, 0, 0, 0, 0, 2, 0, 0, 132, 3, 0, 0, 6];

    let records: [&[u8; 16]; 8] = [
        &tap,
        &swipe,
        &press,
        &move_r,
        &release,
        &key_press,
        &key_release,
        &text,
    ];
    let mut body: Vec<u8> = header(8).to_vec();
    for r in records {
        body.extend_from_slice(r);
    }

    let batch = InputBatch::decode(&body).expect("fixture must decode");
    assert_eq!(batch.event_count(), 8);

    let wire: Vec<InputEvent<'_>> = batch.wire_events().collect();
    assert_eq!(wire.len(), 8);
    assert_eq!(
        wire[0],
        InputEvent::Tap(TapEvent::new(0, 540, 1200)),
        "tap literal bytes"
    );
    assert_eq!(
        wire[1],
        InputEvent::Swipe(SwipeEvent::new(0, 100, 512, 900, 1536)),
        "swipe literal bytes"
    );
    assert_eq!(
        wire[2],
        InputEvent::Pointer(PointerEvent::press(0, 100, 200)),
        "press literal bytes"
    );
    assert_eq!(
        wire[3],
        InputEvent::Pointer(PointerEvent::move_to(0, 300, 400)),
        "move literal bytes"
    );
    assert_eq!(
        wire[4],
        InputEvent::Pointer(PointerEvent::release(0, 300, 400)),
        "release literal bytes"
    );
    assert_eq!(
        wire[5],
        InputEvent::Key(KeyEvent::new(65, KeyPhase::Press)),
        "key press literal bytes"
    );
    assert_eq!(
        wire[6],
        InputEvent::Key(KeyEvent::new(65, KeyPhase::Release)),
        "key release literal bytes"
    );
    assert_eq!(
        wire[7],
        InputEvent::Text(TextEvent::new("ok")),
        "text literal bytes"
    );

    // Re-encode must be byte-for-byte identical to the fixture.
    let mut out = vec![0u8; body.len()];
    let n = batch.encode_into(&mut out).expect("re-encode");
    assert_eq!(n, body.len());
    assert_eq!(out, body, "re-encode must be literal-identical");
}

/// 2. Out-of-range coordinates rejected: x = 1081, y = 2281, and a
/// negative coordinate expressed as a u32 wrap (0xFFFF_FFFF). The
/// rejection is typed and happens on the record scan — the decoder
/// allocates nothing at any point (it is a borrowed decoder: validation
/// precedes any construction, and the batch value never exists).
#[test]
fn out_of_range_coordinates_rejected() {
    let cases: [(u32, u32); 3] = [(MAX_X + 1, 0), (0, MAX_Y + 1), (0xFFFF_FFFF, 0)];
    for (x, y) in cases {
        let mut body: Vec<u8> = header(1).to_vec();
        body.extend_from_slice(&rec(2, 1, 0, x, y));
        let err = InputBatch::decode(&body).expect_err("out-of-range must be rejected");
        match err {
            InputError::Body(InputReject::PointerXOutOfRange)
            | InputError::Body(InputReject::PointerYOutOfRange) => {}
            other => panic!("wrong rejection for ({x},{y}): {other:?}"),
        }
    }
    // Encode-side preflight too: an out-of-range event is refused before
    // any byte is written, and the output is left untouched.
    let mut out = [0u8; 64];
    let before = out;
    let events = [InputEvent::Tap(TapEvent::new(0, MAX_X + 1, 0))];
    let err = InputBatch::encode_body(&events, &mut out).expect_err("encode must reject");
    assert_eq!(err, InputError::Body(InputReject::PointerXOutOfRange));
    assert_eq!(out, before, "rejected encode must not touch output");
}

/// 3. Oversize batch rejected BEFORE allocation: a header declaring
/// MAX_BATCH + 1 events is rejected from the 5-byte header alone — the
/// body does not even need to contain the declared records, proving the
/// count bound is enforced before any per-event work.
#[test]
fn oversize_batch_rejected_before_allocation() {
    let h = header((MAX_BATCH + 1) as u16);
    // No records at all: the count check must fire first.
    let err = InputBatch::decode(&h).expect_err("oversize count must be rejected");
    assert_eq!(err, InputError::Body(InputReject::BatchCountOutOfRange));

    // A full oversize body is rejected the same way.
    let mut body: Vec<u8> = header((MAX_BATCH + 1) as u16).to_vec();
    for _ in 0..MAX_BATCH + 1 {
        body.extend_from_slice(&rec(0, 0, 0, 1, 1));
    }
    let err = InputBatch::decode(&body).expect_err("oversize body must be rejected");
    assert_eq!(err, InputError::Body(InputReject::BatchCountOutOfRange));

    // Exactly MAX_BATCH is accepted.
    let mut body: Vec<u8> = header(MAX_BATCH as u16).to_vec();
    for _ in 0..MAX_BATCH {
        body.extend_from_slice(&rec(0, 0, 0, 1, 1));
    }
    let batch = InputBatch::decode(&body).expect("MAX_BATCH must be accepted");
    assert_eq!(batch.event_count(), MAX_BATCH);
}

/// 4. Held set after a scripted press-tap-release batch is EXACT: press
/// holds slot 0; the tap on slot 1 is press+release (net empty, and it
/// must not disturb slot 0); the release clears slot 0. Asserted against
/// the full HeldSet value, not just emptiness.
#[test]
fn held_set_after_scripted_batch_is_exact() {
    let press = rec(2, 1, 0, 100, 200);
    let tap = rec(0, 0, 1, 540, 1200);
    let release = rec(4, 3, 0, 100, 200);
    let mut body: Vec<u8> = header(3).to_vec();
    body.extend_from_slice(&press);
    body.extend_from_slice(&tap);
    body.extend_from_slice(&release);

    let batch = InputBatch::decode(&body).expect("scripted batch must decode");

    // Held set of the FULL scripted batch: everything netted out.
    let held = batch.held_set();
    assert!(held.is_empty(), "full batch must net to empty: {held:?}");
    assert!(!held.pointer_down(0));
    assert!(!held.pointer_down(1));

    // Prefix batches prove the fold actually tracks state, not just nets:
    // after press only: slot 0 held.
    let after_press = InputBatch::decode(&[header(1).to_vec(), press.to_vec()].concat())
        .expect("prefix 1")
        .held_set();
    assert!(after_press.pointer_down(0));
    assert!(!after_press.pointer_down(1));

    // after press+tap: slot 0 still held, tap netted.
    let after_tap =
        InputBatch::decode(&[header(2).to_vec(), press.to_vec(), tap.to_vec()].concat())
            .expect("prefix 2")
            .held_set();
    assert!(after_tap.pointer_down(0));
    assert!(!after_tap.pointer_down(1));

    // Key press/release fold, exact bit asserted.
    let mut body: Vec<u8> = header(2).to_vec();
    body.extend_from_slice(&[5u8, 0, 0, 65, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    body.extend_from_slice(&[5u8, 0, 0, 66, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    let held = InputBatch::decode(&body).expect("keys").held_set();
    assert!(held.key_down(65));
    assert!(held.key_down(66));
    assert!(!held.key_down(64));
}

/// 5. ID-independence: the same event sequence under two different
/// session/request ids yields an identical held set AND identical encoded
/// event payload bytes. The ids live only in the envelope; the body bytes
/// and the fold never see them.
#[test]
fn id_independence() {
    let events = [
        InputEvent::Pointer(PointerEvent::press(0, 100, 200)),
        InputEvent::Tap(TapEvent::new(1, 540, 1200)),
        InputEvent::Pointer(PointerEvent::release(0, 100, 200)),
    ];

    let mut body_a = [0u8; 5 + 3 * RECORD_EVENT_LEN];
    let mut body_b = [0u8; 5 + 3 * RECORD_EVENT_LEN];
    let n_a = InputBatch::encode_body(&events, &mut body_a).expect("encode a");
    let n_b = InputBatch::encode_body(&events, &mut body_b).expect("encode b");
    assert_eq!(n_a, n_b);
    assert_eq!(
        &body_a[..n_a],
        &body_b[..n_b],
        "payload bytes must be id-free"
    );

    let held_a = InputBatch::decode(&body_a).expect("decode a").held_set();
    let held_b = InputBatch::decode(&body_b).expect("decode b").held_set();
    assert_eq!(held_a, held_b, "held set must be id-independent");
    assert!(held_a.is_empty());

    // Now through the envelope with two DIFFERENT correlation ids and two
    // different boot ids: the payload half of each frame is identical, and
    // the decoded held sets are identical.
    let batch = InputBatch::decode(&body_a).expect("batch");
    let boot_1: &[u8; 16] = b"boot-id-00000001";
    let boot_2: &[u8; 16] = b"boot-id-00000002";
    let mut frame_1 = [0u8; 36 + 64];
    let mut frame_2 = [0u8; 36 + 64];
    let f1 = batch
        .encode_envelope(0xA11CE, boot_1, &mut frame_1, &Policy::default())
        .expect("frame 1");
    let f2 = batch
        .encode_envelope(0xB0B, boot_2, &mut frame_2, &Policy::default())
        .expect("frame 2");
    assert_eq!(f1, f2);
    assert_eq!(&frame_1[36..f1], &frame_2[36..f2], "payload id-free");

    let parsed_1 = Frame::parse(&frame_1[..f1], &Policy::default()).expect("parse 1");
    let parsed_2 = Frame::parse(&frame_2[..f2], &Policy::default()).expect("parse 2");
    assert_eq!(parsed_1.op(), Op::Input);
    assert_eq!(parsed_1.correlation_id(), 0xA11CE);
    assert_eq!(parsed_2.correlation_id(), 0xB0B);

    let framed_1 = InputBatch::from_frame(&parsed_1).expect("framed 1");
    let framed_2 = InputBatch::from_frame(&parsed_2).expect("framed 2");
    assert_eq!(framed_1.batch().held_set(), framed_2.batch().held_set());
    assert_eq!(framed_1.correlation_id(), 0xA11CE);
    assert_eq!(framed_2.correlation_id(), 0xB0B);
}
