//! JSONL feed: turn journal lines into events without ever dying on one bad
//! line. The journal is written by other processes while the screen runs, so
//! partial writes and the occasional corrupt line are normal operation —
//! count them, keep going (the count itself shows up in the status line).

use std::fs::File;
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use trace_events::{from_json_line, Event};

/// What one poll of the feed returned.
#[derive(Debug, Default)]
pub struct FeedBatch {
    pub events: Vec<Event>,
    /// Lines that failed to parse (counted, never fatal).
    pub bad_lines: u64,
}

/// Parse a complete chunk of JSONL text (blank lines skipped).
pub fn parse_chunk(text: &str) -> FeedBatch {
    let mut batch = FeedBatch::default();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        match from_json_line(line) {
            Ok(event) => batch.events.push(event),
            Err(_) => batch.bad_lines += 1,
        }
    }
    batch
}

/// A feed source: a growing file (tailed) or stdin (piped). Bytes are kept
/// until a newline lands, so a partial write or a non-UTF-8 byte corrupts at
/// most one line instead of the whole poll.
pub enum Feed {
    File { path: PathBuf, file: File, pending: Vec<u8> },
    Scheme { file: File, pending: Vec<u8> },
    Stdin { reader: BufReader<io::Stdin>, pending: Vec<u8> },
}

impl Feed {
    pub fn file(path: impl AsRef<Path>) -> io::Result<Self> {
        Ok(Feed::File { path: path.as_ref().to_path_buf(), file: File::open(path)?, pending: Vec::new() })
    }

    pub fn stdin() -> Self {
        Feed::Stdin { reader: BufReader::new(io::stdin()), pending: Vec::new() }
    }

    /// Redox trace: reads are nonblocking and return EAGAIN while the journal is idle.
    pub fn scheme(path: impl AsRef<Path>) -> io::Result<Self> {
        Ok(Feed::Scheme { file: File::open(path)?, pending: Vec::new() })
    }

    /// Read whatever is available now; complete lines are parsed, a trailing
    /// partial line is kept for the next poll.
    pub fn poll(&mut self) -> io::Result<FeedBatch> {
        match self {
            Feed::File { file, pending, .. } => {
                let mut chunk = Vec::new();
                file.read_to_end(&mut chunk)?;
                pending.extend_from_slice(&chunk);
                take_complete_lines(pending)
            }
            Feed::Scheme { file, pending } => {
                let mut chunk = [0u8; 4096];
                loop {
                    match file.read(&mut chunk) {
                        Ok(0) => break,
                        Ok(n) => pending.extend_from_slice(&chunk[..n]),
                        Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                        Err(e) => return Err(e),
                    }
                }
                take_complete_lines(pending)
            }
            Feed::Stdin { reader, pending } => {
                loop {
                    let filled = reader.fill_buf()?;
                    if filled.is_empty() {
                        break;
                    }
                    pending.extend_from_slice(filled);
                    let n = filled.len();
                    reader.consume(n);
                }
                take_complete_lines(pending)
            }
        }
    }
}

/// Split off complete lines from `pending`, keeping the unterminated tail.
fn take_complete_lines(pending: &mut Vec<u8>) -> io::Result<FeedBatch> {
    let complete = match pending.iter().rposition(|b| *b == b'\n') {
        Some(i) => {
            let head: Vec<u8> = pending.drain(..=i).collect();
            head
        }
        None => return Ok(FeedBatch::default()),
    };
    Ok(parse_chunk(&String::from_utf8_lossy(&complete)))
}

/// Re-open a file at 0 (used by the demo replayer).
pub fn reopen_file(path: &Path) -> io::Result<File> {
    let mut f = File::open(path)?;
    f.seek(SeekFrom::Start(0))?;
    Ok(f)
}
