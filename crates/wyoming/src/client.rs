//! Wyoming client state machine over a [`Sink`] transport.
//!
//! Models the control-plane event flow from the design doc
//! (`docs/design/WAVE2-NETSANDBOX.md` §1.1): send `Ping` and audio
//! `Start → Chunk → Stop`, receive `Pong` / `Transcript` / speech audio
//! (`AudioStart`/`AudioChunk`/`AudioStop` from the service) / wake
//! `Detection`. The connection state is an explicit [`ClientState`] enum;
//! **every** transition returns a [`Transition`] record — the from/to states
//! plus the frame involved — so the caller can journal each step (B1
//! tracejournal hook) without re-parsing the wire.
//!
//! Unknown or out-of-band frames never kill the connection: they are
//! journaled as received with no state change, and it is the caller's call
//! what to do with them. Commands issued out of order are rejected with
//! [`Error::InvalidState`] and change nothing.

use std::fmt;

use serde_json::Value;

use crate::error::Error;
use crate::event::Event;
use crate::events::audio::{AudioChunk, AudioStart, AudioStop};
use crate::events::asr::Transcript;
use crate::events::ping::{Ping, Pong};
use crate::events::wake::Detection;
use crate::events::Eventable;
use crate::transport::{Framed, Sink};

/// Explicit connection state of the client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientState {
    /// Connected, no interaction in progress.
    Idle,
    /// A `ping` was sent; the next `pong` closes the keepalive.
    AwaitingPong,
    /// An `audio-start` was sent; chunks may follow until `audio-stop`.
    StreamingAudio,
    /// An `audio-stop` was sent; waiting for the service's response
    /// (typically a `transcript`, possibly speech audio).
    AwaitingResponse,
    /// The service started sending audio (speech): `audio-start` received,
    /// chunks until the service's `audio-stop`.
    ReceivingSpeech,
    /// The connection is closed (clean EOF or [`Client::close`]).
    Closed,
}

impl fmt::Display for ClientState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            ClientState::Idle => "idle",
            ClientState::AwaitingPong => "awaiting-pong",
            ClientState::StreamingAudio => "streaming-audio",
            ClientState::AwaitingResponse => "awaiting-response",
            ClientState::ReceivingSpeech => "receiving-speech",
            ClientState::Closed => "closed",
        };
        f.write_str(name)
    }
}

/// What happened in a transition — the journalable payload.
#[derive(Debug, Clone, PartialEq)]
pub enum TransitionKind {
    /// A frame the client sent to the service.
    Sent(Event),
    /// A frame received from the service.
    Received(Event),
    /// A `pong` the client sent automatically in reply to a service `ping`
    /// (carries the received `ping` frame; the pong itself is
    /// `Pong { text }` with the ping's text copied).
    AutoPong(Event),
    /// The connection ended (clean EOF or explicit close).
    Closed,
}

/// One state-machine step, returned by every client action so the caller
/// can journal it (B1 tracejournal hook).
#[derive(Debug, Clone, PartialEq)]
pub struct Transition {
    /// State before the transition.
    pub from: ClientState,
    /// State after the transition.
    pub to: ClientState,
    /// What happened.
    pub kind: TransitionKind,
}

impl Transition {
    /// One-line journalable summary, e.g.
    /// `idle -> awaiting-pong: sent ping`.
    pub fn summary(&self) -> String {
        let (verb, type_) = match &self.kind {
            TransitionKind::Sent(event) => ("sent", event.type_.as_str()),
            TransitionKind::Received(event) => ("received", event.type_.as_str()),
            TransitionKind::AutoPong(event) => ("auto-pong for", event.type_.as_str()),
            TransitionKind::Closed => ("closed", ""),
        };
        if type_.is_empty() {
            format!("{} -> {}: {verb}", self.from, self.to)
        } else {
            format!("{} -> {}: {verb} {type_}", self.from, self.to)
        }
    }
}

/// A Wyoming client: the [`Framed`] codec plus the explicit state machine.
#[derive(Debug)]
pub struct Client<S: Sink> {
    framed: Framed<S>,
    state: ClientState,
}

impl<S: Sink> Client<S> {
    /// Wrap a connected duplex stream in the client state machine.
    pub fn new(stream: S) -> Self {
        Client {
            framed: Framed::new(stream),
            state: ClientState::Idle,
        }
    }

    /// Current state.
    pub fn state(&self) -> ClientState {
        self.state
    }

    /// Borrow the framed transport (for raw frame access if needed).
    pub fn framed(&self) -> &Framed<S> {
        &self.framed
    }

    /// Close the connection: the transport is flushed and shut down, so
    /// the peer observes EOF immediately — even while this `Client` is
    /// still alive. Idempotent: closing an already-closed client returns
    /// a `closed -> closed` transition.
    pub fn close(&mut self) -> Transition {
        let from = self.state;
        self.state = ClientState::Closed;
        // Best-effort: a dead peer or a double close must not change the
        // journal — the transition is returned either way.
        let _ = self.framed.shutdown();
        Transition {
            from,
            to: ClientState::Closed,
            kind: TransitionKind::Closed,
        }
    }

    /// Send a `ping` (keepalive). From `Idle` this moves to `AwaitingPong`;
    /// from any other open state the state is untouched (the pong is then
    /// journaled without a state change).
    pub fn send_ping(&mut self, text: Option<String>) -> Result<Transition, Error> {
        self.require_open("send_ping")?;
        let from = self.state;
        let event = Ping { text }.to_event();
        self.framed.write_frame(&event)?;
        self.state = if from == ClientState::Idle {
            ClientState::AwaitingPong
        } else {
            from
        };
        Ok(Transition {
            from,
            to: self.state,
            kind: TransitionKind::Sent(event),
        })
    }

    /// Send `audio-start`, opening an audio stream to the service.
    /// Only valid from `Idle`.
    pub fn send_audio_start(
        &mut self,
        rate: u32,
        width: u8,
        channels: u8,
    ) -> Result<Transition, Error> {
        self.require(ClientState::Idle, "send_audio_start")?;
        let from = self.state;
        let event = AudioStart {
            rate,
            width,
            channels,
            timestamp: None,
        }
        .to_event();
        self.framed.write_frame(&event)?;
        self.state = ClientState::StreamingAudio;
        Ok(Transition {
            from,
            to: self.state,
            kind: TransitionKind::Sent(event),
        })
    }

    /// Send one `audio-chunk`. Only valid while `StreamingAudio`.
    pub fn send_audio_chunk(
        &mut self,
        rate: u32,
        width: u8,
        channels: u8,
        audio: Vec<u8>,
    ) -> Result<Transition, Error> {
        self.require(ClientState::StreamingAudio, "send_audio_chunk")?;
        let from = self.state;
        let event = AudioChunk {
            rate,
            width,
            channels,
            audio,
            timestamp: None,
        }
        .to_event();
        self.framed.write_frame(&event)?;
        Ok(Transition {
            from,
            to: from,
            kind: TransitionKind::Sent(event),
        })
    }

    /// Send `audio-stop`, ending the audio stream. Only valid while
    /// `StreamingAudio`; moves to `AwaitingResponse`.
    pub fn send_audio_stop(&mut self) -> Result<Transition, Error> {
        self.require(ClientState::StreamingAudio, "send_audio_stop")?;
        let from = self.state;
        let event = AudioStop::default().to_event();
        self.framed.write_frame(&event)?;
        self.state = ClientState::AwaitingResponse;
        Ok(Transition {
            from,
            to: self.state,
            kind: TransitionKind::Sent(event),
        })
    }

    /// Send an arbitrary raw frame (escape hatch for events the typed
    /// helpers do not cover, e.g. `synthesize`). No state change.
    pub fn send(&mut self, event: &Event) -> Result<Transition, Error> {
        self.require_open("send")?;
        let from = self.state;
        self.framed.write_frame(event)?;
        Ok(Transition {
            from,
            to: from,
            kind: TransitionKind::Sent(event.clone()),
        })
    }

    /// Read the next frame(s) from the service and apply the state machine.
    ///
    /// Returns one [`Transition`] per journalable step: normally one, but a
    /// service `ping` yields two (`Received` + `AutoPong`, since the client
    /// answers it automatically). A clean EOF moves to `Closed`.
    pub fn recv(&mut self) -> Result<Vec<Transition>, Error> {
        if self.state == ClientState::Closed {
            return Err(Error::InvalidState("recv on a closed connection".to_string()));
        }
        let from = self.state;
        let frame = match self.framed.read_frame()? {
            None => {
                self.state = ClientState::Closed;
                // Peer is gone; release our side of the channel too.
                let _ = self.framed.shutdown();
                return Ok(vec![Transition {
                    from,
                    to: ClientState::Closed,
                    kind: TransitionKind::Closed,
                }]);
            }
            Some(frame) => frame,
        };

        let mut out = Vec::new();
        match frame.type_.as_str() {
            t if t == Pong::TYPE => {
                // Pong closes the keepalive only when one is outstanding.
                self.state = if self.state == ClientState::AwaitingPong {
                    ClientState::Idle
                } else {
                    self.state
                };
                out.push(Transition {
                    from,
                    to: self.state,
                    kind: TransitionKind::Received(frame),
                });
            }
            t if t == Transcript::TYPE => {
                // Transcript completes the service's response.
                self.state = if self.state == ClientState::AwaitingResponse {
                    ClientState::Idle
                } else {
                    self.state
                };
                out.push(Transition {
                    from,
                    to: self.state,
                    kind: TransitionKind::Received(frame),
                });
            }
            t if t == AudioStart::TYPE => {
                // audio-start from the service = speech audio coming in.
                if self.state == ClientState::Idle || self.state == ClientState::AwaitingResponse {
                    self.state = ClientState::ReceivingSpeech;
                }
                out.push(Transition {
                    from,
                    to: self.state,
                    kind: TransitionKind::Received(frame),
                });
            }
            t if t == AudioChunk::TYPE => {
                // Speech chunk; state unchanged while receiving speech.
                out.push(Transition {
                    from,
                    to: self.state,
                    kind: TransitionKind::Received(frame),
                });
            }
            t if t == AudioStop::TYPE => {
                // Service's audio-stop ends the speech stream.
                self.state = if self.state == ClientState::ReceivingSpeech {
                    ClientState::Idle
                } else {
                    self.state
                };
                out.push(Transition {
                    from,
                    to: self.state,
                    kind: TransitionKind::Received(frame),
                });
            }
            t if t == Detection::TYPE => {
                // Wake word detection: journalable, no state change.
                out.push(Transition {
                    from,
                    to: self.state,
                    kind: TransitionKind::Received(frame),
                });
            }
            t if t == Ping::TYPE => {
                // Service keepalive ping: journal it and answer with a pong
                // (copying `text`, like the Python client handler).
                out.push(Transition {
                    from,
                    to: self.state,
                    kind: TransitionKind::Received(frame.clone()),
                });
                let text = frame
                    .get("text")
                    .and_then(Value::as_str)
                    .map(String::from);
                let pong = Pong { text }.to_event();
                self.framed.write_frame(&pong)?;
                out.push(Transition {
                    from: self.state,
                    to: self.state,
                    kind: TransitionKind::AutoPong(frame),
                });
            }
            _ => {
                // Unknown or out-of-vocabulary frame: journal, no change.
                out.push(Transition {
                    from,
                    to: self.state,
                    kind: TransitionKind::Received(frame),
                });
            }
        }
        Ok(out)
    }

    fn require_open(&self, what: &str) -> Result<(), Error> {
        if self.state == ClientState::Closed {
            return Err(Error::InvalidState(format!(
                "{what} not allowed: connection is closed"
            )));
        }
        Ok(())
    }

    fn require(&self, want: ClientState, what: &str) -> Result<(), Error> {
        self.require_open(what)?;
        if self.state != want {
            return Err(Error::InvalidState(format!(
                "{what} not allowed in state {self_state} (requires {want})",
                self_state = self.state
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::error::ErrorEvent;
    use std::io::{Cursor, Read, Write};

    /// Mock duplex sink: frames fed in `input`, frames captured in `output`.
    struct MockSink {
        input: Cursor<Vec<u8>>,
        output: Vec<u8>,
        shutdown_called: bool,
    }

    impl MockSink {
        fn new() -> Self {
            MockSink {
                input: Cursor::new(Vec::new()),
                output: Vec::new(),
                shutdown_called: false,
            }
        }

        fn feed(&mut self, event: &Event) {
            let mut buf = Vec::new();
            crate::encode_into(event, &mut buf).expect("encode");
            self.input.get_mut().extend(buf);
        }

        fn written(&self) -> Vec<Event> {
            let mut reader = Cursor::new(self.output.clone());
            let mut events = Vec::new();
            while let Some(event) = crate::decode(&mut reader).expect("decode written frame") {
                events.push(event);
            }
            events
        }
    }

    impl Read for MockSink {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.input.read(buf)
        }
    }

    impl Write for MockSink {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.output.write(buf)
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl Sink for MockSink {
        fn shutdown(&mut self) -> std::io::Result<()> {
            self.shutdown_called = true;
            Ok(())
        }
    }

    fn client() -> Client<MockSink> {
        Client::new(MockSink::new())
    }

    #[test]
    fn ping_pong_cycle() {
        let mut c = client();
        let t = c.send_ping(Some("hi".into())).expect("ping");
        assert_eq!((t.from, t.to), (ClientState::Idle, ClientState::AwaitingPong));
        assert_eq!(c.state(), ClientState::AwaitingPong);

        // The ping frame must be on the wire with its text.
        let sent = c.framed.get_ref().written();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].type_, "ping");
        assert_eq!(sent[0].get("text"), Some(&Value::String("hi".into())));

        // Feed a pong back.
        c.framed.get_mut().feed(&Pong { text: Some("hi".into()) }.to_event());
        let ts = c.recv().expect("recv");
        assert_eq!(ts.len(), 1);
        assert_eq!(ts[0].from, ClientState::AwaitingPong);
        assert_eq!(ts[0].to, ClientState::Idle);
        assert_eq!(c.state(), ClientState::Idle);
    }

    #[test]
    fn audio_stream_then_transcript() {
        let mut c = client();
        let t = c.send_audio_start(16000, 2, 1).expect("start");
        assert_eq!((t.from, t.to), (ClientState::Idle, ClientState::StreamingAudio));

        let t = c
            .send_audio_chunk(16000, 2, 1, vec![0u8; 512])
            .expect("chunk");
        assert_eq!((t.from, t.to), (ClientState::StreamingAudio, ClientState::StreamingAudio));
        let t = c
            .send_audio_chunk(16000, 2, 1, vec![1u8; 512])
            .expect("chunk 2");
        assert_eq!(t.to, ClientState::StreamingAudio);

        let t = c.send_audio_stop().expect("stop");
        assert_eq!((t.from, t.to), (ClientState::StreamingAudio, ClientState::AwaitingResponse));

        // Wire check: start, 2 chunks with payloads, stop.
        let sent = c.framed.get_ref().written();
        let types: Vec<&str> = sent.iter().map(|e| e.type_.as_str()).collect();
        assert_eq!(types, ["audio-start", "audio-chunk", "audio-chunk", "audio-stop"]);
        assert_eq!(sent[1].payload.as_deref().map(<[u8]>::len), Some(512));

        // Transcript completes the interaction.
        c.framed.get_mut().feed(
            &Transcript {
                text: "turn on the lights".into(),
                language: None,
                context: None,
            }
            .to_event(),
        );
        let ts = c.recv().expect("recv");
        assert_eq!(ts.len(), 1);
        assert_eq!(ts[0].from, ClientState::AwaitingResponse);
        assert_eq!(ts[0].to, ClientState::Idle);
        assert_eq!(c.state(), ClientState::Idle);
    }

    #[test]
    fn speech_audio_stream() {
        let mut c = client();
        // Service pushes speech audio.
        c.framed.get_mut().feed(
            &AudioStart {
                rate: 22050,
                width: 2,
                channels: 1,
                timestamp: None,
            }
            .to_event(),
        );
        let ts = c.recv().expect("recv start");
        assert_eq!(ts[0].to, ClientState::ReceivingSpeech);

        c.framed
            .get_mut()
            .feed(&AudioChunk {
                rate: 22050,
                width: 2,
                channels: 1,
                audio: vec![7u8; 1024],
                timestamp: None,
            }.to_event());
        let ts = c.recv().expect("recv chunk");
        assert_eq!(ts[0].to, ClientState::ReceivingSpeech);
        assert_eq!(
            ts[0].kind,
            TransitionKind::Received(
                AudioChunk {
                    rate: 22050,
                    width: 2,
                    channels: 1,
                    audio: vec![7u8; 1024],
                    timestamp: None,
                }
                .to_event()
            )
        );

        c.framed.get_mut().feed(&AudioStop::default().to_event());
        let ts = c.recv().expect("recv stop");
        assert_eq!(ts[0].to, ClientState::Idle);
        assert_eq!(c.state(), ClientState::Idle);
    }

    #[test]
    fn wake_detection_journaled_without_state_change() {
        let mut c = client();
        c.framed.get_mut().feed(
            &Detection {
                name: Some("hey mycroft".into()),
                timestamp: Some(1234),
                speaker: None,
                context: None,
            }
            .to_event(),
        );
        let ts = c.recv().expect("recv detection");
        assert_eq!(ts.len(), 1);
        assert_eq!(ts[0].from, ClientState::Idle);
        assert_eq!(ts[0].to, ClientState::Idle);
        assert_eq!(ts[0].kind, TransitionKind::Received(
            Detection {
                name: Some("hey mycroft".into()),
                timestamp: Some(1234),
                speaker: None,
                context: None,
            }
            .to_event()
        ));
    }

    #[test]
    fn service_ping_gets_auto_pong() {
        let mut c = client();
        c.framed.get_mut().feed(&Ping { text: Some("hb".into()) }.to_event());
        let ts = c.recv().expect("recv ping");
        assert_eq!(ts.len(), 2);
        assert_eq!(ts[0].kind, TransitionKind::Received(Ping { text: Some("hb".into()) }.to_event()));
        assert_eq!(ts[1].kind, TransitionKind::AutoPong(Ping { text: Some("hb".into()) }.to_event()));
        assert_eq!(c.state(), ClientState::Idle);

        // The pong went out with the text copied.
        let sent = c.framed.get_ref().written();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].type_, "pong");
        assert_eq!(sent[0].get("text"), Some(&Value::String("hb".into())));
    }

    #[test]
    fn out_of_order_commands_rejected() {
        let mut c = client();
        // Chunk before start.
        let err = c
            .send_audio_chunk(16000, 2, 1, vec![0u8])
            .expect_err("chunk before start must fail");
        assert!(matches!(&err, Error::InvalidState(_)));
        // Stop before start.
        let err = c.send_audio_stop().expect_err("stop before start must fail");
        assert!(matches!(&err, Error::InvalidState(_)));
        // Double start.
        c.send_audio_start(16000, 2, 1).expect("start");
        let err = c.send_audio_start(16000, 2, 1).expect_err("double start must fail");
        assert!(matches!(&err, Error::InvalidState(_)));
        // Nothing was written for the rejected commands.
        let sent = c.framed.get_ref().written();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].type_, "audio-start");
    }

    #[test]
    fn clean_eof_closes() {
        let mut c = client();
        let ts = c.recv().expect("recv on empty input");
        assert_eq!(ts.len(), 1);
        assert_eq!(ts[0].to, ClientState::Closed);
        assert_eq!(c.state(), ClientState::Closed);
        // Further actions are rejected.
        assert!(matches!(
            &c.send_ping(None).expect_err("ping after close"),
            Error::InvalidState(_)
        ));
        assert!(matches!(
            &c.recv().expect_err("recv after close"),
            Error::InvalidState(_)
        ));
    }

    #[test]
    fn close_is_idempotent() {
        let mut c = client();
        let t = c.close();
        assert_eq!((t.from, t.to), (ClientState::Idle, ClientState::Closed));
        let t = c.close();
        assert_eq!((t.from, t.to), (ClientState::Closed, ClientState::Closed));
    }

    #[test]
    fn close_shuts_down_transport() {
        // Regression companion: close() must hand the sink a shutdown so
        // the peer observes EOF while the Client is still alive.
        let mut c = client();
        c.close();
        assert!(
            c.framed().get_ref().shutdown_called,
            "close() did not shut down the transport"
        );
    }

    #[test]
    fn eof_also_shuts_down_transport() {
        let mut c = client();
        let ts = c.recv().expect("recv on empty input");
        assert_eq!(ts[0].to, ClientState::Closed);
        assert!(
            c.framed().get_ref().shutdown_called,
            "EOF transition did not shut down the transport"
        );
    }

    #[test]
    fn unknown_and_error_frames_journal_only() {
        let mut c = client();
        c.framed.get_mut().feed(&Event::new("some-future-event"));
        let ts = c.recv().expect("recv unknown");
        assert_eq!(ts.len(), 1);
        assert_eq!(ts[0].from, ClientState::Idle);
        assert_eq!(ts[0].to, ClientState::Idle);

        c.framed.get_mut().feed(
            &ErrorEvent {
                code: Some("asr-error".into()),
                text: "boom".into(),
            }
            .to_event(),
        );
        let ts = c.recv().expect("recv error");
        assert_eq!(ts.len(), 1);
        assert_eq!(ts[0].to, ClientState::Idle);
        assert_eq!(c.state(), ClientState::Idle);
    }

    #[test]
    fn transitions_are_journalable_summaries() {
        let mut c = client();
        let t = c.send_ping(None).expect("ping");
        assert_eq!(t.summary(), "idle -> awaiting-pong: sent ping");
        c.framed.get_mut().feed(&Pong { text: None }.to_event());
        let ts = c.recv().expect("recv");
        assert_eq!(ts[0].summary(), "awaiting-pong -> idle: received pong");
        let t = c.close();
        assert_eq!(t.summary(), "idle -> closed: closed");
    }
}
