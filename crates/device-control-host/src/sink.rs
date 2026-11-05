//! Shared writer-state logic for the host log store.
//!
//! The append path in [`crate::store`] talks to this trait, never to a
//! concrete file, so a caller (or a test fixture) can inject a sink that
//! fails, and the store's error handling gets exercised on the same code
//! path production uses.
//!
//! Trust contract for a caller-provided sink — read it before injecting one:
//! - the store TRUSTS the sink completely. It cannot tell a real file from a
//!   buffer, so it performs no repair and forms no opinion about what the
//!   sink actually persisted;
//! - `sync` on a sink that merely buffers is NOT durability. A sink is not
//!   required to reach media at all, and nothing here proves that it does;
//! - `write_all` failing mid-call may leave a partial record in the sink's
//!   own storage; the store will not truncate or rewrite it;
//! - the sink lives behind a trait object, so the store also cannot inspect
//!   it again after construction.
//!
//! There is deliberately no built-in fake or null sink: a store must be
//! given a sink, and what that sink guarantees is the caller's problem to
//! state. The in-memory fault-injection sink used by this crate's tests
//! lives in `tests/fixtures/failing_sink.rs` and is not compiled into
//! production.

use std::fs::File;
use std::io::{self, Write};

/// The only two side effects the store performs on the sink it was given.
pub trait ByteSink {
    /// Append all bytes or fail. A partial write must be reported as an
    /// error, never as `Ok`, so the store cannot book bytes it does not have.
    fn write_all(&mut self, bytes: &[u8]) -> io::Result<()>;
    /// Flush whatever this sink buffers, then sync. For [`FileSink`] that is
    /// `File::flush` + `File::sync_data`: it proves the bytes were handed to
    /// the OS, not that they survived a power loss.
    fn sync(&mut self) -> io::Result<()>;
}

/// Sink over one opened file handle, opened for writing by the caller.
///
/// Durability contract: `sync` asks the OS to commit the region and reports
/// failure if the OS refuses. It makes NO claim about disk power loss,
/// write caches, or a phone that loses power mid-write, and it proves
/// nothing about how the file was created.
pub struct FileSink {
    file: File,
}

impl FileSink {
    pub fn new(file: File) -> Self {
        FileSink { file }
    }
}

impl ByteSink for FileSink {
    fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.file.write_all(bytes)
    }
    fn sync(&mut self) -> io::Result<()> {
        self.file.flush()?;
        self.file.sync_data()
    }
}
