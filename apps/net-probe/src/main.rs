// E1-NET-NATIVE-PROBE: tiny std-only native TCP client to discriminate the
// guest write path from the ion builtin path. Zero dependencies.
// Usage: net-probe <nonce> <host:port> [--scheme]
//   Guest runs MUST pass an explicit endpoint, e.g. 10.0.2.2:4765 (slirp host
//   alias). 127.0.0.1 is the guest's own stack — host listener / host smoke only.
// Every step is logged to stderr (lands on the serial console).
use std::env;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

fn main() {
    let args: Vec<String> = env::args().collect();
    let nonce = args.get(1).cloned().unwrap_or_else(|| "e1np-missing".to_string());
    let endpoint = match args.get(2) {
        Some(e) if e.contains(':') => e.clone(),
        _ => {
            eprintln!("net-probe: USAGE net-probe <nonce> <host:port> [--scheme]");
            eprintln!("net-probe: ERROR no explicit endpoint (refusing default)");
            std::process::exit(2);
        }
    };

    eprintln!("net-probe: start pid_nonce={nonce} endpoint={endpoint}");

    eprintln!("net-probe: CONNECT_START {endpoint}");
    match TcpStream::connect(&endpoint) {
        Ok(mut s) => {
            eprintln!("net-probe: CONNECT_OK");
            let payload = format!("{nonce}\n");
            eprintln!("net-probe: WRITE_START {} bytes", payload.len());
            match s.write_all(payload.as_bytes()) {
                Ok(()) => eprintln!("net-probe: WRITE_OK {} bytes", payload.len()),
                Err(e) => eprintln!("net-probe: WRITE_ERROR {e}"),
            }
            match s.flush() {
                Ok(()) => eprintln!("net-probe: FLUSH_OK"),
                Err(e) => eprintln!("net-probe: FLUSH_ERROR {e}"),
            }
            // Keep the socket OPEN after write_all and wait bounded for the
            // host ACK nonce. Do not write-and-return: that conflates
            // payload-write with close-before-drain.
            match s.set_read_timeout(Some(Duration::from_secs(5))) {
                Ok(()) => eprintln!("net-probe: READ_TIMEOUT_SET 5s"),
                Err(e) => eprintln!(
                    "net-probe: READ_TIMEOUT_UNSUPPORTED {e} (relying on boot kill-after bound)"
                ),
            }
            let mut buf = [0u8; 128];
            match s.read(&mut buf) {
                Ok(0) => eprintln!("net-probe: READ_EOF peer closed before ACK"),
                Ok(n) => {
                    let got = String::from_utf8_lossy(&buf[..n]).trim_end().to_string();
                    if got == format!("ACK-{nonce}") {
                        eprintln!("net-probe: READ_ACK match {n} bytes: {got}");
                    } else {
                        eprintln!("net-probe: READ_MISMATCH {n} bytes: {got}");
                    }
                }
                Err(e) => eprintln!("net-probe: READ_ERROR {e}"),
            }
            drop(s); // close only after the bounded read
            eprintln!("net-probe: CLOSED");
        }
        Err(e) => eprintln!("net-probe: CONNECT_ERROR {e}"),
    }

    if args.iter().any(|a| a == "--scheme") {
        use std::fs::OpenOptions;
        eprintln!("net-probe: SCHEME_START /scheme/tcp");
        match OpenOptions::new().read(true).write(true).open("/scheme/tcp") {
            Ok(_) => eprintln!("net-probe: SCHEME_OK"),
            Err(e) => eprintln!("net-probe: SCHEME_ERROR {e}"),
        }
    }
    eprintln!("net-probe: done");
}
