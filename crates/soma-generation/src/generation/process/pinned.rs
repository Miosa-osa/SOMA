//! One external tool pinned to the exact bytes that will execute.
//!
//! The compiler used to hash a program by path and then spawn that same path, so the bytes it
//! measured and the bytes the kernel loaded were related only by a name the build host can
//! repoint between the two operations.
//! A pinned tool is opened once, hashed through that one open file description, and executed
//! through that same description, so the digest bound into evidence is the digest of the
//! process image that actually ran.
//!
//! Execution names the descriptor rather than the original path.
//! Opening `/proc/self/fd/N` re-opens the object the descriptor already holds, and the child
//! inherits the descriptor across the fork, so renaming or replacing the tool between the hash
//! and the spawn cannot change what runs.
//! The kernel resolves that path before it applies close-on-exec, which is why the descriptor
//! stays close-on-exec and never leaks into a tool.
//!
//! Opening the descriptor path is proved to reach the same object before the tool is used, so a
//! system without that mapping fails as unsupported instead of silently running a path again.

use std::{
    fs::File,
    io::Read as _,
    path::{Path, PathBuf},
};

use sha2::{Digest as _, Sha256};

use super::{
    super::{
        artifacts::Sha256Digest,
        error::{CompileError, CompileErrorKind, CompilePhase},
    },
    control,
};

/// Largest executable the compiler will hash and run.
const MAX_TOOL_BYTES: u64 = 256 * 1024 * 1024;
const READ_CHUNK: usize = 64 * 1024;

/// One external tool held open, measured, and bound to execute exactly as measured.
#[derive(Debug)]
pub(crate) struct PinnedTool {
    name: String,
    digest: Sha256Digest,
    program: PathBuf,
    // The descriptor the program path names; dropping it invalidates that path.
    file: File,
}

impl PinnedTool {
    /// Opens one tool, proves it is a regular file, hashes it through that descriptor, and
    /// proves the descriptor path reaches the same object.
    ///
    /// # Errors
    ///
    /// Returns [`CompileErrorKind::Toolchain`] when the path is missing, is not a regular file,
    /// or is empty, [`CompileErrorKind::Io`] when it cannot be read,
    /// [`CompileErrorKind::LimitExceeded`] beyond [`MAX_TOOL_BYTES`], and
    /// [`CompileErrorKind::Unsupported`] where descriptors have no path.
    pub(crate) fn open(program: &Path, phase: CompilePhase) -> Result<Self, CompileError> {
        let name = program
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .filter(|name| !name.is_empty())
            .ok_or_else(|| CompileError::new(phase, CompileErrorKind::Toolchain))?;
        let mut file = File::open(program)
            .map_err(|_| CompileError::new(phase, CompileErrorKind::Toolchain))?;
        let regular = file
            .metadata()
            .map_err(|_| CompileError::new(phase, CompileErrorKind::Io))?
            .is_file();
        if !regular {
            return Err(CompileError::new(phase, CompileErrorKind::Toolchain));
        }
        let digest = digest_of(&mut file, phase)?;
        let program = control::descriptor_path(&file);
        control::require_same_object(&file, &program)
            .map_err(|()| CompileError::new(phase, CompileErrorKind::Unsupported))?;
        Ok(Self {
            name,
            digest,
            program,
            file,
        })
    }

    /// Returns the program name without its host directory.
    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    /// Returns the digest of the exact bytes this tool executes.
    pub(crate) const fn digest(&self) -> Sha256Digest {
        self.digest
    }

    /// Returns the descriptor path a child must execute to run the measured bytes.
    pub(crate) fn program(&self) -> &Path {
        &self.program
    }

    /// Returns the descriptor number the child must keep open to run the measured bytes.
    #[cfg(unix)]
    pub(crate) fn descriptor(&self) -> std::os::unix::io::RawFd {
        use std::os::unix::io::AsRawFd as _;

        self.file.as_raw_fd()
    }

    /// Returns a descriptor number no system without descriptor paths can use.
    #[cfg(not(unix))]
    pub(crate) const fn descriptor(&self) -> i32 {
        -1
    }

    /// Proves the program path still names the measured object.
    ///
    /// This runs immediately before every spawn, so a descriptor path that stopped resolving to
    /// the measured bytes fails the invocation instead of executing something else.
    ///
    /// # Errors
    ///
    /// Returns [`CompileErrorKind::Toolchain`] when the path no longer names the descriptor.
    pub(crate) fn require_bound(&self, phase: CompilePhase) -> Result<(), CompileError> {
        control::require_same_object(&self.file, &self.program)
            .map_err(|()| CompileError::new(phase, CompileErrorKind::Toolchain))
    }
}

fn digest_of(file: &mut File, phase: CompilePhase) -> Result<Sha256Digest, CompileError> {
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; READ_CHUNK];
    let mut total = 0_u64;
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|_| CompileError::new(phase, CompileErrorKind::Io))?;
        if count == 0 {
            break;
        }
        total = u64::try_from(count)
            .ok()
            .and_then(|count| total.checked_add(count))
            .ok_or_else(|| CompileError::new(phase, CompileErrorKind::Io))?;
        if total > MAX_TOOL_BYTES {
            return Err(CompileError::new(phase, CompileErrorKind::LimitExceeded));
        }
        hasher.update(&buffer[..count]);
    }
    if total == 0 {
        return Err(CompileError::new(phase, CompileErrorKind::Toolchain));
    }
    let mut digest = [0_u8; 32];
    digest.copy_from_slice(hasher.finalize().as_ref());
    Ok(Sha256Digest::from_bytes(digest))
}
