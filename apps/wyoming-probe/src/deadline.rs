//! In-process bounded reads/writes for platforms where socket timeouts are
//! *accepted but not enforced* — proven on the E1 guest image: phase
//! wy-1792950931-guest1 armed `SO_RCVTIMEO=5s` (serial: "PROBE read_timeout
//! set 5s"), the peer stayed silent after receiving `describe`, and the
//! blocked read never woke (no `PROBE FAIL` inside the 15 s scan bound).
//!
//! Source hypothesis for the observed timeout gap (cached netstack c4f68cd;
//! installed netstack build identity has not been byte-verified):
//! `netstack/src/scheme/mod.rs`): a blocking read that returns EWOULDBLOCK
//! is parked on a per-connection `wait_queue` with an absolute `until`
//! (line 429+), and the expiry sweep that answers it with ETIMEDOUT
//! (lines 493+) only runs when the scheme's event loop iterates. A silent
//! peer produces no events, so the sweep never runs and the op hangs
//! forever. Nonblocking ops (`is_explicitly_nonblock()`) instead take the
//! synchronous EWOULDBLOCK return path, which the scheme answers
//! immediately — that is the primitive this adapter builds on.
//!
//! Mechanism: the socket is switched to nonblocking at construction (fail
//! closed if the platform refuses), and every read/write loops over
//! nonblocking `read`/`write` with a per-call wall-clock deadline measured
//! by `Instant`. A silent peer therefore yields a `TimedOut` error inside
//! the operation instead of an unbounded block in the scheme.
//!
//! Frame semantics are preserved: partial reads return as they arrive
//! (`wyoming::Framed`'s BufReader reassembles), `Ok(0)` is clean EOF, and
//! only deadline exhaustion synthesizes an error.

use std::io::{self, ErrorKind, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::thread;
use std::time::{Duration, Instant};

/// Sleep between nonblocking retries inside one bounded operation. Small
/// enough to keep reply latency low, large enough not to spin a core.
/// Shared with `diag` so the diagnostic exercises the same backoff the
/// adapter actually sleeps on.
pub(crate) const RETRY_BACKOFF: Duration = Duration::from_millis(10);

/// A `TcpStream` whose reads and writes are each bounded by an in-process
/// per-operation deadline, independent of any platform socket timeout.
pub struct DeadlineStream {
    stream: TcpStream,
    timeout: Duration,
}

impl DeadlineStream {
    /// Arm the adapter. `set_nonblocking` is the load-bearing primitive:
    /// if the platform will not make the socket nonblocking, this process
    /// cannot bound its own reads, so this fails closed rather than
    /// pretending to be bounded.
    pub fn new(stream: TcpStream, timeout: Duration) -> io::Result<Self> {
        stream.set_nonblocking(true)?;
        Ok(Self { stream, timeout })
    }
}

impl Read for DeadlineStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let until = Instant::now() + self.timeout;
        loop {
            // This fleet's forked std lacks TcpStream::try_read; on a
            // nonblocking socket plain read/write return WouldBlock.
            match self.stream.read(buf) {
                Ok(n) => return Ok(n),
                Err(e) if e.kind() == ErrorKind::WouldBlock => {
                    if Instant::now() >= until {
                        return Err(io::Error::new(
                            ErrorKind::TimedOut,
                            "read exceeded per-operation deadline (silent peer)",
                        ));
                    }
                    thread::sleep(RETRY_BACKOFF);
                }
                Err(e) => return Err(e),
            }
        }
    }
}

impl Write for DeadlineStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let until = Instant::now() + self.timeout;
        loop {
            match self.stream.write(buf) {
                Ok(n) => return Ok(n),
                Err(e) if e.kind() == ErrorKind::WouldBlock => {
                    if Instant::now() >= until {
                        return Err(io::Error::new(
                            ErrorKind::TimedOut,
                            "write exceeded per-operation deadline",
                        ));
                    }
                    thread::sleep(RETRY_BACKOFF);
                }
                Err(e) => return Err(e),
            }
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        self.stream.flush()
    }
}

impl wyoming::Sink for DeadlineStream {
    fn shutdown(&mut self) -> io::Result<()> {
        self.stream.shutdown(Shutdown::Both)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error as _;
    use std::net::TcpListener;

    /// Connected (adapter, peer) pair over loopback.
    fn pair(timeout: Duration) -> (DeadlineStream, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let client = TcpStream::connect(addr).expect("connect");
        let (peer, _) = listener.accept().expect("accept");
        (
            DeadlineStream::new(client, timeout).expect("arm adapter"),
            peer,
        )
    }

    #[test]
    fn silent_peer_read_times_out_within_deadline() {
        let (mut stream, _peer) = pair(Duration::from_millis(300));
        let started = Instant::now();
        let mut buf = [0u8; 16];
        let err = stream.read(&mut buf).expect_err("silent peer must error");
        let elapsed = started.elapsed();
        assert_eq!(err.kind(), ErrorKind::TimedOut, "must be TimedOut: {err}");
        assert!(
            elapsed >= Duration::from_millis(300),
            "fired early: {elapsed:?}"
        );
        assert!(elapsed < Duration::from_secs(5), "fired late: {elapsed:?}");
    }

    #[test]
    fn each_operation_gets_a_fresh_per_operation_deadline() {
        // Mirrors set_read_timeout semantics: consecutive blocked reads
        // each get the full timeout, not one shared budget.
        let (mut stream, _peer) = pair(Duration::from_millis(250));
        let mut buf = [0u8; 8];
        for _ in 0..2 {
            let started = Instant::now();
            assert_eq!(
                stream.read(&mut buf).expect_err("must time out").kind(),
                ErrorKind::TimedOut
            );
            assert!(
                started.elapsed() >= Duration::from_millis(250),
                "fired early"
            );
        }
    }

    #[test]
    fn bytes_arriving_mid_operation_return_without_waiting_for_the_deadline() {
        let (mut stream, peer) = pair(Duration::from_secs(5));
        let writer = thread::spawn(move || {
            let mut peer = peer;
            thread::sleep(Duration::from_millis(120));
            peer.write_all(b"describe\n").expect("peer write");
        });
        let started = Instant::now();
        let mut buf = [0u8; 32];
        let n = stream.read(&mut buf).expect("read must succeed");
        assert_eq!(&buf[..n], b"describe\n");
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "reply waited for deadline: {:?}",
            started.elapsed()
        );
        writer.join().expect("writer");
    }

    #[test]
    fn partial_bytes_reassemble_across_reads() {
        // Frame semantics preserved: the adapter never invents or drops
        // bytes; a split write reassembles through successive reads.
        let (mut stream, peer) = pair(Duration::from_secs(5));
        let writer = thread::spawn(move || {
            let mut peer = peer;
            peer.write_all(b"partial-").expect("part 1");
            thread::sleep(Duration::from_millis(80));
            peer.write_all(b"frame\n").expect("part 2");
        });
        let mut got = Vec::new();
        let mut buf = [0u8; 4];
        let started = Instant::now();
        while got != b"partial-frame\n" {
            if started.elapsed() > Duration::from_secs(5) {
                panic!("reassembly stalled: {got:?}");
            }
            let n = stream.read(&mut buf).expect("read");
            assert!(n > 0, "unexpected EOF");
            got.extend_from_slice(&buf[..n]);
        }
        writer.join().expect("writer");
    }

    #[test]
    fn clean_eof_is_zero_not_an_error() {
        let (mut stream, peer) = pair(Duration::from_secs(5));
        drop(peer);
        let mut buf = [0u8; 8];
        assert_eq!(stream.read(&mut buf).expect("EOF must be Ok(0)"), 0);
    }

    #[test]
    fn wyoming_frames_round_trip_through_the_adapter() {
        // End-to-end through wyoming::Client: a real describe frame from
        // the peer must decode unchanged over the adapter.
        use wyoming::events::info::Describe;
        use wyoming::events::Eventable;
        let (stream, mut peer) = pair(Duration::from_secs(5));
        let frame = wyoming::encode(&Describe.to_event());
        peer.write_all(&frame).expect("peer write frame");
        let mut client = wyoming::Client::new(stream);
        let transitions = client.recv().expect("recv must decode the frame");
        assert!(
            transitions.iter().any(|t| matches!(
                &t.kind,
                wyoming::TransitionKind::Received(e) if e.type_ == "describe"
            )),
            "describe frame lost through the adapter: {transitions:?}"
        );
    }

    #[test]
    fn silent_peer_makes_client_recv_fail_within_the_deadline() {
        // The leg-2 regression, host-side: peer accepts and never answers;
        // recv must surface a timeout error, not hang, inside the bound.
        let (stream, _peer) = pair(Duration::from_millis(300));
        let mut client = wyoming::Client::new(stream);
        let started = Instant::now();
        let err = client.recv().expect_err("silent peer must error");
        let elapsed = started.elapsed();
        let io = err
            .source()
            .and_then(|e| e.downcast_ref::<io::Error>())
            .expect("error must carry the io error");
        assert_eq!(io.kind(), ErrorKind::TimedOut, "must be TimedOut: {err}");
        assert!(elapsed < Duration::from_secs(5), "fired late: {elapsed:?}");
    }
}
