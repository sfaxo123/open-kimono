//! Local development transport. Protocol: publish trace-events JSONL and
//! receive {"seq":N} ACKs, or send one {"subscribe":filter,"backlog":N}
//! line and read trace-events JSONL indefinitely. No TCP listener exists.
use serde::Deserialize;
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use std::{env, fs, thread};
use tracejournal::{Filter, Journal, MAX_LINE_BYTES};

#[derive(Deserialize)]
struct Subscribe {
    subscribe: Filter,
    #[serde(default)]
    backlog: usize,
}

type Shared = Arc<(Mutex<Journal>, Condvar)>;

fn now_us() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as u64
}

fn read_line_limited(reader: &mut impl BufRead, line: &mut String) -> io::Result<usize> {
    line.clear();
    let mut limited = io::Read::take(reader, (MAX_LINE_BYTES + 2) as u64);
    let n = limited.read_line(line)?;
    if n > MAX_LINE_BYTES + 1 || (n > 0 && !line.ends_with('\n')) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "line exceeds limit or lacks newline",
        ));
    }
    Ok(n)
}

fn client(mut stream: UnixStream, shared: Shared) -> io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    if read_line_limited(&mut reader, &mut line)? == 0 {
        return Ok(());
    }
    if serde_json::from_str::<serde_json::Value>(&line)
        .ok()
        .and_then(|v| v.get("subscribe").cloned())
        .is_some()
    {
        let request: Subscribe = serde_json::from_str(&line)?;
        let (lock, changed) = &*shared;
        let mut sub = lock
            .lock()
            .unwrap()
            .subscribe(request.backlog, request.subscribe);
        loop {
            let batch = {
                let mut journal = lock.lock().unwrap();
                while journal.next_seq() == sub.next_seq() {
                    journal = changed.wait(journal).unwrap();
                }
                journal.poll(&mut sub, 64)
            };
            for event in batch {
                writeln!(stream, "{}", trace_events::to_json_line(&event)?)?;
            }
        }
    }
    loop {
        let event = match trace_events::from_json_line(&line) {
            Ok(event) => event,
            Err(_) => {
                writeln!(stream, "{{\"error\":\"invalid_event\"}}")?;
                return Ok(());
            }
        };
        let (lock, changed) = &*shared;
        let result = lock.lock().unwrap().publish(event, now_us());
        match result {
            Ok(seq) => {
                writeln!(stream, "{{\"seq\":{seq}}}")?;
                changed.notify_all();
            }
            Err(err) => {
                writeln!(stream, "{{\"error\":\"{err:?}\"}}")?;
            }
        }
        if read_line_limited(&mut reader, &mut line)? == 0 {
            return Ok(());
        }
    }
}

fn main() -> io::Result<()> {
    let path = env::args().nth(1).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "usage: traced-linux SOCKET_PATH",
        )
    })?;
    if fs::metadata(&path).is_ok() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "socket path already exists",
        ));
    }
    let listener = UnixListener::bind(&path)?;
    let shared = Arc::new((Mutex::new(Journal::new(256, 100)), Condvar::new()));
    for connection in listener.incoming() {
        let stream = connection?;
        let shared = shared.clone();
        thread::spawn(move || {
            if let Err(error) = client(stream, shared) {
                eprintln!("traced-linux: {error}");
            }
        });
    }
    Ok(())
}
