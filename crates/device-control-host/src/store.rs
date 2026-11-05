//! Bounded, append-only HOST-side log store over control-envelope v1 frames.
//!
//! This stores what a host RECEIVED from a device — it is not phone logging
//! and makes no claim that any live device produced these bytes. Identity
//! fields (boot id, build id) are supplied by the caller and TRUSTED: they
//! prevent cross-wiring of streams, they are NOT authentication.
//!
//! Guarantees this module actually provides:
//! - creation is exclusive (`OpenOptions::create_new`): an existing path
//!   fails; the directory is created private (mode 0o700, private at the
//!   moment of creation, not widened and then narrowed) and the file is
//!   created private (mode 0o600) before any byte is written;
//! - `dir_name` and `file_name` are validated as names before any directory
//!   or file is touched (see [`crate::pathname`]); the parent stays a
//!   caller-supplied precondition, so this is not a symlink-jail claim;
//! - config is validated by EVERY constructor, before any file is created;
//! - quota covers encoded bytes and is checked BEFORE writing;
//! - every accepted record is `write_all` then `sync_data`; any write or
//!   sync error poisons the store — no blind retry, no fake durable success;
//! - no overwrites, no truncation, no automatic deletion.
//!
//! This module does NOT claim: symlink-jail safety against an adversarial
//! parent (the parent is caller-trusted), protection against disk power
//! loss, or any phone/hardware durability proof.

use std::fs::OpenOptions;
use std::io;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use device_control::log_record::LogRecord;
use device_control::{Frame, Op, Policy};

use crate::pathname;
use crate::sink::ByteSink;

/// Rejection before any file is created.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigReject {
    ZeroBootId,
    ZeroBuildId,
    /// Quota must be in 1..=64 MiB.
    QuotaOutOfRange,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreError {
    Config(ConfigReject),
    /// The store path already exists (or the private dir/file could not be
    /// created exclusively). The io kind is preserved for the caller.
    Create(String),
    /// Envelope failed to parse under the frozen frame policy.
    Frame(device_control::Error),
    /// Body failed to decode under the frozen log-body schema.
    Body(device_control::log_record::LogError),
    /// Envelope boot id or body build id did not match the expected ones.
    WrongBootId,
    WrongBuildId,
    /// `op` was not `Logs`.
    NotLogs,
    /// Sequence was equal to or below the last accepted one, or left a gap.
    NonIncreasingSequence {
        last: u64,
        got: u64,
    },

    /// Encoded bytes would exceed the quota; rejected before writing.
    QuotaExceeded {
        used: u64,
        frame_len: u64,
        quota: u64,
    },
    /// A previous write or sync failed; the store is poisoned and accepts
    /// nothing further (the on-disk tail may be incomplete — replay decides).
    Poisoned,
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::Config(r) => write!(f, "invalid store config: {r:?}"),
            StoreError::Create(m) => write!(f, "store creation failed: {m}"),
            StoreError::Frame(e) => write!(f, "frame rejected: {e}"),
            StoreError::Body(e) => write!(f, "log body rejected: {e}"),
            StoreError::WrongBootId => write!(f, "envelope boot id does not match expected"),
            StoreError::WrongBuildId => write!(f, "record build id does not match expected"),
            StoreError::NotLogs => write!(f, "frame op is not Logs"),
            StoreError::NonIncreasingSequence { last, got } => {
                write!(f, "sequence {got} is not above last accepted {last}")
            }

            StoreError::QuotaExceeded {
                used,
                frame_len,
                quota,
            } => {
                write!(f, "quota exceeded: used {used} + {frame_len} > {quota}")
            }
            StoreError::Poisoned => write!(f, "store poisoned by a prior write or sync failure"),
        }
    }
}

/// What one `append` call did, per record. A record that was not stored is
/// never counted as stored: `stored_bytes` stays 0 and `stored` is false, so
/// a caller can tell "this file already had that sequence" from "this record
/// was accepted" without re-reading the file.
///
/// Frozen DC-103 semantics: every field describes THIS call —
/// `sequence_gaps` is what this call created or observed (always 0 when the
/// call stored nothing), and a duplicate refusal is a TYPED outcome (`Ok`
/// with `stored == false`), never a strict `Err` rejection. The standing
/// totals live on the store: `store.sequence_gaps()`,
/// `store.skipped_duplicates()`, `store.rejected_backwards()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppendOutcome {
    /// True only when this call wrote this record's bytes.
    pub stored: bool,
    /// Bytes of the original frame stored by THIS call.
    pub stored_bytes: u64,
    /// Sequence numbers missing between the last accepted record and this
    /// one — the gaps created or observed by THIS call, never a standing
    /// total (the frozen DC-103 semantics: the field means the same thing
    /// on every path). `seq 5 -> 7` is one missing number. A gap is never
    /// repaired and never filled in: the record after a gap is still
    /// stored, and the gap is reported, because refusing it would silently
    /// discard a live stream. The cumulative total over the store's whole
    /// life is `store.sequence_gaps()`.
    pub sequence_gaps: u64,
    /// Duplicate (equal-seq) frames refused so far, including this call.
    pub skipped_duplicates: u64,
    /// Backwards (lower-seq) frames refused so far, including this call.
    pub rejected_backwards: u64,
}

impl AppendOutcome {
    /// Nothing was written, and the reason is in `skipped_duplicates` /
    /// `rejected_backwards`.
    pub fn refused(&self) -> bool {
        !self.stored
    }
}

/// Caller-trusted configuration. Ids are identity, not authentication.
#[derive(Debug, Clone, Copy)]
pub struct StoreConfig {
    pub expected_boot_id: [u8; 16],
    pub expected_build_id: [u8; 16],
    pub quota_bytes: u64,
}

pub const QUOTA_MIN: u64 = 1;
pub const QUOTA_MAX: u64 = 64 * 1024 * 1024;

fn is_zero(id: &[u8; 16]) -> bool {
    id.iter().all(|&b| b == 0)
}

/// The one config check, used by every constructor. Identity fields are
/// caller-supplied, but a zero id means the check is not happening at all,
/// so it is refused here rather than trusted.
fn check_config(config: &StoreConfig) -> Result<(), ConfigReject> {
    if is_zero(&config.expected_boot_id) {
        return Err(ConfigReject::ZeroBootId);
    }
    if is_zero(&config.expected_build_id) {
        return Err(ConfigReject::ZeroBuildId);
    }
    if config.quota_bytes < QUOTA_MIN || config.quota_bytes > QUOTA_MAX {
        return Err(ConfigReject::QuotaOutOfRange);
    }
    Ok(())
}

/// Every sequence number strictly between `last` and `got`, which is what a
/// reader needs to tell a gap from a complete stream. `last == 0` means no
/// record has been accepted yet, so the first record's own sequence is also
/// behind: starting at seq 3 loses sequences 1 and 2 before anything was
/// stored — the probe states "seq 1->3 and 3->7 lose sequence numbers".
fn missing_sequences(last: u64, got: u64) -> u64 {
    if last >= got {
        return 0;
    }
    match got.checked_sub(last) {
        Some(d) if d > 1 => d - 1,
        _ => 0,
    }
}

/// Append-only store over one host file. The file holds the exact original
/// framed bytes of every accepted record, concatenated, nothing else.
pub struct HostLogStore {
    sink: Box<dyn ByteSink>,
    config: StoreConfig,
    used: u64,
    last_seq: u64,
    poisoned: bool,
    skipped_duplicates: u64,
    rejected_backwards: u64,
    total_gaps: u64,
}

impl HostLogStore {
    /// Create a NEW private directory `dir_name` under the caller-supplied
    /// `parent` and a NEW exclusive file inside it.
    ///
    /// `parent` is a precondition: it is used as given, so this is not a
    /// symlink-jail defence against a hostile parent. What IS checked is the
    /// two names, because a name is where a typo or a crafted input escapes:
    /// an empty name, `.`, `..`, a name with a separator, an absolute name,
    /// or any multi-component name is refused before a directory or file is
    /// touched. A refused create may still leave behind a directory the
    /// caller explicitly named — this function never deletes a path that is
    /// not its own creation.
    pub fn create(
        parent: &Path,
        dir_name: &str,
        file_name: &str,
        config: StoreConfig,
    ) -> io::Result<Self> {
        check_config(&config).map_err(|r| io::Error::other(StoreError::Config(r).to_string()))?;
        let dir = pathname::child(parent, dir_name).map_err(|r| {
            io::Error::other(format!("rejected store directory name {dir_name:?}: {r:?}"))
        })?;
        let file = pathname::child(&dir, file_name).map_err(|r| {
            io::Error::other(format!("rejected store file name {file_name:?}: {r:?}"))
        })?;
        // Private at creation: no window where the directory is world-readable.
        pathname::create_private_dir(&dir)?;
        let opened = match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&file)
        {
            Ok(f) => f,
            // The directory was named by the caller and may be half-built;
            // it is left exactly as found, because removing a path that the
            // caller might have made is a bigger risk than leaving it.
            Err(e) => return Err(e),
        };
        Ok(HostLogStore::new(
            Box::new(crate::sink::FileSink::new(opened)),
            config,
        ))
    }

    /// Drive the same append logic over a caller-provided sink.
    ///
    /// Config validation is NOT skipped here: a store built through this
    /// seam refuses a zero boot id, a zero build id, and a quota outside
    /// 1..=64 MiB, exactly as `create` does. What the sink guarantees
    /// (nothing, if it is a test fake) is the caller's problem — see the
    /// trust contract in [`crate::sink`].
    pub fn with_sink(sink: Box<dyn ByteSink>, config: StoreConfig) -> Result<Self, StoreError> {
        check_config(&config).map_err(StoreError::Config)?;
        Ok(Self::new(sink, config))
    }

    fn new(sink: Box<dyn ByteSink>, config: StoreConfig) -> Self {
        HostLogStore {
            sink,
            config,
            used: 0,
            last_seq: 0,
            poisoned: false,
            skipped_duplicates: 0,
            rejected_backwards: 0,
            total_gaps: 0,
        }
    }

    pub fn is_poisoned(&self) -> bool {
        self.poisoned
    }
    pub fn used_bytes(&self) -> u64 {
        self.used
    }
    pub fn last_seq(&self) -> u64 {
        self.last_seq
    }
    /// Duplicate (equal-seq) frames skipped so far, reported explicitly.
    pub fn skipped_duplicates(&self) -> u64 {
        self.skipped_duplicates
    }
    pub fn rejected_backwards(&self) -> u64 {
        self.rejected_backwards
    }
    /// Sequence numbers lost so far, summed over every accepted record.
    pub fn sequence_gaps(&self) -> u64 {
        self.total_gaps
    }

    /// Accept one candidate envelope. All validation and the quota check
    /// happen BEFORE any write; on write/sync error the store is poisoned.
    pub fn append(&mut self, input: &[u8]) -> Result<AppendOutcome, StoreError> {
        if self.poisoned {
            return Err(StoreError::Poisoned);
        }
        let frame = match Frame::parse(input, &Policy::default()) {
            Ok(f) => f,
            Err(e) => return Err(StoreError::Frame(e)),
        };
        if frame.op() != Op::Logs {
            return Err(StoreError::NotLogs);
        }
        if frame.boot_id() != &self.config.expected_boot_id {
            return Err(StoreError::WrongBootId);
        }
        let framed = match LogRecord::from_frame(&frame) {
            Ok(r) => r,
            Err(e) => return Err(StoreError::Body(e)),
        };
        if framed.record().build_id() != &self.config.expected_build_id {
            return Err(StoreError::WrongBuildId);
        }
        let seq = framed.record().sequence();
        if seq == self.last_seq {
            self.skipped_duplicates += 1;
            // A duplicate refusal is a TYPED outcome (Ok + stored == false),
            // never an Err. Frozen semantics: `sequence_gaps` describes
            // THIS call, and this call created no gaps — it stores nothing,
            // so it can neither create nor erase one. The standing total
            // (`self.total_gaps`) is unchanged and stays readable through
            // `store.sequence_gaps()`.
            return Ok(AppendOutcome {
                stored: false,
                stored_bytes: 0,
                sequence_gaps: 0,
                skipped_duplicates: self.skipped_duplicates,
                rejected_backwards: self.rejected_backwards,
            });
        }
        if seq < self.last_seq {
            self.rejected_backwards += 1;
            return Err(StoreError::NonIncreasingSequence {
                last: self.last_seq,
                got: seq,
            });
        }
        // A gap is reported, never repaired: the record is still stored,
        // because refusing it would silently discard a live stream after a
        // crash or reconnect, and the host cannot tell "the device skipped
        // numbers" from "the device restarted its numbering".
        let gaps = missing_sequences(self.last_seq, seq);
        let frame_len = match u64::try_from(input.len()) {
            Ok(n) => n,
            Err(_) => {
                return Err(StoreError::QuotaExceeded {
                    used: self.used,
                    frame_len: u64::MAX,
                    quota: self.config.quota_bytes,
                })
            }
        };
        let next = match self.used.checked_add(frame_len) {
            Some(n) => n,
            None => {
                return Err(StoreError::QuotaExceeded {
                    used: self.used,
                    frame_len,
                    quota: self.config.quota_bytes,
                })
            }
        };
        if next > self.config.quota_bytes {
            return Err(StoreError::QuotaExceeded {
                used: self.used,
                frame_len,
                quota: self.config.quota_bytes,
            });
        }
        // Exact original framed bytes, byte-for-byte, then durability sync.
        if self.sink.write_all(input).is_err() {
            self.poisoned = true;
            return Err(StoreError::Poisoned);
        }
        if self.sink.sync().is_err() {
            self.poisoned = true;
            return Err(StoreError::Poisoned);
        }
        self.used = next;
        self.last_seq = seq;
        self.total_gaps += gaps;
        Ok(AppendOutcome {
            stored: true,
            stored_bytes: frame_len,
            sequence_gaps: gaps,
            skipped_duplicates: self.skipped_duplicates,
            rejected_backwards: self.rejected_backwards,
        })
    }
}
