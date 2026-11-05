//! Fault-injection sinks for the host log store's writer path.
//!
//! Two kinds, because the two things worth proving are different:
//! - [`MemorySink`] discards bytes into a buffer, which is how a test
//!   observes what the shared append logic *tried* to write;
//! - [`FileFaultSink`] drives the REAL `std::fs::File` path, so a write or
//!   sync fault is injected into the same code a deployment would use.
//!
//! Neither is a durability model and neither is a production sink. A store
//! given one of these is being told to remember nothing: it is a test seam,
//! not a fallback, and the library has no reason to prefer either.

use std::fs::File;
use std::io::{self, Write};

use device_control_host::ByteSink;

/// How many times the sink may report failure before it behaves normally,
/// and whether a failing write first hands over half the record.
#[derive(Debug, Clone, Copy)]
pub struct FaultPlan {
    /// `Some(n)`: the (n+1)-th `write_all` reports failure. Counting from 0.
    pub fail_write_at: Option<usize>,
    /// `Some(n)`: the (n+1)-th `sync` reports failure. Counting from 0.
    pub fail_sync_at: Option<usize>,
    /// On a failing `write_all`, hand over exactly half the bytes first.
    pub partial_on_fail: bool,
}

impl FaultPlan {
    pub const NO_FAULTS: FaultPlan = FaultPlan {
        fail_write_at: None,
        fail_sync_at: None,
        partial_on_fail: false,
    };

    pub fn write_fails_at(n: usize, partial: bool) -> Self {
        FaultPlan {
            fail_write_at: Some(n),
            fail_sync_at: None,
            partial_on_fail: partial,
        }
    }

    pub fn sync_fails_at(n: usize) -> Self {
        FaultPlan {
            fail_write_at: None,
            fail_sync_at: Some(n),
            partial_on_fail: false,
        }
    }
}

/// In-memory sink: keeps whatever the store wrote so a test can compare it
/// against the bytes it expects. Discards nothing, persists nothing.
#[derive(Debug)]
pub struct MemorySink {
    pub written: Vec<u8>,
    plan: FaultPlan,
    write_calls: usize,
    sync_calls: usize,
}

impl MemorySink {
    pub fn new() -> Self {
        MemorySink {
            written: Vec::new(),
            plan: FaultPlan::NO_FAULTS,
            write_calls: 0,
            sync_calls: 0,
        }
    }

    pub fn with_plan(plan: FaultPlan) -> Self {
        MemorySink {
            written: Vec::new(),
            plan,
            write_calls: 0,
            sync_calls: 0,
        }
    }

    pub fn write_calls(&self) -> usize {
        self.write_calls
    }

    pub fn sync_calls(&self) -> usize {
        self.sync_calls
    }

    fn write_fails(&mut self) -> bool {
        let n = self.write_calls;
        self.write_calls += 1;
        self.plan.fail_write_at == Some(n)
    }

    fn sync_fails(&mut self) -> bool {
        let n = self.sync_calls;
        self.sync_calls += 1;
        self.plan.fail_sync_at == Some(n)
    }
}

impl Default for MemorySink {
    fn default() -> Self {
        Self::new()
    }
}

impl ByteSink for MemorySink {
    fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
        if self.write_fails() {
            if self.plan.partial_on_fail && !bytes.is_empty() {
                let half = bytes.len() / 2;
                self.written.extend_from_slice(&bytes[..half]);
            }
            return Err(io::Error::other("injected write failure"));
        }
        self.written.extend_from_slice(bytes);
        Ok(())
    }

    fn sync(&mut self) -> io::Result<()> {
        if self.sync_fails() {
            return Err(io::Error::other("injected sync failure"));
        }
        Ok(())
    }
}

/// Real-file sink that fails on demand, so a test can put a fault in the
/// exact code path production uses (`File::write_all` / `File::sync_data`)
/// instead of only in a buffer.
#[derive(Debug)]
pub struct FileFaultSink {
    file: File,
    plan: FaultPlan,
    write_calls: usize,
    sync_calls: usize,
}

impl FileFaultSink {
    pub fn new(file: File) -> Self {
        FileFaultSink {
            file,
            plan: FaultPlan::NO_FAULTS,
            write_calls: 0,
            sync_calls: 0,
        }
    }

    pub fn with_plan(file: File, plan: FaultPlan) -> Self {
        FileFaultSink {
            file,
            plan,
            write_calls: 0,
            sync_calls: 0,
        }
    }

    fn write_fails(&mut self) -> bool {
        let n = self.write_calls;
        self.write_calls += 1;
        self.plan.fail_write_at == Some(n)
    }

    fn sync_fails(&mut self) -> bool {
        let n = self.sync_calls;
        self.sync_calls += 1;
        self.plan.fail_sync_at == Some(n)
    }
}

impl ByteSink for FileFaultSink {
    fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
        if self.write_fails() {
            if self.plan.partial_on_fail && !bytes.is_empty() {
                let half = bytes.len() / 2;
                self.file.write_all(&bytes[..half])?;
            }
            return Err(io::Error::other("injected write failure"));
        }
        self.file.write_all(bytes)
    }

    fn sync(&mut self) -> io::Result<()> {
        if self.sync_fails() {
            return Err(io::Error::other("injected sync failure"));
        }
        self.file.flush()?;
        self.file.sync_data()
    }
}
