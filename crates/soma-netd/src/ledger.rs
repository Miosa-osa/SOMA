//! The durable append-only ownership ledger.
//!
//! Every record is one create-exclusive file named by bundle identity and cleanup generation.
//! Content is written to a private temporary file, synced, and then hard-linked into its
//! final name, so a record is either absent or complete and durable.
//! Records are never rewritten; release appends a second record kind beside the assignment.

use std::{
    fmt::Write as _,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

use crate::{BundleId, CleanupGeneration, Error, Step};

mod record;

pub use record::{AssignmentRecord, MAX_RECORD};

/// The result of recording one assignment.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecordOutcome {
    /// The record was durably created now.
    Recorded,
    /// An identical assignment already existed; nothing was written.
    Replayed,
}

/// One assignment together with its release state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LedgerEntry {
    /// The durable assignment.
    pub record: AssignmentRecord,
    /// Whether a release record exists for the same bundle and generation.
    pub released: bool,
}

/// One ledger directory.
#[derive(Debug)]
pub struct Ledger {
    root: PathBuf,
}

impl Ledger {
    /// Opens or creates the ledger directory.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Kernel`] at [`Step::LedgerOpen`] when the directory cannot be created.
    pub fn open(root: &Path) -> Result<Self, Error> {
        fs::create_dir_all(root).map_err(|error| Error::io(Step::LedgerOpen, &error))?;
        Ok(Self {
            root: root.to_path_buf(),
        })
    }

    /// Records one assignment exactly once per bundle and generation.
    ///
    /// # Errors
    ///
    /// Returns [`Error::LedgerConflict`] when a different operation owns the slot,
    /// [`Error::ReplayMismatch`] when the same operation replays a changed intent, and
    /// [`Error::LedgerCorrupt`] when the existing record cannot be verified.
    pub fn record_assignment(&self, record: &AssignmentRecord) -> Result<RecordOutcome, Error> {
        let path = self.path(record.bundle, record.generation, "assigned");
        match self.publish(&path, &record.encode()) {
            Ok(()) => Ok(RecordOutcome::Recorded),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                let existing = read_record(&path)?;
                if existing.replays(record) {
                    Ok(RecordOutcome::Replayed)
                } else if existing.operation == record.operation {
                    Err(Error::ReplayMismatch)
                } else {
                    Err(Error::LedgerConflict)
                }
            }
            Err(error) => Err(Error::io(Step::LedgerWrite, &error)),
        }
    }

    /// Records the release of one assignment; a repeated release is a no-op.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NotAssigned`] when no assignment exists.
    pub fn record_release(
        &self,
        bundle: BundleId,
        generation: CleanupGeneration,
    ) -> Result<(), Error> {
        if !self.path(bundle, generation, "assigned").exists() {
            return Err(Error::NotAssigned);
        }
        let path = self.path(bundle, generation, "released");
        match self.publish(&path, b"released\n") {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(()),
            Err(error) => Err(Error::io(Step::LedgerWrite, &error)),
        }
    }

    /// Looks up one bundle and generation.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NotAssigned`] when absent or [`Error::LedgerCorrupt`] when unreadable.
    pub fn lookup(
        &self,
        bundle: BundleId,
        generation: CleanupGeneration,
    ) -> Result<LedgerEntry, Error> {
        let path = self.path(bundle, generation, "assigned");
        if !path.exists() {
            return Err(Error::NotAssigned);
        }
        Ok(LedgerEntry {
            record: read_record(&path)?,
            released: self.path(bundle, generation, "released").exists(),
        })
    }

    /// Lists every assignment, sorted by file name; a corrupt record fails the listing.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Kernel`] at [`Step::LedgerRead`] or [`Error::LedgerCorrupt`].
    pub fn entries(&self) -> Result<Vec<LedgerEntry>, Error> {
        let mut names = Vec::new();
        for entry in
            fs::read_dir(&self.root).map_err(|error| Error::io(Step::LedgerRead, &error))?
        {
            let entry = entry.map_err(|error| Error::io(Step::LedgerRead, &error))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if let Some(stem) = name.strip_suffix(".assigned") {
                names.push(stem.to_owned());
            }
        }
        names.sort_unstable();
        names
            .iter()
            .map(|stem| {
                let record = read_record(&self.root.join(format!("{stem}.assigned")))?;
                let released = self.root.join(format!("{stem}.released")).exists();
                Ok(LedgerEntry { record, released })
            })
            .collect()
    }

    fn path(&self, bundle: BundleId, generation: CleanupGeneration, kind: &str) -> PathBuf {
        let hex = bundle
            .as_bytes()
            .iter()
            .fold(String::new(), |mut hex, byte| {
                let _ = write!(hex, "{byte:02x}");
                hex
            });
        self.root
            .join(format!("{hex}-{:08x}.{kind}", generation.get()))
    }

    fn publish(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        let temp = self.root.join(format!(
            ".tmp-{}-{}",
            std::process::id(),
            path.file_name()
                .map(|n| n.to_string_lossy())
                .unwrap_or_default()
        ));
        let result = write_temp(&temp, bytes).and_then(|()| fs::hard_link(&temp, path));
        let _ = fs::remove_file(&temp);
        result?;
        #[cfg(unix)]
        {
            File::open(&self.root)?.sync_all()?;
        }
        Ok(())
    }
}

fn write_temp(temp: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(temp)?;
    file.write_all(bytes)?;
    file.sync_all()
}

fn read_record(path: &Path) -> Result<AssignmentRecord, Error> {
    let mut file = File::open(path).map_err(|error| Error::io(Step::LedgerRead, &error))?;
    let mut bytes = Vec::with_capacity(MAX_RECORD);
    Read::by_ref(&mut file)
        .take(MAX_RECORD as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| Error::io(Step::LedgerRead, &error))?;
    AssignmentRecord::decode(&bytes)
}

#[cfg(test)]
mod tests {
    use super::{record::tests::record, *};

    #[test]
    fn assignment_is_recorded_once_replayed_exactly_and_conflicts_otherwise() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ledger = Ledger::open(dir.path()).expect("ledger");
        let first = record(4, 33);
        assert_eq!(
            ledger.record_assignment(&first).expect("recorded"),
            RecordOutcome::Recorded
        );
        assert_eq!(
            ledger.record_assignment(&first).expect("replayed"),
            RecordOutcome::Replayed
        );
        assert_eq!(
            ledger
                .record_assignment(&record(4, 34))
                .expect_err("changed"),
            Error::ReplayMismatch
        );
        assert_eq!(
            ledger
                .record_assignment(&record(5, 33))
                .expect_err("other op"),
            Error::LedgerConflict
        );
        let entry = ledger
            .lookup(first.bundle, first.generation)
            .expect("present");
        assert_eq!(entry.record, first);
        assert!(!entry.released);
        ledger
            .record_release(first.bundle, first.generation)
            .expect("released");
        ledger
            .record_release(first.bundle, first.generation)
            .expect("idempotent");
        assert!(ledger.entries().expect("list")[0].released);
        assert_eq!(
            ledger.record_release(first.bundle, CleanupGeneration::new(9).expect("g")),
            Err(Error::NotAssigned)
        );
        assert_eq!(fs::read_dir(dir.path()).expect("dir").count(), 2);
    }

    #[test]
    fn corrupt_records_fail_closed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ledger = Ledger::open(dir.path()).expect("ledger");
        let first = record(4, 33);
        ledger.record_assignment(&first).expect("recorded");
        let path = ledger.path(first.bundle, first.generation, "assigned");
        fs::write(&path, b"SOMANETLjunk").expect("overwrite");
        assert_eq!(
            ledger.lookup(first.bundle, first.generation),
            Err(Error::LedgerCorrupt)
        );
        assert_eq!(ledger.entries(), Err(Error::LedgerCorrupt));
        assert_eq!(
            ledger.record_assignment(&first).expect_err("cannot verify"),
            Error::LedgerCorrupt
        );
    }
}
