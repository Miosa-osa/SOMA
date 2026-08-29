//! Destruction of a released head: unlink under the capability directory, then sync the
//! directory so the removal is durable before the ledger forgets the assignment.

use std::fmt;
use std::fs::File;
use std::io;
use std::os::fd::BorrowedFd;

use cap_std::fs::Dir;

use crate::head::{HeadName, HeadToken};
use crate::lease::{HeadLedger, LeaseError};

/// What release found in the directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReleaseOutcome {
    /// The head existed and was unlinked.
    Destroyed(HeadName),
    /// The head was already absent; the ledger still retires the token.
    AlreadyAbsent(HeadName),
}

/// Why a release failed; the ledger is unchanged on every variant.
#[derive(Debug)]
pub enum ReleaseError {
    /// The ledger refused the token.
    Lease(LeaseError),
    /// The unlink failed for a reason other than absence.
    Unlink(HeadName, io::Error),
    /// The directory could not be reopened or synced.
    DirSync(io::Error),
}

impl fmt::Display for ReleaseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Lease(error) => write!(f, "release refused: {error}"),
            Self::Unlink(name, error) => write!(f, "unlink of {name} failed: {error}"),
            Self::DirSync(error) => write!(f, "directory sync failed: {error}"),
        }
    }
}

impl std::error::Error for ReleaseError {}

/// Unlinks the head owned by `token` under `dir`, syncs the directory, and retires the token.
///
/// # Errors
///
/// Returns the first failing step without retiring the token, so the caller can retry.
pub fn release_head(
    ledger: &mut HeadLedger,
    dir: BorrowedFd<'_>,
    token: HeadToken,
) -> Result<ReleaseOutcome, ReleaseError> {
    let name = ledger
        .assigned_name(token)
        .cloned()
        .ok_or(ReleaseError::Lease(LeaseError::UnknownToken(token)))?;
    let handle = reopen(dir).map_err(ReleaseError::DirSync)?;
    let outcome = match handle.remove_file(name.as_str()) {
        Ok(()) => ReleaseOutcome::Destroyed(name.clone()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            ReleaseOutcome::AlreadyAbsent(name.clone())
        }
        Err(error) => return Err(ReleaseError::Unlink(name, error)),
    };
    sync_dir(dir).map_err(ReleaseError::DirSync)?;
    ledger.release(token).map_err(ReleaseError::Lease)?;
    Ok(outcome)
}

/// Reopens the directory descriptor as a capability directory handle.
pub(crate) fn reopen(dir: BorrowedFd<'_>) -> io::Result<Dir> {
    Ok(Dir::from_std_file(File::from(dir.try_clone_to_owned()?)))
}

/// `fsync` of the directory through a duplicated descriptor.
pub(crate) fn sync_dir(dir: BorrowedFd<'_>) -> io::Result<()> {
    File::from(dir.try_clone_to_owned()?).sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::fd::AsFd;

    fn token(byte: u8) -> HeadToken {
        HeadToken::new([byte; 16]).expect("non-zero")
    }

    #[test]
    fn destroys_the_owned_head_and_retires_the_token() {
        let temp = tempfile::tempdir().expect("tempdir");
        let dir = File::open(temp.path()).expect("open dir");
        let mut ledger = HeadLedger::new();
        let name = token(1).head_name();
        ledger.lease(token(1), name.clone()).expect("lease");
        std::fs::write(temp.path().join(name.as_str()), b"head").expect("write head");

        let outcome = release_head(&mut ledger, dir.as_fd(), token(1)).expect("release");
        assert_eq!(outcome, ReleaseOutcome::Destroyed(name.clone()));
        assert!(!temp.path().join(name.as_str()).exists());
        assert_eq!(ledger.assigned_count(), 0);
        assert!(ledger.is_retired_name(&name));
    }

    #[test]
    fn absent_head_is_reported_and_still_retired() {
        let temp = tempfile::tempdir().expect("tempdir");
        let dir = File::open(temp.path()).expect("open dir");
        let mut ledger = HeadLedger::new();
        let name = token(2).head_name();
        ledger.lease(token(2), name.clone()).expect("lease");
        let outcome = release_head(&mut ledger, dir.as_fd(), token(2)).expect("release");
        assert_eq!(outcome, ReleaseOutcome::AlreadyAbsent(name));
        assert_eq!(ledger.assigned_count(), 0);
    }

    #[test]
    fn unknown_token_changes_nothing() {
        let temp = tempfile::tempdir().expect("tempdir");
        let dir = File::open(temp.path()).expect("open dir");
        let mut ledger = HeadLedger::new();
        let error = release_head(&mut ledger, dir.as_fd(), token(3)).expect_err("unknown");
        assert!(matches!(
            error,
            ReleaseError::Lease(LeaseError::UnknownToken(_))
        ));
        assert_eq!(ledger, HeadLedger::new());
    }
}
