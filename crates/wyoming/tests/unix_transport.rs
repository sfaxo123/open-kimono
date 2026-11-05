#![cfg(unix)]
//! UnixStream transport coverage (Cursor review W-10).
//!
//! `transport.rs` implements [`wyoming::Sink`] for `std::os::unix::net::UnixStream`
//! with a real `shutdown(Shutdown::Both)`, but until now no test exercised it:
//! the in-crate tests only cover the in-memory mock and TCP loopback. These
//! tests drive the codec over a real `UnixStream::pair()` and assert:
//!
//! 1. a `Framed` round-trip of several events (one with a binary payload)
//!    in both directions;
//! 2. `Framed::shutdown` on one end delivers a clean EOF — the peer's
//!    `read_frame` returns `Ok(None)`, bounded by a read timeout so a
//!    missing shutdown fails fast instead of hanging;
//! 3. a truncated frame written raw and then shut down yields
//!    `Err(UnexpectedEof)` on the peer — an error, not a hang.

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::time::Duration;

use serde_json::json;
use wyoming::error::Error;
use wyoming::transport::Framed;
use wyoming::{encode, Event};

/// Every read in these tests is bounded: a transport that never delivers
/// EOF (or the error ending a frame) fails with a timeout instead of
/// hanging the test suite.
const READ_TIMEOUT: Duration = Duration::from_millis(1500);

/// A connected unix socketpair, both ends wrapped in `Framed`, both read
/// sides on a bounded timeout.
fn framed_pair() -> (Framed<UnixStream>, Framed<UnixStream>) {
    let (a, b) = UnixStream::pair().expect("unix socketpair");
    a.set_read_timeout(Some(READ_TIMEOUT))
        .expect("set read timeout on end A");
    b.set_read_timeout(Some(READ_TIMEOUT))
        .expect("set read timeout on end B");
    (Framed::new(a), Framed::new(b))
}

fn ping_event() -> Event {
    let mut data = serde_json::Map::new();
    data.insert("text".to_string(), json!("unix-hello"));
    Event::new("ping").with_data(data)
}

/// Event with inline data AND a 1 KiB binary payload block.
fn audio_chunk_event() -> Event {
    let mut data = serde_json::Map::new();
    data.insert("rate".to_string(), json!(16000));
    data.insert("channels".to_string(), json!(1));
    Event::new("audio-chunk")
        .with_data(data)
        .with_payload((0..=255u8).cycle().take(1024).collect::<Vec<u8>>())
}

/// (1) Framed round-trip over a unix socketpair in both directions,
/// including a frame with a binary payload.
#[test]
fn unix_pair_roundtrip_both_directions() {
    let (mut a, mut b) = framed_pair();

    // A -> B: a plain frame and one with data + payload.
    a.write_frame(&ping_event()).expect("A write ping");
    a.write_frame(&audio_chunk_event()).expect("A write audio-chunk");

    // B -> A: concurrent reverse direction.
    let pong = Event::new("pong");
    b.write_frame(&pong).expect("B write pong");

    let got_ping = b.read_frame().expect("B read ping").expect("ping frame");
    assert_eq!(got_ping, ping_event());

    let got_chunk = b
        .read_frame()
        .expect("B read audio-chunk")
        .expect("audio-chunk frame");
    assert_eq!(got_chunk, audio_chunk_event());
    assert_eq!(
        got_chunk.payload.as_deref().map(<[u8]>::len),
        Some(1024),
        "binary payload block must survive the unix socket intact"
    );

    let got_pong = a.read_frame().expect("A read pong").expect("pong frame");
    assert_eq!(got_pong, pong);
}

/// (2) `Framed::shutdown` on one end makes the peer's `read_frame` return
/// `Ok(None)` — a clean EOF — within the bounded read timeout.
#[test]
fn unix_shutdown_delivers_clean_eof_to_peer() {
    let (mut a, mut b) = framed_pair();

    // A sends one frame, then shuts the channel down.
    a.write_frame(&Event::new("bye")).expect("A write bye");
    a.shutdown().expect("A shutdown");

    // B still receives the queued frame, then must observe clean EOF.
    let bye = b.read_frame().expect("B read bye").expect("bye frame");
    assert_eq!(bye, Event::new("bye"));

    let eof = b.read_frame();
    assert!(
        matches!(eof, Ok(None)),
        "peer must observe clean EOF (Ok(None)) after Framed::shutdown, got {eof:?}"
    );
}

/// (3) Negative: a truncated frame (header promises a payload block, only
/// part of it is written) followed by shutdown must surface
/// `Err(UnexpectedEof)` on the peer — an error, not a hang.
#[test]
fn unix_truncated_frame_then_shutdown_yields_error_not_hang() {
    let (mut a, mut b) = framed_pair();

    // A writes one complete frame, then a truncated one raw (the last 512
    // payload bytes never arrive), then shuts down.
    let full = Event::new("transcript");
    a.write_frame(&full).expect("A write full frame");

    // Raw write bypasses the codec so the frame can be truncated.
    let mut bytes = encode(&audio_chunk_event());
    bytes.truncate(bytes.len() - 512);
    a.get_mut().write_all(&bytes).expect("A raw write truncated");
    a.get_mut().flush().expect("A raw flush");
    a.shutdown().expect("A shutdown");

    // B reads the complete frame first: shutdown must not eat queued bytes.
    let got_full = b.read_frame().expect("B read full").expect("transcript frame");
    assert_eq!(got_full, full);

    // Then the truncated frame must fail with UnexpectedEof, not hang and
    // not silently decode.
    let err = b
        .read_frame()
        .expect_err("truncated frame must surface an error, not decode or hang");
    assert!(
        matches!(err, Error::UnexpectedEof),
        "expected UnexpectedEof for a truncated frame, got {err:?}"
    );
}
