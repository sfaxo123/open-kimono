//! CLI front end for the opt-in raw-I/O diagnostic, kept out of `main.rs` so
//! the probe CLI stays small and the legacy positional form is untouched.
//!
//! `wyoming-probe diag <numeric-ip:port> [timeout_secs] [max_retries]`
//!
//! Point it at a listener that accepts and stays silent — the shape that hung
//! the guest. It sends no Wyoming frame, so it cannot change or stand in for a
//! probe result. Exit codes: 0 = every primitive answered (a correct bounded
//! silent timeout included); 1 = a primitive did not answer, or the clock looks
//! non-advancing; 2 = bad usage.

use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

use crate::diag::{self, Diag, DiagConfig, Verdict};

/// One truthful summary line per verdict. A bounded timeout is NOT reported as
/// a hang: the raw read returned WouldBlock every time and the in-process
/// deadline fired.
fn summary(v: Verdict) -> &'static str {
    match v {
        Verdict::BoundedTimeout => {
            "BOUNDED SILENT TIMEOUT OK: every read returned WouldBlock and the in-process deadline fired; this is the known-good control trail"
        }
        Verdict::ReadAnswered => "READ ANSWERED: no hang to localise on this socket",
        Verdict::ReadErrored => {
            "READ ANSWERED WITH AN ERROR: the primitive returned, the socket did not"
        }
        Verdict::RetryCapHit => {
            "RETRY CAP BEFORE DEADLINE: clock not advancing, or max_retries below op_timeout/backoff; the last DIAG-R phase names where iterations were still returning"
        }
        Verdict::NonblockingFailed => {
            "SET_NONBLOCKING FAILED: refusing to run unbounded; nothing downstream can be bounded"
        }
    }
}

pub fn run(args: &[String]) -> i32 {
    let usage = "usage: wyoming-probe diag <numeric-ip:port> [timeout_secs] [max_retries]";
    let Some(raw_addr) = args.get(2) else {
        eprintln!("{usage}");
        return 2;
    };
    let addr: SocketAddr = match raw_addr.parse() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("bad address {raw_addr:?}: numeric ip:port required ({e})");
            return 2;
        }
    };
    let timeout_secs = match args.get(3) {
        Some(raw) => match raw.parse::<u64>() {
            Ok(0) | Err(_) => {
                eprintln!("bad timeout_secs: {raw}");
                return 2;
            }
            Ok(secs) => secs,
        },
        None => crate::DEFAULT_DEADLINE_SECS,
    };
    let mut cfg = DiagConfig::new(Duration::from_secs(timeout_secs));
    if let Some(raw) = args.get(4) {
        match raw.parse::<u32>() {
            Ok(0) | Err(_) => {
                eprintln!("bad max_retries: {raw}");
                return 2;
            }
            Ok(n) => cfg.max_retries = n,
        }
    }

    let stream = match TcpStream::connect_timeout(&addr, Duration::from_secs(timeout_secs)) {
        Ok(s) => s,
        Err(e) => {
            println!("DIAG FAIL connect {addr}: {e}");
            return 1;
        }
    };
    println!(
        "DIAG connected {addr} op_timeout={}s backoff={}ms max_retries={} (external timeout --kill-after still required: nothing here can bound a call that never returns)",
        timeout_secs,
        cfg.backoff.as_millis(),
        cfg.max_retries
    );

    let mut probe = Diag::new(true);
    let out = diag::run(&stream, &cfg, &mut probe);
    println!(
        "DIAG SUMMARY markers={} verdict={} first_read={:?} first_write={:?} retries={} clock_advanced_us={} nonblocking_took_us={}",
        probe.markers().len(),
        out.verdict.name(),
        out.read_result,
        out.write_result,
        out.retries,
        out.clock_advanced_us,
        out.nonblocking_took_us
    );
    println!("DIAG {}", summary(out.verdict));
    out.verdict.exit_code()
}
