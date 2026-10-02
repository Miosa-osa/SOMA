use std::{
    fs::File,
    io::{self, BufRead, BufReader, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

use crate::runner::journal::{OffsetOnly, corrupt};

/// One shipped line: its offset, where it starts in the file, and its bytes with the newline.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShippedLine {
    pub offset: u64,
    pub position: u64,
    pub bytes: Vec<u8>,
}

/// A sequential reader over the journal for the shipper.
///
/// It keeps its byte position, so shipping does not rescan the file. Repositioning to an
/// arbitrary offset scans from the start once, which only happens at startup and when the
/// control plane acknowledges less than a whole batch.
pub struct Reader {
    path: PathBuf,
    position: u64,
}

impl Reader {
    /// A reader positioned at the first line whose offset is greater than `acked`.
    ///
    /// # Errors
    ///
    /// Returns the failure to open or read the journal.
    pub fn after(path: &Path, acked: u64) -> io::Result<Self> {
        let mut reader = Self {
            path: path.to_owned(),
            position: 0,
        };
        reader.seek_after(acked)?;
        Ok(reader)
    }

    /// Moves to the first line whose offset is greater than `acked`, scanning from the start.
    ///
    /// # Errors
    ///
    /// Returns the failure to read the journal.
    pub fn seek_after(&mut self, acked: u64) -> io::Result<()> {
        let mut file = BufReader::new(File::open(&self.path)?);
        let mut position = 0;
        let mut line = Vec::new();
        loop {
            line.clear();
            let read = file.read_until(b'\n', &mut line)?;
            if read == 0 || line.last() != Some(&b'\n') {
                break;
            }
            let entry: OffsetOnly = serde_json::from_slice(&line)
                .map_err(|_| corrupt("a journal line carries no offset"))?;
            if entry.offset > acked {
                break;
            }
            position += read as u64;
        }
        self.position = position;
        Ok(())
    }

    /// Moves to a position a previous [`Self::batch`] reported.
    pub const fn seek_to(&mut self, position: u64) {
        self.position = position;
    }

    /// Reads up to `max_lines` complete lines whose offsets are at most `durable`.
    ///
    /// # Errors
    ///
    /// Returns the failure to read the journal.
    pub fn batch(&mut self, max_lines: usize, durable: u64) -> io::Result<Vec<ShippedLine>> {
        let mut file = File::open(&self.path)?;
        file.seek(SeekFrom::Start(self.position))?;
        let mut file = BufReader::new(file);
        let mut lines = Vec::new();
        let mut position = self.position;
        while lines.len() < max_lines {
            let mut bytes = Vec::new();
            let read = file.read_until(b'\n', &mut bytes)?;
            if read == 0 || bytes.last() != Some(&b'\n') {
                break;
            }
            let entry: OffsetOnly = serde_json::from_slice(&bytes)
                .map_err(|_| corrupt("a journal line carries no offset"))?;
            if entry.offset > durable {
                break;
            }
            lines.push(ShippedLine {
                offset: entry.offset,
                position,
                bytes,
            });
            position += read as u64;
        }
        self.position = position;
        Ok(lines)
    }
}

/// The shipper's record of the highest offset the control plane has stored.
pub struct AckFile {
    path: PathBuf,
}

impl AckFile {
    #[must_use]
    pub fn beside(journal: &Path) -> Self {
        Self {
            path: journal.with_extension("acked"),
        }
    }

    /// The stored acknowledgement, or 0 when nothing was ever acknowledged.
    ///
    /// # Errors
    ///
    /// Returns a read failure other than the file being absent, or unparseable contents.
    pub fn load(&self) -> io::Result<u64> {
        match std::fs::read_to_string(&self.path) {
            Ok(contents) => contents
                .trim()
                .parse()
                .map_err(|_| corrupt("the acknowledgement file is not a number")),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(0),
            Err(error) => Err(error),
        }
    }

    /// Replaces the stored acknowledgement atomically.
    ///
    /// # Errors
    ///
    /// Returns the write, sync, or rename failure.
    pub fn store(&self, acked: u64) -> io::Result<()> {
        let temporary = self.path.with_extension("acked.tmp");
        let mut file = File::create(&temporary)?;
        file.write_all(acked.to_string().as_bytes())?;
        file.sync_data()?;
        std::fs::rename(&temporary, &self.path)
    }
}
