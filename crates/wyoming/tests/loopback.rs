//! Loopback gate: the Rust client state machine over a real TCP socket on
//! 127.0.0.1 against a minimal fake Wyoming server written in Rust (same
//! crate codec, so the server side is byte-compatible with the Python
//! reference by construction — the 59 golden fixtures in
//! `tests/golden_fixtures.rs` already prove the codec cross-check).
//!
//! Flow asserted end-to-end:
//!   client ping -> server pong
//!   unsolicited wake `detection`
//!   client audio-start/chunk/stop -> server transcript
//!   server speech audio-start/chunk/stop -> client back to idle
//!   server keepalive ping -> client auto-pong
//! plus a journal of every transition on the client side.

use std::io::{BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;

use serde_json::{json, Value};
use wyoming::client::{Client, ClientState, Transition, TransitionKind};
use wyoming::events::audio::{AudioChunk, AudioStart, AudioStop};
use wyoming::events::asr::Transcript;
use wyoming::events::ping::{Ping, Pong};
use wyoming::events::wake::Detection;
use wyoming::events::Eventable;
use wyoming::{decode, encode_into, Event};

/// A minimal fake Wyoming service: speaks the protocol with the same crate
/// codec, no Python involved.
fn fake_server(listener: TcpListener) {
    let (socket, _peer) = listener.accept().expect("accept loopback");
    let mut reader = BufReader::new(socket.try_clone().expect("clone socket"));

    let write = |event: &Event| {
        let mut sock = socket.try_clone().expect("clone socket");
        encode_into(event, &mut sock).expect("server write");
        sock.flush().expect("server flush");
    };

    // 1. ping -> pong (copy text, like the Python handler).
    let frame = decode(&mut reader)
        .expect("server decode ping")
        .expect("ping frame");
    assert_eq!(frame.type_, "ping");
    write(
        &Pong {
            text: frame.get("text").and_then(Value::as_str).map(String::from),
        }
        .to_event(),
    );

    // 2. Wake detection pushed unsolicited (satellite-style).
    write(
        &Detection {
            name: Some("hey redox".into()),
            timestamp: Some(42),
            speaker: None,
            context: None,
        }
        .to_event(),
    );

    // 3. audio-start / audio-chunk+ / audio-stop -> transcript.
    let start = decode(&mut reader)
        .expect("server decode start")
        .expect("audio-start frame");
    assert_eq!(start.type_, "audio-start");
    assert_eq!(start.get("rate"), Some(&json!(16000)));

    let mut total_audio = 0usize;
    loop {
        let frame = decode(&mut reader)
            .expect("server decode chunk/stop")
            .expect("chunk or stop frame");
        match frame.type_.as_str() {
            "audio-chunk" => total_audio += frame.payload.as_deref().map_or(0, <[u8]>::len),
            "audio-stop" => break,
            other => panic!("server: unexpected frame while streaming: {other}"),
        }
    }
    assert_eq!(total_audio, 1024, "server saw the full audio stream");

    write(
        &Transcript {
            text: "turn on the lights".into(),
            language: Some("en".into()),
            context: None,
        }
        .to_event(),
    );

    // 4. Speech audio pushed back to the client.
    write(
        &AudioStart {
            rate: 22050,
            width: 2,
            channels: 1,
            timestamp: None,
        }
        .to_event(),
    );
    write(
        &AudioChunk {
            rate: 22050,
            width: 2,
            channels: 1,
            audio: vec![0x5a; 2048],
            timestamp: None,
        }
        .to_event(),
    );
    write(&AudioStop::default().to_event());

    // 5. Server keepalive ping; expect the client's auto-pong.
    write(&Ping { text: Some("hb".into()) }.to_event());
    let pong = decode(&mut reader)
        .expect("server decode auto-pong")
        .expect("pong frame");
    assert_eq!(pong.type_, "pong");
    assert_eq!(pong.get("text"), Some(&json!("hb")));

    // Function end drops every server-side descriptor: the client must
    // observe a clean EOF -> Closed.
}

/// Read one frame's worth of transitions (blocking). The fake server is
/// lock-step, so each `recv` here is deterministic: no nonblocking tricks.
fn step(client: &mut Client<TcpStream>) -> Vec<Transition> {
    client.recv().expect("recv")
}

#[test]
fn loopback_client_against_fake_server() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let addr = listener.local_addr().expect("local addr");
    let server = thread::spawn(move || fake_server(listener));

    let framed = wyoming::transport::connect(addr).expect("connect to fake server");
    let mut client = Client::new(framed.into_inner());

    let mut journal: Vec<Transition> = Vec::new();

    // --- ping / pong ------------------------------------------------------
    journal.push(client.send_ping(Some("hello".into())).expect("send ping"));
    journal.extend(step(&mut client)); // pong
    journal.extend(step(&mut client)); // unsolicited detection

    // --- audio start/chunk/stop -> transcript ------------------------------
    journal.push(client.send_audio_start(16000, 2, 1).expect("audio start"));
    journal.push(
        client
            .send_audio_chunk(16000, 2, 1, vec![0x11; 512])
            .expect("chunk 1"),
    );
    journal.push(
        client
            .send_audio_chunk(16000, 2, 1, vec![0x22; 512])
            .expect("chunk 2"),
    );
    journal.push(client.send_audio_stop().expect("audio stop"));
    journal.extend(step(&mut client)); // transcript

    // --- speech audio back --------------------------------------------------
    journal.extend(step(&mut client)); // speech audio-start
    journal.extend(step(&mut client)); // speech audio-chunk
    journal.extend(step(&mut client)); // speech audio-stop

    // --- server ping -> auto-pong, then clean EOF -> closed ------------------
    journal.extend(step(&mut client)); // received ping + auto-pong
    journal.extend(step(&mut client)); // EOF -> closed
    assert_eq!(client.state(), ClientState::Closed);

    server.join().expect("fake server thread");

    // Journal shape: every step is a Transition the caller can journal.
    let kinds: Vec<(String, String, String)> = journal
        .iter()
        .map(|t| {
            let type_ = match &t.kind {
                TransitionKind::Sent(e) | TransitionKind::Received(e) | TransitionKind::AutoPong(e) => {
                    e.type_.clone()
                }
                TransitionKind::Closed => String::new(),
            };
            (t.from.to_string(), t.to.to_string(), type_)
        })
        .collect();

    let expected: Vec<(String, String, String)> = [
        ("idle", "awaiting-pong", "ping"),
        ("awaiting-pong", "idle", "pong"),
        ("idle", "idle", "detection"),
        ("idle", "streaming-audio", "audio-start"),
        ("streaming-audio", "streaming-audio", "audio-chunk"),
        ("streaming-audio", "streaming-audio", "audio-chunk"),
        ("streaming-audio", "awaiting-response", "audio-stop"),
        ("awaiting-response", "idle", "transcript"),
        ("idle", "receiving-speech", "audio-start"),
        ("receiving-speech", "receiving-speech", "audio-chunk"),
        ("receiving-speech", "idle", "audio-stop"),
        ("idle", "idle", "ping"),
        ("idle", "idle", "ping"), // auto-pong entry carries the ping frame
        ("idle", "closed", ""),
    ]
    .into_iter()
    .map(|(from, to, type_)| (from.to_string(), to.to_string(), type_.to_string()))
    .collect();
    assert_eq!(kinds, expected, "full loopback journal");
    assert_eq!(journal.len(), 14);

    // Payload integrity through the socket.
    let speech_chunk = journal
        .iter()
        .find(|t| matches!(&t.kind, TransitionKind::Received(e) if e.type_ == "audio-chunk"))
        .expect("speech chunk journaled");
    match &speech_chunk.kind {
        TransitionKind::Received(e) => {
            assert_eq!(e.payload.as_deref().map(<[u8]>::len), Some(2048));
            assert_eq!(e.get("rate"), Some(&json!(22050)));
        }
        _ => unreachable!(),
    }

    // Transcript text survived the round trip.
    let transcript = journal
        .iter()
        .find_map(|t| match &t.kind {
            TransitionKind::Received(e) if e.type_ == "transcript" => Some(e.clone()),
            _ => None,
        })
        .expect("transcript journaled");
    assert_eq!(transcript.get("text"), Some(&json!("turn on the lights")));
}

#[test]
fn loopback_wire_bytes_match_golden_encoder() {
    // The fake server writes with `encode_into` — the same function the
    // golden cross-check proved byte-identical to Python `write_event`.
    // Assert the client's raw read of a server-written frame equals the
    // typed event, i.e. the transport adds no framing drift.
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    let server = thread::spawn(move || {
        let (mut socket, _) = listener.accept().expect("accept");
        let mut buf = Vec::new();
        encode_into(
            &Transcript {
                text: "golden-ish".into(),
                language: Some("en".into()),
                context: None,
            }
            .to_event(),
            &mut buf,
        )
        .expect("encode transcript frame");
        socket.write_all(&buf).expect("write frame");
        socket.flush().expect("flush");
    });

    let mut framed = wyoming::transport::connect(addr).expect("connect");
    let event = framed.read_frame().expect("read").expect("one frame");
    assert_eq!(event.type_, "transcript");
    assert_eq!(event.get("text"), Some(&json!("golden-ish")));
    assert_eq!(event.get("language"), Some(&json!("en")));
    server.join().expect("server thread");
}

/// Regression (Luna review): `Client::close()` must shut the transport down
/// so the peer observes EOF **while the Client is still alive** — not only
/// when the Client is dropped. Bounded: the server-side read carries a
/// 1.5 s timeout, so a missing EOF fails fast instead of hanging.
#[test]
fn close_signals_eof_to_peer_while_client_lives() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let addr = listener.local_addr().expect("local addr");

    let server = thread::spawn(move || {
        let (mut socket, _) = listener.accept().expect("accept");
        socket
            .set_read_timeout(Some(std::time::Duration::from_millis(1500)))
            .expect("read timeout");
        let mut buf = [0u8; 1];
        match socket.read(&mut buf) {
            Ok(0) => true, // EOF observed: client transport went away
            Ok(n) => panic!("unexpected {n} bytes from client after close"),
            Err(e) => {
                eprintln!("peer did NOT observe EOF while client lives: {e}");
                false
            }
        }
    });

    let framed = wyoming::transport::connect(addr).expect("connect loopback");
    let mut client = Client::new(framed.into_inner());

    let t = client.close();
    assert_eq!(t.from, ClientState::Idle);
    assert_eq!(t.to, ClientState::Closed);

    // The Client is deliberately still alive here (not dropped): the peer
    // must already see EOF without waiting for the drop.
    let eof_seen = server.join().expect("server thread");
    assert!(
        eof_seen,
        "regression: peer did not observe EOF while Client lives \
         (close() left the socket open)"
    );

    // Public API semantics preserved: closed state, idempotent close.
    assert_eq!(client.state(), ClientState::Closed);
    let again = client.close();
    assert_eq!(
        (again.from, again.to),
        (ClientState::Closed, ClientState::Closed)
    );
}
