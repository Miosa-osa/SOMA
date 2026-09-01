//! Entropy sources: the host CSPRNG and a deterministic test source.

use std::fmt;
use std::io;

/// Why entropy could not be produced; a launch failure before Ready.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EntropyError {
    /// The host source could not be opened or read.
    Unavailable(io::ErrorKind),
    /// The host source returned fewer bytes than requested.
    Short,
}

impl fmt::Display for EntropyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "host entropy failed: {self:?}")
    }
}

impl std::error::Error for EntropyError {}

impl From<io::Error> for EntropyError {
    fn from(error: io::Error) -> Self {
        Self::Unavailable(error.kind())
    }
}

/// A source that fills a buffer completely with fresh random bytes.
pub trait EntropyBackend {
    /// Fills all of `buf`.
    ///
    /// # Errors
    /// Returns the typed failure; the device then stops.
    fn fill(&mut self, buf: &mut [u8]) -> Result<(), EntropyError>;
}

/// The operating-system CSPRNG.
///
/// On Linux the bytes come from the `getrandom` syscall, so the source needs neither a path
/// nor a descriptor. That matters because a machine restored inside the jail has an empty
/// root with no `/dev/urandom` to open, and `getrandom` is admitted in both filter phases
/// while `open` is not. Elsewhere the source is a handle on `/dev/urandom`, opened once per
/// Instance. Bytes are never buffered, logged, or captured either way.
#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
pub struct OsEntropy {
    _private: (),
}

#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
impl OsEntropy {
    /// Names the host source; the syscall itself needs nothing opened.
    ///
    /// # Errors
    /// Never fails, and returns a `Result` so that both platform sources present one shape.
    pub const fn open() -> Result<Self, EntropyError> {
        Ok(Self { _private: () })
    }
}

#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
#[allow(unsafe_code)]
impl EntropyBackend for OsEntropy {
    fn fill(&mut self, buf: &mut [u8]) -> Result<(), EntropyError> {
        let mut filled = 0;
        while filled < buf.len() {
            let remainder = &mut buf[filled..];
            // SAFETY: the pointer and length describe valid writable storage.
            let taken =
                unsafe { libc::getrandom(remainder.as_mut_ptr().cast(), remainder.len(), 0) };
            match usize::try_from(taken) {
                // A short read is a signal interruption rather than exhaustion, so the loop
                // asks again; the kernel never returns zero for a non-empty buffer.
                Ok(0) => return Err(EntropyError::Short),
                Ok(count) => filled += count,
                Err(_) => {
                    let error = io::Error::last_os_error();
                    if error.kind() == io::ErrorKind::Interrupted {
                        continue;
                    }
                    return Err(EntropyError::Unavailable(error.kind()));
                }
            }
        }
        Ok(())
    }
}

/// The operating-system CSPRNG read from `/dev/urandom`.
///
/// Opened once per Instance; bytes are never buffered, logged, or captured.
#[cfg(all(
    unix,
    not(all(
        target_os = "linux",
        any(target_arch = "x86_64", target_arch = "aarch64")
    ))
))]
pub struct OsEntropy {
    source: std::fs::File,
}

#[cfg(all(
    unix,
    not(all(
        target_os = "linux",
        any(target_arch = "x86_64", target_arch = "aarch64")
    ))
))]
impl OsEntropy {
    /// Opens the host source.
    ///
    /// # Errors
    /// Returns the typed failure when the source cannot be opened.
    pub fn open() -> Result<Self, EntropyError> {
        Ok(Self {
            source: std::fs::File::open("/dev/urandom")?,
        })
    }
}

#[cfg(all(
    unix,
    not(all(
        target_os = "linux",
        any(target_arch = "x86_64", target_arch = "aarch64")
    ))
))]
impl EntropyBackend for OsEntropy {
    fn fill(&mut self, buf: &mut [u8]) -> Result<(), EntropyError> {
        use std::io::Read;
        self.source
            .read_exact(buf)
            .map_err(|error| match error.kind() {
                io::ErrorKind::UnexpectedEof => EntropyError::Short,
                kind => EntropyError::Unavailable(kind),
            })
    }
}

/// A deterministic counter source for tests only; never fresh entropy.
#[cfg(test)]
#[derive(Clone, Debug, Default)]
pub(crate) struct CounterEntropy {
    pub next: u8,
    pub fail: Option<EntropyError>,
    pub bytes_served: u64,
}

#[cfg(test)]
impl EntropyBackend for CounterEntropy {
    fn fill(&mut self, buf: &mut [u8]) -> Result<(), EntropyError> {
        if let Some(error) = self.fail {
            return Err(error);
        }
        for byte in buf.iter_mut() {
            *byte = self.next;
            self.next = self.next.wrapping_add(1);
        }
        self.bytes_served += u64::try_from(buf.len()).unwrap_or(u64::MAX);
        Ok(())
    }
}
