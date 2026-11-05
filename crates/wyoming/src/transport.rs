//! Socket transport adapter for the Wyoming frame codec.
//!
//! The codec in [`crate::event`] stays pure: `encode_into` works over any
//! [`std::io::Write`] and `decode` over any [`std::io::BufRead`], so it stays
//! host-testable without sockets. This module only wraps it:
//!
//! * [`Sink`] — a duplex byte channel (`Read + Write`) with a `shutdown`
//!   hook so closing a client signals EOF to the peer. Implemented for
//!   [`TcpStream`] and `UnixStream` (real shutdowns); in-memory mocks in
//!   tests get the default no-op.
//! * [`Framed`] — the codec over a [`Sink`]: `read_frame`/`write_frame`.
//! * [`connect`] — the TCP adapter: dial an address, set `TCP_NODELAY`
//!   (Wyoming streams latency-sensitive voice audio), and wrap the socket.

use std::io::{self, BufReader, Read, Write};
use std::net::{Shutdown, TcpStream, ToSocketAddrs};

use crate::error::Error;
use crate::event::{decode, encode_into, Event};

/// A duplex byte channel the Wyoming codec can be layered on.
///
/// Implemented for [`TcpStream`] and (on unix) `std::os::unix::net::UnixStream`
/// with real socket shutdowns; in-memory duplex mocks and anything else that
/// is readable and writable through one handle implement it with the default
/// no-op [`Sink::shutdown`].
pub trait Sink: Read + Write {
    /// Shut the channel down so the peer observes EOF.
    ///
    /// Default: no-op — in-memory channels have no peer to notify. Any
    /// real transport (socket, pipe, device) MUST override this, or
    /// `Client::close` will not deliver EOF to the peer (Cursor review
    /// W-7 footgun).
    fn shutdown(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Sink for TcpStream {
    fn shutdown(&mut self) -> io::Result<()> {
        TcpStream::shutdown(self, Shutdown::Both)
    }
}

#[cfg(unix)]
impl Sink for std::os::unix::net::UnixStream {
    fn shutdown(&mut self) -> io::Result<()> {
        std::os::unix::net::UnixStream::shutdown(self, Shutdown::Both)
    }
}

/// A Wyoming connection: the pure frame codec over a duplex [`Sink`].
///
/// The read side is buffered (frame reads mix line and exact-length block
/// reads, so buffering is required); the write side goes straight to the
/// underlying sink, which [`encode_into`] flushes per frame.
#[derive(Debug)]
pub struct Framed<S: Sink> {
    stream: BufReader<S>,
}

impl<S: Sink> Framed<S> {
    /// Wrap a duplex stream in the Wyoming framing codec.
    pub fn new(stream: S) -> Self {
        Framed {
            stream: BufReader::new(stream),
        }
    }

    /// Read the next frame. `Ok(None)` on a clean end of stream, exactly
    /// like [`decode`].
    pub fn read_frame(&mut self) -> Result<Option<Event>, Error> {
        decode(&mut self.stream)
    }

    /// Write one complete frame and flush.
    pub fn write_frame(&mut self, event: &Event) -> Result<(), Error> {
        encode_into(event, self.stream.get_mut())
    }

    /// Borrow the underlying stream.
    pub fn get_ref(&self) -> &S {
        self.stream.get_ref()
    }

    /// Mutably borrow the underlying stream.
    pub fn get_mut(&mut self) -> &mut S {
        self.stream.get_mut()
    }

    /// Unwrap the underlying stream, dropping the read buffer.
    pub fn into_inner(self) -> S {
        self.stream.into_inner()
    }

    /// Flush pending writes, then [`Sink::shutdown`] the channel so the
    /// peer observes EOF.
    ///
    /// The shutdown is attempted even when the flush fails: a peer that
    /// never received the last frame must still see EOF (Cursor review
    /// W-8). The first error is returned.
    pub fn shutdown(&mut self) -> io::Result<()> {
        let flushed = self.stream.get_mut().flush();
        let shutdown = self.stream.get_mut().shutdown();
        flushed.and(shutdown)
    }
}

/// Connect to a Wyoming service over TCP and wrap the socket in [`Framed`].
///
/// Sets `TCP_NODELAY`: Wyoming frames are small and latency-sensitive
/// (voice audio chunks, keepalive pings), so Nagle batching is harmful.
pub fn connect<A: ToSocketAddrs>(addr: A) -> Result<Framed<TcpStream>, Error> {
    let stream = TcpStream::connect(addr)?;
    stream.set_nodelay(true)?;
    Ok(Framed::new(stream))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::net::TcpListener;
    use std::thread;

    /// In-memory duplex mock: reads come from `input`, writes land in
    /// `output`. Proves the codec runs over a non-socket [`Sink`].
    struct Duplex {
        input: Cursor<Vec<u8>>,
        output: Vec<u8>,
    }

    impl Duplex {
        fn new(input: impl Into<Vec<u8>>) -> Self {
            Duplex {
                input: Cursor::new(input.into()),
                output: Vec::new(),
            }
        }
    }

    impl Read for Duplex {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.input.read(buf)
        }
    }

    impl Write for Duplex {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.output.write(buf)
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    // In-memory mock: default no-op shutdown (no peer to notify).
    impl Sink for Duplex {}

    /// Mock whose `flush` always fails but records `shutdown` (review W-8).
    struct FlushFails {
        inner: Duplex,
        shutdown_called: bool,
    }

    impl Read for FlushFails {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.inner.read(buf)
        }
    }

    impl Write for FlushFails {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.inner.write(buf)
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Err(std::io::Error::other("flush boom"))
        }
    }

    impl Sink for FlushFails {
        fn shutdown(&mut self) -> std::io::Result<()> {
            self.shutdown_called = true;
            Ok(())
        }
    }

    /// Review W-8: a failing flush must not short-circuit the shutdown —
    /// the peer must still observe EOF; the flush error is still returned.
    #[test]
    fn shutdown_runs_even_when_flush_fails() {
        let mut framed = Framed::new(FlushFails {
            inner: Duplex::new(Vec::new()),
            shutdown_called: false,
        });
        let err = framed.shutdown().expect_err("flush error must surface");
        assert_eq!(err.to_string(), "flush boom");
        assert!(
            framed.get_mut().shutdown_called,
            "shutdown must run even when the flush fails"
        );
    }

    fn sample_event() -> Event {
        Event::new("audio-chunk")
            .with_data({
                let mut data = serde_json::Map::new();
                data.insert("rate".to_string(), serde_json::json!(16000));
                data
            })
            .with_payload(b"test\npayload".to_vec())
    }

    #[test]
    fn mock_sink_roundtrip() {
        let frame = encode_frame(&sample_event());
        let mut framed = Framed::new(Duplex::new(frame.clone()));

        let got = framed.read_frame().expect("read ok").expect("one frame");
        assert_eq!(got, sample_event());
        assert_eq!(framed.read_frame().expect("read ok"), None);

        framed.write_frame(&sample_event()).expect("write ok");
        assert_eq!(framed.into_inner().output, frame);
    }

    #[test]
    fn mock_sink_multiple_frames() {
        let mut bytes = Vec::new();
        bytes.extend(encode_frame(&Event::new("ping")));
        bytes.extend(encode_frame(&sample_event()));
        bytes.extend(encode_frame(&Event::new("pong")));

        let mut framed = Framed::new(Duplex::new(bytes));
        let mut types = Vec::new();
        while let Some(event) = framed.read_frame().expect("read ok") {
            types.push(event.type_);
        }
        assert_eq!(types, ["ping", "audio-chunk", "pong"]);
    }

    #[test]
    fn tcp_loopback_roundtrip() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let addr = listener.local_addr().expect("local addr");

        let server = thread::spawn(move || {
            let (socket, _) = listener.accept().expect("accept");
            let mut framed = Framed::new(socket);
            // Echo one frame back, then close.
            let event = framed.read_frame().expect("server read").expect("one frame");
            framed.write_frame(&event).expect("server write");
        });

        let mut client = connect(addr).expect("connect loopback");
        client.write_frame(&sample_event()).expect("client write");
        let got = client.read_frame().expect("client read").expect("one frame");
        assert_eq!(got, sample_event());
        // Server closed: clean EOF.
        assert_eq!(client.read_frame().expect("client read"), None);

        server.join().expect("server thread");
    }

    fn encode_frame(event: &Event) -> Vec<u8> {
        let mut buf = Vec::new();
        encode_into(event, &mut buf).expect("encode");
        buf
    }
}
