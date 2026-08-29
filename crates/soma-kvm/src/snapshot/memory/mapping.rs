//! Private copy-on-write mapping of an immutable memory object (ADR 0002).
//!
//! This is the only unsafe surface of the snapshot codec.
//! The mapping owns its address range and releases it on drop.

use std::{
    error::Error,
    fmt,
    fs::File,
    os::fd::AsRawFd,
    ptr::{self, NonNull},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MappingOperation {
    Metadata,
    Mmap,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MappingError {
    ZeroLength,
    LengthExceedsAddressSpace(u64),
    NotRegularFile,
    FileShorterThanMapping {
        file_len: u64,
        requested: u64,
    },
    Io {
        operation: MappingOperation,
        errno: i32,
    },
}

impl fmt::Display for MappingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroLength => formatter.write_str("mapping length must be non-zero"),
            Self::LengthExceedsAddressSpace(length) => {
                write!(
                    formatter,
                    "mapping length {length} exceeds the address space"
                )
            }
            Self::NotRegularFile => formatter.write_str("memory object must be a regular file"),
            Self::FileShorterThanMapping {
                file_len,
                requested,
            } => write!(
                formatter,
                "memory object is {file_len} bytes but {requested} were requested"
            ),
            Self::Io { operation, errno } => {
                write!(formatter, "{operation:?} failed with errno {errno}")
            }
        }
    }
}

impl Error for MappingError {}

/// One `MAP_PRIVATE | MAP_NORESERVE` read-write mapping of a memory object.
///
/// Writes through the mapping create process-private pages and never reach the file or
/// any other mapping of the same inode.
#[derive(Debug)]
pub struct PrivateMapping {
    base: NonNull<u8>,
    len: usize,
}

impl PrivateMapping {
    /// Maps the first `len` bytes of `file` privately.
    ///
    /// The file must be a regular file at least `len` bytes long at the time of the call.
    /// The immutable-artifact lifecycle (ADR 0002) must prevent later shrinking or
    /// replacement; that guarantee is outside this type.
    ///
    /// # Errors
    ///
    /// Returns a typed [`MappingError`] for zero or oversized lengths, a non-regular or
    /// too-short file, or a failing `fstat` or `mmap`.
    pub fn map(file: &File, len: u64) -> Result<Self, MappingError> {
        if len == 0 {
            return Err(MappingError::ZeroLength);
        }
        let mapped_len = usize::try_from(len)
            .ok()
            .filter(|value| isize::try_from(*value).is_ok())
            .ok_or(MappingError::LengthExceedsAddressSpace(len))?;
        let metadata = file.metadata().map_err(|error| MappingError::Io {
            operation: MappingOperation::Metadata,
            errno: error.raw_os_error().unwrap_or(0),
        })?;
        if !metadata.is_file() {
            return Err(MappingError::NotRegularFile);
        }
        if metadata.len() < len {
            return Err(MappingError::FileShorterThanMapping {
                file_len: metadata.len(),
                requested: len,
            });
        }

        // SAFETY: `mmap` with a null hint lets the kernel choose an unused range, so no
        // existing Rust allocation is overwritten. `mapped_len` is non-zero and fits in
        // `isize`. The descriptor is a live regular file at least `len` bytes long, so every
        // byte inside the mapping is file-backed and touching it cannot raise SIGBUS at
        // mapping time. `MAP_PRIVATE` guarantees that writes never reach the file or another
        // mapping. The returned pointer is checked against `MAP_FAILED` before use.
        let raw = unsafe {
            libc::mmap(
                ptr::null_mut(),
                mapped_len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_NORESERVE,
                file.as_raw_fd(),
                0,
            )
        };
        if raw == libc::MAP_FAILED {
            return Err(MappingError::Io {
                operation: MappingOperation::Mmap,
                errno: std::io::Error::last_os_error().raw_os_error().unwrap_or(0),
            });
        }
        let base = NonNull::new(raw.cast::<u8>()).ok_or(MappingError::Io {
            operation: MappingOperation::Mmap,
            errno: 0,
        })?;
        Ok(Self {
            base,
            len: mapped_len,
        })
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Host address of the mapping, suitable for a later KVM memory-slot registration.
    #[must_use]
    pub const fn as_ptr(&self) -> *mut u8 {
        self.base.as_ptr()
    }

    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        // SAFETY: `base` points to `len` mapped, readable, file-backed bytes owned by
        // `self`. The mapping is private, so no other process can alter these pages, and the
        // immutable-artifact contract forbids modifying the backing file while mapped. The
        // borrow of `self` prevents a concurrent `as_mut_slice`.
        unsafe { std::slice::from_raw_parts(self.base.as_ptr(), self.len) }
    }

    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        // SAFETY: as in `as_slice`, plus `&mut self` guarantees unique access to the pages
        // for the lifetime of the returned slice. Writes land in private copy-on-write pages.
        unsafe { std::slice::from_raw_parts_mut(self.base.as_ptr(), self.len) }
    }
}

impl Drop for PrivateMapping {
    fn drop(&mut self) {
        // SAFETY: `base` and `len` are exactly the range returned by the successful `mmap`
        // in `map`, and it is unmapped only once because `Drop` runs once.
        unsafe {
            libc::munmap(self.base.as_ptr().cast(), self.len);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs::{self, File, OpenOptions},
        io::{Read as _, Seek as _, SeekFrom, Write as _},
        path::PathBuf,
        process,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::{MappingError, PrivateMapping};

    struct TempFile(PathBuf);

    impl TempFile {
        fn create(content: &[u8]) -> (Self, File) {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos());
            let path = std::env::temp_dir().join(format!(
                "soma-snapshot-mapping-{}-{nanos}.raw",
                process::id()
            ));
            let mut file = OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(&path)
                .unwrap();
            file.write_all(content).unwrap();
            file.sync_all().unwrap();
            (Self(path), file)
        }
    }

    impl Drop for TempFile {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }

    #[test]
    fn writes_through_one_private_mapping_are_invisible_to_a_sibling_and_the_file() {
        let original = vec![0x5a_u8; 8192];
        let (_guard, mut file) = TempFile::create(&original);
        let mut first = PrivateMapping::map(&file, 8192).unwrap();
        let second = PrivateMapping::map(&file, 8192).unwrap();
        assert_eq!(first.len(), 8192);
        assert!(!first.is_empty());
        assert_ne!(first.as_ptr(), second.as_ptr());
        assert_eq!(first.as_slice(), &original[..]);

        first.as_mut_slice()[0] = 0xa5;
        first.as_mut_slice()[4096] = 0xa5;
        assert_eq!(first.as_slice()[0], 0xa5);
        assert_eq!(second.as_slice(), &original[..]);

        let mut on_disk = Vec::new();
        file.seek(SeekFrom::Start(0)).unwrap();
        file.read_to_end(&mut on_disk).unwrap();
        assert_eq!(on_disk, original);
        drop(first);
        assert_eq!(second.as_slice(), &original[..]);
    }

    #[test]
    fn rejects_zero_length_and_short_files() {
        let (_guard, file) = TempFile::create(&[1; 4096]);
        assert_eq!(
            PrivateMapping::map(&file, 0).unwrap_err(),
            MappingError::ZeroLength
        );
        assert_eq!(
            PrivateMapping::map(&file, 4097).unwrap_err(),
            MappingError::FileShorterThanMapping {
                file_len: 4096,
                requested: 4097
            }
        );
        assert_eq!(
            PrivateMapping::map(&file, u64::MAX).unwrap_err(),
            MappingError::LengthExceedsAddressSpace(u64::MAX)
        );
        let directory = File::open(std::env::temp_dir()).unwrap();
        assert_eq!(
            PrivateMapping::map(&directory, 4096).unwrap_err(),
            MappingError::NotRegularFile
        );
    }
}
