//! E1 Wyoming probe: the smallest native Wyoming client that proves the
//! `crates/wyoming` crate works end-to-end over a real TCP socket.
//!
//! Usage: `wyoming-probe <numeric-ip:port> [nonce] [deadline_secs]`
//! Opt-in raw-I/O diagnostic (OFF by default, protocol untouched):
//! `wyoming-probe diag <numeric-ip:port> [timeout_secs] [max_retries]`
//!
//! SCOPE OF THE PEER: the host smoke uses the pinned Python reference
//! **echo fixture** (`crates/wyoming/tests/pyinterop_server.py`), which
//! echoes every non-`ping` frame back and answers `ping` with a real
//! `Pong` (text copied). Legs 1 and 2 therefore prove the typed codec
//! round-trips through the reference encoder/decoder — they are NOT proof
//! that a service answers `describe` with `info`. A normal Wyoming service
//! does answer `describe` with an `info` frame; this probe deliberately
//! does not depend on that, because the fixture available here does not.
//!
//! Legs, in order:
//! 1. `describe` — typed `Describe` out; the echo fixture sends the same
//!    frame back, re-parsed with `Describe::from_event`.
//! 2. `info`     — typed `Info` (one wake program) out; the fixture sends
//!    it back, re-parsed with `Info::from_event` and compared.
//! 3. nonce      — `ping { text: <nonce> }`; the fixture's `pong` copies
//!    the text, so the nonce must come back verbatim. Real
//!    request/response, not an echo.
//!
//! BOUNDING — honest about what bounds what:
//! * connect is bounded by `TcpStream::connect_timeout` on a numeric
//!   `SocketAddr` (no resolver, so no unbounded DNS step);
//! * each read and write is bounded PER OPERATION by the in-process
//!   `deadline::DeadlineStream` adapter (nonblocking socket + `Instant`
//!   deadline). Platform socket timeouts are NOT trusted: phase
//!   wy-1792950931-guest1 PROVED the E1 guest accepts `SO_RCVTIMEO` yet
//!   never wakes a blocked read from a silent peer (see `deadline.rs`
//!   for the netstack source mechanism). `SO_RCVTIMEO`/`SO_SNDTIMEO` are
//!   still requested as defense in depth, but their absence is not fatal
//!   and their presence is not proof.
//! * the wall-clock deadline is COOPERATIVE: it is checked between frames,
//!   so a sequence of slow-but-succeeding partial operations can run past
//!   it; the per-operation timeouts cap each one, not the whole process.
//! * the hard whole-process lifetime bound is therefore EXTERNAL: the
//!   harness runs the client under `timeout --kill-after`.
//! * `TCP_NODELAY` is attempted but never claimed: on the E1 guest relibc
//!   520d881 `setsockopt` falls through to `todo_skip!` for IPPROTO_TCP
//!   options and returns Ok (serial of wy-1792950931-guest1 shows the
//!   TODO line at socket.rs:1013), so it is a silent no-op there.
//! * the overall deadline and the per-leg frame budget still bound the
//!   number of round trips once reads do return.
//!
//! Exits 0 on full pass, 1 on any failure/timeout/unsupported primitive,
//! 2 on bad usage.

mod deadline;
mod diag;
mod diag_cli;

use deadline::DeadlineStream;
use std::error::Error as _;
use std::io::ErrorKind;
use std::net::{SocketAddr, TcpStream};
use std::process;
use std::time::{Duration, Instant};

use wyoming::events::info::{Attribution, Describe, Info, WakeProgram};
use wyoming::events::ping::Pong;
use wyoming::events::Eventable;
use wyoming::{Client, Transition, TransitionKind};

/// Default cooperative deadline for the exchange, seconds.
const DEFAULT_DEADLINE_SECS: u64 = 10;
/// Hard cap on frames consumed by one `recv_of_type` leg.
const MAX_FRAMES_PER_LEG: u32 = 32;

/// One journalable step, printed as it happens.
fn journal(t: &Transition) {
    println!("PROBE frame {}", t.summary());
}

/// True when an error is a socket timeout (a read that hit its timeout).
fn is_timeout(err: &wyoming::Error) -> bool {
    err.source()
        .and_then(|e| e.downcast_ref::<std::io::Error>())
        .is_some_and(|io| matches!(io.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut))
}

/// Bounded receive: read frames until one has the wanted type. Reads are
/// bounded PER OPERATION by the socket timeout armed in `dial`; the
/// deadline is a cooperative check between frames and the frame budget
/// caps the loop. `Ok(None)` = peer closed before answering.
fn recv_of_type(
    client: &mut Client<DeadlineStream>,
    want: &str,
    deadline: Instant,
) -> Result<Option<wyoming::Event>, String> {
    let mut frames = 0u32;
    loop {
        if Instant::now() >= deadline {
            return Err(format!("{want}: deadline exceeded before the reply"));
        }
        let transitions = match client.recv() {
            Ok(t) => t,
            Err(e) if is_timeout(&e) => {
                return Err(format!("{want}: socket read timed out (silent peer)"))
            }
            Err(e) => return Err(format!("recv {want}: {e}")),
        };
        for t in &transitions {
            journal(t);
            if let TransitionKind::Received(event) = &t.kind {
                frames += 1;
                if event.type_ == want {
                    return Ok(Some(event.clone()));
                }
                if frames >= MAX_FRAMES_PER_LEG {
                    return Err(format!(
                        "{want}: frame budget exceeded ({MAX_FRAMES_PER_LEG} frames, no match)"
                    ));
                }
            }
            if let TransitionKind::Closed = &t.kind {
                return Ok(None);
            }
        }
    }
}

/// Dial with a bounded connect, then arm the in-process deadline adapter.
/// `set_nonblocking` is REQUIRED (fail closed): it is the only primitive
/// this process can interrupt its own reads with. Platform socket timeouts
/// are requested as defense in depth only — the E1 guest accepted them
/// and still hung on a silent peer (leg 2 of wy-1792950931-guest1).
fn dial(addr: SocketAddr, timeout: Duration) -> Result<DeadlineStream, String> {
    let stream = TcpStream::connect_timeout(&addr, timeout)
        .map_err(|e| format!("connect_timeout {addr} in {}s: {e}", timeout.as_secs()))?;
    println!("PROBE connect_timeout ok {}s", timeout.as_secs());
    // Known silent no-op on the E1 guest (relibc todo_skip returns Ok);
    // attempted for platforms that do implement it, never claimed.
    match stream.set_nodelay(true) {
        Ok(()) => println!("PROBE tcp_nodelay attempted (may be a no-op on Redox)"),
        Err(e) => println!("PROBE tcp_nodelay unsupported: {e}"),
    }
    // Defense in depth, not the bound: absence is not fatal, presence is
    // not proof (see deadline.rs for the netstack expiry-sweep gap).
    match stream.set_read_timeout(Some(timeout)) {
        Ok(()) => println!(
            "PROBE read_timeout requested {}s (not trusted)",
            timeout.as_secs()
        ),
        Err(e) => println!("PROBE read_timeout unsupported: {e}"),
    }
    match stream.set_write_timeout(Some(timeout)) {
        Ok(()) => println!(
            "PROBE write_timeout requested {}s (not trusted)",
            timeout.as_secs()
        ),
        Err(e) => println!("PROBE write_timeout unsupported: {e}"),
    }
    // The real bound. Fail closed if nonblocking is unavailable: running
    // unbounded is exactly the leg-2 failure this patch exists to fix.
    let stream = DeadlineStream::new(stream, timeout).map_err(|e| {
        format!("FATAL set_nonblocking unsupported: {e} — refusing to run unbounded")
    })?;
    println!("PROBE deadline adapter armed {}s", timeout.as_secs());
    Ok(stream)
}

fn run(addr: SocketAddr, nonce: &str, deadline_secs: u64) -> Result<(), String> {
    let started = Instant::now();
    let deadline = started + Duration::from_secs(deadline_secs);
    let socket_timeout = Duration::from_secs(deadline_secs);

    let stream = dial(addr, socket_timeout)?;
    let mut client = Client::new(stream);
    println!("PROBE connected {addr} deadline={deadline_secs}s");

    // 1. describe — echo fixture leg: proves the typed codec, not info answering.
    let sent = client
        .send(&Describe.to_event())
        .map_err(|e| format!("send describe: {e}"))?;
    journal(&sent);
    let got = recv_of_type(&mut client, "describe", deadline)?
        .ok_or("describe: connection closed before reply")?;
    Describe::from_event(&got).map_err(|e| format!("describe echo not a Describe: {e}"))?;
    println!("PROBE describe ok (echo fixture round-trip; a normal service would answer info)");

    // 2. info — typed Info out, re-parsed on the echo back.
    let info = Info {
        wake: vec![WakeProgram {
            name: "wyoming-probe".to_string(),
            attribution: Attribution {
                name: "redox-oneplus6".to_string(),
                url: "https://github.com/rhasspy/wyoming".to_string(),
            },
            installed: true,
            description: None,
            version: Some("0.1.0".to_string()),
            models: Vec::new(),
        }],
        ..Default::default()
    };
    let sent = client
        .send(&info.to_event())
        .map_err(|e| format!("send info: {e}"))?;
    journal(&sent);
    let got = recv_of_type(&mut client, "info", deadline)?
        .ok_or("info: connection closed before reply")?;
    let back = Info::from_event(&got).map_err(|e| format!("info echo invalid: {e}"))?;
    if back != info {
        return Err(format!("info echo mismatch: {back:?}"));
    }
    println!("PROBE info ok (typed Info round-tripped through the echo fixture)");

    // 3. nonce — real request/response: ping carries it, pong copies it back.
    let sent = client
        .send_ping(Some(nonce.to_string()))
        .map_err(|e| format!("send ping: {e}"))?;
    journal(&sent);
    let got = recv_of_type(&mut client, "pong", deadline)?
        .ok_or("pong: connection closed before reply")?;
    let pong = Pong::from_event(&got).map_err(|e| format!("pong invalid: {e}"))?;
    match pong.text.as_deref() {
        Some(text) if text == nonce => {
            println!("PROBE nonce ok (nonce {nonce} came back in pong)")
        }
        other => {
            return Err(format!(
                "nonce mismatch: pong text {other:?}, want {nonce:?}"
            ))
        }
    }

    // Clean close: the peer must observe EOF (Sink::shutdown).
    let closed = client.close();
    journal(&closed);
    println!("PROBE elapsed {:?}", started.elapsed());
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    // Opt-in diagnostic subcommand, dispatched before the positional probe
    // CLI: a numeric `ip:port` can never parse as "diag", so the existing
    // guest invocation `/usr/bin/wyoming-probe <ip:port> <nonce> <secs>` is
    // byte-for-byte unaffected.
    if args.get(1).map(String::as_str) == Some("diag") {
        process::exit(diag_cli::run(&args));
    }
    let Some(raw_addr) = args.get(1) else {
        eprintln!("usage: wyoming-probe <numeric-ip:port> [nonce] [deadline_secs]");
        process::exit(2);
    };
    // Numeric only: connect_timeout needs a resolved SocketAddr, and a
    // resolver step would be an unbounded network call of its own.
    let addr: SocketAddr = match raw_addr.parse() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("bad address {raw_addr:?}: numeric ip:port required ({e})");
            process::exit(2);
        }
    };
    let nonce = args
        .get(2)
        .cloned()
        .unwrap_or_else(|| format!("wy-probe-{}", process::id()));
    let deadline_secs = match args.get(3) {
        Some(raw) => match raw.parse::<u64>() {
            Ok(0) | Err(_) => {
                eprintln!("bad deadline_secs: {raw}");
                process::exit(2);
            }
            Ok(secs) => secs,
        },
        None => DEFAULT_DEADLINE_SECS,
    };

    match run(addr, &nonce, deadline_secs) {
        Ok(()) => {
            println!("PROBE PASS addr={addr} nonce={nonce}");
            process::exit(0);
        }
        Err(e) => {
            println!("PROBE FAIL {e}");
            process::exit(1);
        }
    }
}
