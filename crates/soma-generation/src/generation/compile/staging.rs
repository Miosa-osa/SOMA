//! The private staging directory one compilation uses for formatter output.
//!
//! The directory is created exclusively and removed on every exit path, so no build ever reads
//! or writes another build's formatter output.
//! A directory left behind by a killed process is never reused: the next build advances to the
//! next unused name instead, which keeps a repeated process identifier from blocking a build.

use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use crate::generation::error::{CompileError, CompileErrorKind, CompilePhase};

/// Names tried before a build gives up on finding an unused staging directory.
const MAX_ATTEMPTS: u64 = 1024;

static STAGING_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(super) struct Staging {
    pub(super) path: PathBuf,
}

impl Staging {
    pub(super) fn create(parent: &Path) -> Result<Self, CompileError> {
        let process = std::process::id();
        for _ in 0..MAX_ATTEMPTS {
            let sequence = STAGING_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = parent.join(format!("soma-generation-{process}-{sequence}"));
            return match fs::create_dir(&path) {
                Ok(()) => Ok(Self { path }),
                Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
                Err(_) => Err(io_error()),
            };
        }
        Err(io_error())
    }
}

const fn io_error() -> CompileError {
    CompileError::new(CompilePhase::ResolveInputs, CompileErrorKind::Io)
}

impl Drop for Staging {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::Staging;

    #[test]
    fn a_directory_left_by_a_killed_build_is_never_reused() {
        let parent = tempfile::tempdir().expect("scratch parent");
        let first = Staging::create(parent.path()).expect("first staging directory");
        let taken = first.path.clone();
        std::mem::forget(first);

        let second = Staging::create(parent.path()).expect("second staging directory");

        assert!(taken.is_dir(), "the stale directory must be left untouched");
        assert_ne!(second.path, taken);
        assert!(second.path.is_dir());
        let path = second.path.clone();
        drop(second);
        assert!(!path.exists(), "a finished build removes its own directory");
        std::fs::remove_dir_all(&taken).expect("clean up the stale directory");
    }
}
