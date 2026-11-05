//! Three focused tests for the scripted borrowed-frame fake transport.

use device_control::fake_transport::{
    open, FakeTransport, Script, TransportKind, UnsupportedTransport, UNSUPPORTED_TRANSPORT,
};
use device_control::image_record::ImageRecord;
use device_control::log_record::LogRecord;
use device_control::{Frame, Op, Policy};

/// Test 1: the two log frames are served in script order as slices BORROWED
/// from the script buffers and decode through the log_record codec.
#[test]
fn log_frames_served_in_order_borrowed_and_decodable() {
    let script = Script::new();
    let mut t = FakeTransport::scripted(&script);
    let policy = Policy::default();

    let first = t.serve().expect("frame 1");
    let second = t.serve().expect("frame 2");

    // Borrowed proof: both slices lie inside the script's own buffers.
    let base = script.span_base();
    let end = script.span_end();
    let p1 = first.as_ptr() as usize;
    let p2 = second.as_ptr() as usize;
    assert!(p1 >= base && p1 + first.len() <= end);
    assert!(p2 >= base && p2 + second.len() <= end);

    let f1 = Frame::parse(first, &policy).expect("frame 1 parses");
    let f2 = Frame::parse(second, &policy).expect("frame 2 parses");
    assert_eq!(f1.op(), Op::Logs);
    assert_eq!(f2.op(), Op::Logs);

    let l1 = LogRecord::from_frame(&f1).expect("log 1 decodes");
    let l2 = LogRecord::from_frame(&f2).expect("log 2 decodes");
    // Script order: seq 1 then seq 2.
    assert_eq!(l1.record().sequence(), 1);
    assert_eq!(l2.record().sequence(), 2);
    assert_eq!(l1.record().message(), "fake boot");
    assert_eq!(l2.record().message(), "fake ready");

    // Script has exactly 3 frames: after the screenshot, serve is None.
    assert!(t.serve().is_some());
    assert!(t.serve().is_none());
}

/// Test 2: the screenshot frame is served third and decodes through the
/// image_record codec.
#[test]
fn screenshot_frame_served_and_decodable() {
    let script = Script::new();
    let mut t = FakeTransport::scripted(&script);
    let policy = Policy::default();

    let _ = t.serve().expect("log 1");
    let _ = t.serve().expect("log 2");
    let bytes = t.serve().expect("screenshot frame");

    // Borrowed from the script, and matches the declared screenshot span.
    let (s, e) = script.frame_span(2).expect("span 2");
    assert_eq!(bytes.as_ptr() as usize - script.span_base(), s);
    assert_eq!(bytes.len(), e - s);

    let frame = Frame::parse(bytes, &policy).expect("screenshot parses");
    assert_eq!(frame.op(), Op::Screenshot);
    let img = ImageRecord::from_frame(&frame).expect("image decodes");
    assert_eq!(img.record().width(), 2);
    assert_eq!(img.record().height(), 2);
    // Pixels borrow the served frame — no copy.
    assert_eq!(img.record().pixels(), &[0x10u8; 16]);

    // Bare-body decode path works too.
    let bare = ImageRecord::decode(frame.payload()).expect("bare body decodes");
    assert_eq!(bare.frame_seq(), 1);
}

/// Test 3: an unsupported transport variant returns the typed
/// UNSUPPORTED-TRANSPORT result — no fallback, no panic.
#[test]
fn unsupported_variant_returns_typed_unsupported_transport() {
    let script = Script::new();
    let res = open(TransportKind::Usb, &script);
    assert!(res.is_err());
    let err = match res {
        Err(e) => e,
        Ok(_) => unreachable!(),
    };
    assert_eq!(err, UnsupportedTransport::UnsupportedTransport);
    assert_eq!(err, UNSUPPORTED_TRANSPORT);

    extern crate alloc;
    use alloc::string::ToString;
    assert_eq!(err.to_string(), "UNSUPPORTED-TRANSPORT");

    // The supported variant still opens.
    assert!(open(TransportKind::Scripted, &script).is_ok());
}
