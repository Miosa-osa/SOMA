//! The durable append-only ledger of worker lifecycle, claims, and transfer steps.
//!
//! Every record is one create-exclusive, checksummed file named by its sequence number,
//! synced together with its directory before the append returns.
//! Records are never rewritten; the projection in [`Ledger::entries`] folds them in order
//! and fails closed on a corrupt record or an invariant violation such as a sterile record
//! after an assignment.

mod fold;
mod record;

use std::{
    collections::BTreeMap,
    fmt,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

pub use record::{RECORD_LEN, Record, RecordKind, now_nanos};

use fold::fold;

use crate::{LeaseGeneration, OperationId, RequestFingerprint, WorkerId, WorkerLedgerEntry};

/// Why the ledger refused or could not answer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LedgerError {
    /// The directory could not be created or read.
    Open(io::ErrorKind),
    /// A record could not be written or synced.
    Write {
        /// The sequence number.
        seq: u64,
        /// The failure.
        kind: io::ErrorKind,
    },
    /// Another writer owns the sequence number; the ledger is not exclusively owned.
    Contended {
        /// The sequence number.
        seq: u64,
    },
    /// A record could not be read.
    Read(io::ErrorKind),
    /// A record failed its checksum or layout.
    Corrupt {
        /// The sequence number.
        seq: u64,
    },
    /// The record sequence violates the state machine.
    Invariant {
        /// The worker.
        worker: WorkerId,
        /// The offending kind.
        kind: RecordKind,
    },
}

impl fmt::Display for LedgerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Open(kind) => write!(formatter, "ledger open failed: {kind:?}"),
            Self::Write { seq, kind } => write!(formatter, "record {seq} write failed: {kind:?}"),
            Self::Contended { seq } => write!(formatter, "record {seq} already exists"),
            Self::Read(kind) => write!(formatter, "ledger read failed: {kind:?}"),
            Self::Corrupt { seq } => write!(formatter, "record {seq} is corrupt"),
            Self::Invariant { worker, kind } => {
                write!(
                    formatter,
                    "{worker:?} record {kind:?} violates the state machine"
                )
            }
        }
    }
}

impl std::error::Error for LedgerError {}

/// One claim as the ledger recorded it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClaimRecord {
    /// The operation.
    pub operation: OperationId,
    /// The fingerprint.
    pub fingerprint: RequestFingerprint,
    /// The worker.
    pub worker: WorkerId,
    /// The lease generation.
    pub lease_generation: LeaseGeneration,
    /// The claim class code.
    pub class: u8,
}

/// One ledger directory.
#[derive(Debug)]
pub struct Ledger {
    root: PathBuf,
    next: AtomicU64,
}

impl Ledger {
    /// Opens or creates the ledger and resumes after the highest existing sequence.
    ///
    /// # Errors
    ///
    /// Returns [`LedgerError::Open`] when the directory is unusable.
    pub fn open(root: &Path) -> Result<Self, LedgerError> {
        fs::create_dir_all(root).map_err(|error| LedgerError::Open(error.kind()))?;
        let highest = sequences(root)?.last().copied().unwrap_or(0);
        Ok(Self {
            root: root.to_path_buf(),
            next: AtomicU64::new(highest + 1),
        })
    }

    /// Returns the directory.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Appends one record durably and returns its sequence number.
    ///
    /// # Errors
    ///
    /// Returns [`LedgerError::Contended`] when the name exists or [`LedgerError::Write`].
    pub fn append(&self, record: &Record) -> Result<u64, LedgerError> {
        let seq = self.next.fetch_add(1, Ordering::AcqRel);
        let path = self.root.join(format!("{seq:016x}.rec"));
        let mut file = match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                return Err(LedgerError::Contended { seq });
            }
            Err(error) => {
                return Err(LedgerError::Write {
                    seq,
                    kind: error.kind(),
                });
            }
        };
        let written = file
            .write_all(&record.encode())
            .and_then(|()| file.sync_all())
            .and_then(|()| File::open(&self.root)?.sync_all());
        if let Err(error) = written {
            let _ = fs::remove_file(&path);
            return Err(LedgerError::Write {
                seq,
                kind: error.kind(),
            });
        }
        Ok(seq)
    }

    /// Reads every record in sequence order.
    ///
    /// # Errors
    ///
    /// Returns [`LedgerError::Read`] or [`LedgerError::Corrupt`]; a corrupt record fails the
    /// whole listing.
    pub fn records(&self) -> Result<Vec<(u64, Record)>, LedgerError> {
        sequences(&self.root)?
            .into_iter()
            .map(|seq| {
                let path = self.root.join(format!("{seq:016x}.rec"));
                let mut bytes = Vec::with_capacity(RECORD_LEN + 1);
                File::open(&path)
                    .and_then(|file| file.take(RECORD_LEN as u64 + 1).read_to_end(&mut bytes))
                    .map_err(|error| LedgerError::Read(error.kind()))?;
                Record::decode(&bytes)
                    .map(|record| (seq, record))
                    .ok_or(LedgerError::Corrupt { seq })
            })
            .collect()
    }

    /// Folds every record into one entry per worker.
    ///
    /// # Errors
    ///
    /// Returns a read failure or [`LedgerError::Invariant`].
    pub fn entries(&self) -> Result<BTreeMap<WorkerId, WorkerLedgerEntry>, LedgerError> {
        let mut entries = BTreeMap::new();
        for (_, record) in self.records()? {
            fold(&mut entries, &record)?;
        }
        Ok(entries)
    }

    /// Lists every recorded claim in order.
    ///
    /// # Errors
    ///
    /// Returns a read failure.
    pub fn claims(&self) -> Result<Vec<ClaimRecord>, LedgerError> {
        Ok(self
            .records()?
            .into_iter()
            .filter(|(_, record)| record.kind == RecordKind::Claiming)
            .filter_map(|(_, record)| {
                Some(ClaimRecord {
                    operation: record.operation?,
                    fingerprint: record.fingerprint?,
                    worker: record.worker,
                    lease_generation: record.lease_generation,
                    class: record.detail,
                })
            })
            .collect())
    }

    /// Returns the claim the ledger recorded for `operation`, if any.
    ///
    /// This is the durable record of an idempotent claim: the in-memory registry may evict
    /// a binding or start empty after a restart, and this lookup restores it.
    ///
    /// # Errors
    ///
    /// Returns a read failure.
    pub fn claim_of(&self, operation: OperationId) -> Result<Option<ClaimRecord>, LedgerError> {
        Ok(self
            .claims()?
            .into_iter()
            .find(|claim| claim.operation == operation))
    }
}

fn sequences(root: &Path) -> Result<Vec<u64>, LedgerError> {
    let mut sequences = Vec::new();
    for entry in fs::read_dir(root).map_err(|error| LedgerError::Read(error.kind()))? {
        let entry = entry.map_err(|error| LedgerError::Read(error.kind()))?;
        let name = entry.file_name();
        if let Some(stem) = name.to_string_lossy().strip_suffix(".rec")
            && let Ok(seq) = u64::from_str_radix(stem, 16)
        {
            sequences.push(seq);
        }
    }
    sequences.sort_unstable();
    Ok(sequences)
}

#[cfg(test)]
mod tests;
