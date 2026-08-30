//! Block backends accept only validated byte ranges from the request parser.

use std::fmt;

#[cfg(unix)]
use std::fs::File;
use std::io;

/// Why a backend operation failed; carries no host path or descriptor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackendError {
    /// The host I/O failed with this kind.
    Io(io::ErrorKind),
    /// The store is read-only.
    ReadOnly,
    /// The validated range is outside the store; a device bug if it happens.
    OutOfRange,
}

impl fmt::Display for BackendError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "block backend failed: {self:?}")
    }
}

impl std::error::Error for BackendError {}

impl From<io::Error> for BackendError {
    fn from(error: io::Error) -> Self {
        Self::Io(error.kind())
    }
}

/// A byte-addressed store behind one block device.
///
/// Callers pass only ranges already checked against `capacity_bytes`; an
/// implementation still rejects anything outside its store.
pub trait BlockBackend {
    /// Total bytes; a multiple of the sector size.
    fn capacity_bytes(&self) -> u64;
    /// Whether writes and flushes are refused.
    fn read_only(&self) -> bool;
    /// Fills `buf` from `offset`; returns bytes read (short reads are errors upstream).
    ///
    /// # Errors
    /// Returns the typed failure.
    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<usize, BackendError>;
    /// Writes `data` at `offset`; returns bytes written.
    ///
    /// # Errors
    /// Returns the typed failure.
    fn write_at(&mut self, offset: u64, data: &[u8]) -> Result<usize, BackendError>;
    /// Makes completed writes durable.
    ///
    /// # Errors
    /// Returns the typed failure.
    fn flush(&mut self) -> Result<(), BackendError>;
}

fn in_range(capacity: u64, offset: u64, len: usize) -> Result<(), BackendError> {
    let len = u64::try_from(len).map_err(|_| BackendError::OutOfRange)?;
    match offset.checked_add(len) {
        Some(end) if end <= capacity => Ok(()),
        _ => Err(BackendError::OutOfRange),
    }
}

/// Positional I/O on a raw image file.
///
/// The capacity is the file length rounded down to whole sectors at open
/// time; the device never grows the file.
#[cfg(unix)]
pub struct FileBackend {
    file: File,
    capacity: u64,
    read_only: bool,
}

#[cfg(unix)]
impl FileBackend {
    /// Wraps an already-open file; `read_only` refuses writes regardless of
    /// how the file was opened.
    ///
    /// # Errors
    /// Fails when the length cannot be read.
    pub fn new(file: File, read_only: bool) -> io::Result<Self> {
        let len = file.metadata()?.len();
        let capacity = len - len % super::request::SECTOR_SIZE;
        Ok(Self {
            file,
            capacity,
            read_only,
        })
    }
}

#[cfg(unix)]
impl BlockBackend for FileBackend {
    fn capacity_bytes(&self) -> u64 {
        self.capacity
    }

    fn read_only(&self) -> bool {
        self.read_only
    }

    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<usize, BackendError> {
        use std::os::unix::fs::FileExt;
        in_range(self.capacity, offset, buf.len())?;
        self.file.read_exact_at(buf, offset)?;
        Ok(buf.len())
    }

    fn write_at(&mut self, offset: u64, data: &[u8]) -> Result<usize, BackendError> {
        use std::os::unix::fs::FileExt;
        if self.read_only {
            return Err(BackendError::ReadOnly);
        }
        in_range(self.capacity, offset, data.len())?;
        self.file.write_all_at(data, offset)?;
        Ok(data.len())
    }

    fn flush(&mut self) -> Result<(), BackendError> {
        if self.read_only {
            return Err(BackendError::ReadOnly);
        }
        self.file.sync_data()?;
        Ok(())
    }
}

/// A host-heap store for tests; never used in production.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MemoryBackend {
    pub bytes: Vec<u8>,
    pub read_only: bool,
    pub flushes: u32,
    /// When set, reads return this many bytes fewer than asked.
    pub short_by: usize,
    /// When set, every operation fails with this kind.
    pub fail: Option<io::ErrorKind>,
}

impl MemoryBackend {
    /// A zero-filled store of `sectors` sectors.
    #[must_use]
    pub fn zeroed(sectors: usize, read_only: bool) -> Self {
        Self {
            bytes: vec![0; sectors * 512],
            read_only,
            ..Self::default()
        }
    }
}

impl BlockBackend for MemoryBackend {
    fn capacity_bytes(&self) -> u64 {
        u64::try_from(self.bytes.len()).unwrap_or(u64::MAX)
    }

    fn read_only(&self) -> bool {
        self.read_only
    }

    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<usize, BackendError> {
        if let Some(kind) = self.fail {
            return Err(BackendError::Io(kind));
        }
        in_range(self.capacity_bytes(), offset, buf.len())?;
        let start = usize::try_from(offset).map_err(|_| BackendError::OutOfRange)?;
        let take = buf.len().saturating_sub(self.short_by);
        buf[..take].copy_from_slice(&self.bytes[start..start + take]);
        Ok(take)
    }

    fn write_at(&mut self, offset: u64, data: &[u8]) -> Result<usize, BackendError> {
        if let Some(kind) = self.fail {
            return Err(BackendError::Io(kind));
        }
        if self.read_only {
            return Err(BackendError::ReadOnly);
        }
        in_range(self.capacity_bytes(), offset, data.len())?;
        let start = usize::try_from(offset).map_err(|_| BackendError::OutOfRange)?;
        let take = data.len().saturating_sub(self.short_by);
        self.bytes[start..start + take].copy_from_slice(&data[..take]);
        Ok(take)
    }

    fn flush(&mut self) -> Result<(), BackendError> {
        if let Some(kind) = self.fail {
            return Err(BackendError::Io(kind));
        }
        if self.read_only {
            return Err(BackendError::ReadOnly);
        }
        self.flushes = self.flushes.saturating_add(1);
        Ok(())
    }
}

/// A backend that declares a store's shape while holding no store at all.
///
/// A prepared worker is built before any Instance exists, and the prepared worker protocol
/// forbids it from holding a private disk head.
/// The device can still be constructed from the admitted capacity and writability.
/// Every I/O operation fails closed because an unassigned worker must never run a vCPU.
#[derive(Clone, Copy, Debug)]
#[cfg(any(test, all(target_os = "linux", target_arch = "x86_64")))]
pub struct Detached {
    capacity_bytes: u64,
    read_only: bool,
}

#[cfg(any(test, all(target_os = "linux", target_arch = "x86_64")))]
impl Detached {
    /// Declares the shape of the store that will be attached later.
    #[must_use]
    pub const fn new(capacity_bytes: u64, read_only: bool) -> Self {
        Self {
            capacity_bytes,
            read_only,
        }
    }
}

#[cfg(any(test, all(target_os = "linux", target_arch = "x86_64")))]
impl BlockBackend for Detached {
    fn capacity_bytes(&self) -> u64 {
        self.capacity_bytes
    }

    fn read_only(&self) -> bool {
        self.read_only
    }

    fn read_at(&mut self, _offset: u64, _buf: &mut [u8]) -> Result<usize, BackendError> {
        Err(BackendError::OutOfRange)
    }

    fn write_at(&mut self, _offset: u64, _data: &[u8]) -> Result<usize, BackendError> {
        Err(BackendError::OutOfRange)
    }

    fn flush(&mut self) -> Result<(), BackendError> {
        Err(BackendError::OutOfRange)
    }
}
