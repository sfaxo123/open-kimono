//! Canonical command-REQUEST frame demo for the bounded command codec.
//!
//! Encodes ONE command request (kind = request-body tag 1, argv `ls` /
//! "" / b"\xff\xfe bytes", timeout 5000 ms) into a full control-envelope
//! v1 frame and prints the frame's bytes as one lowercase hex line. The
//! printed bytes are asserted byte-for-byte against the checked-in literal
//! fixture `REQ_WIRE` in `tests/command_request.rs` (see
//! `demo_example_output_matches_checked_in_fixture` there). This example
//! executes nothing: it only turns validated arguments into bytes and
//! decodes them back from the printed buffer.

use device_control::command_record::CommandRequest;
use device_control::{Frame, Policy};

fn main() {
    // The canonical demo request — identical to the fixture in the test
    // target. Any drift between the two breaks the test, not just here.
    let args: [&[u8]; 3] = [
        b"ls".as_slice(),
        b"".as_slice(),
        b"\xff\xfe bytes".as_slice(),
    ];
    let request = CommandRequest::new(5000, &args).expect("canonical request is valid");

    let correlation_id: u64 = 0x0102_0304_0506_0708;
    let boot_id: [u8; 16] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];

    // Encode the full envelope (36-byte frame header + 23-byte request
    // body) into a caller-provided buffer. 59 is the exact canonical size.
    let mut frame = [0u8; 59];
    let written = request
        .encode_envelope(correlation_id, &boot_id, &mut frame, &Policy::default())
        .expect("canonical envelope encodes");

    // Decode it back through the one-call path and show that every field
    // came from the printed bytes — nothing is fabricated.
    let decoded = CommandRequest::decode_envelope(&frame[..written], &Policy::default())
        .expect("canonical envelope decodes");
    let reparsed = Frame::parse(&frame[..written], &Policy::default()).expect("frame parses");

    println!(
        "command request frame ({} bytes, op={:?}, kind=tag 1):",
        written,
        reparsed.op()
    );
    for i in 0..written {
        print!("{:02x}", frame[i]);
    }
    println!();
    println!(
        "decoded: correlation_id={:#018x} timeout_ms={} argc={} args={}..{}",
        decoded.correlation_id(),
        decoded.request().timeout_ms(),
        decoded.request().argc(),
        decoded.request().args().next().map_or(0, |a| a[0]),
        decoded
            .request()
            .args()
            .last()
            .map_or(0, |a| a[a.len() - 1]),
    );
}
