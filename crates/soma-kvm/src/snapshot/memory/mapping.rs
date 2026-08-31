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

#[cfg(test)]
mod tests;

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

    /// Releases the mapped range to a caller that takes over unmapping it exactly once.
    ///
    /// The machine layer adopts the range so one owner registers it with KVM, serves guest
    /// accesses through it, and unmaps it after the VM is gone.
    #[must_use]
    pub fn into_raw(self) -> (*mut u8, usize) {
        let this = std::mem::ManuallyDrop::new(self);
        (this.base.as_ptr(), this.len)
    }

    /// Reads one byte of every page so the whole mapping is resident before it is used.
    ///
    /// This installs a present, read-only page-table entry for each page of the image and
    /// leaves every page shared with the page cache and with every other mapping of the same
    /// inode, so it costs no copy and no private memory. A later guest write still takes its
    /// own copy-on-write fault; only the cost of finding the page is paid up front.
    ///
    /// It is an eager cost, linear in the size of the image, and therefore belongs to whoever
    /// can pay it before a request arrives rather than on the launch path.
    ///
    /// Returns the number of pages touched.
    #[must_use]
    pub fn prefault(&self) -> usize {
        let stride = page_size();
        let mut touched = 0;
        let mut offset = 0;
        while offset < self.len {
            // SAFETY: `offset` is inside the mapping, which is `len` readable file-backed
            // bytes owned by `self`. The read is volatile so it cannot be elided, and it
            // cannot fault fatally because every byte of the range is backed by the file.
            let _ignored = unsafe { self.base.as_ptr().add(offset).read_volatile() };
            touched += 1;
            offset += stride;
        }
        touched
    }

    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        // SAFETY: as in `as_slice`, plus `&mut self` guarantees unique access to the pages
        // for the lifetime of the returned slice. Writes land in private copy-on-write pages.
        unsafe { std::slice::from_raw_parts_mut(self.base.as_ptr(), self.len) }
    }
}

/// The host page size, which is the stride a prefault must walk.
fn page_size() -> usize {
    // SAFETY: `sysconf` reads a constant of the running system and has no preconditions.
    let reported = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    usize::try_from(reported).unwrap_or(4096).max(1)
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
