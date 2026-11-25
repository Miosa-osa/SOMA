//! Opening the durable volume file one launch binds, by the host's own path.
//!
//! It is the one exception to this module's rule that nothing resolves a path. The volume store
//! derives the path from its own root and certifies it (its mount, the volume's state, and this
//! start's repair) before the assignment carries it, so the path is host authority rather than a
//! caller's choice: a launch cannot name a file of its own.
//!
//! The descriptor is opened `O_CLOEXEC`, so a worker that later executes anything does not hand
//! the tenant's volume on. A machine whose file cannot be opened refuses to start rather than
//! serve an empty disk where the data belongs.

use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::Path;

/// Opens one volume file read-write, by the path the assignment carried, and takes its exclusive
/// lock for as long as the machine holds the descriptor.
///
/// The lock is the volume store's proof that a live machine owns the file: its destroy, detach and
/// repair each try it first and refuse while it is held. The sealed launch path locks the same
/// way before it hands the descriptor over.
///
/// # Errors
///
/// Returns the error of the failed open, or `WouldBlock` when another holder has the lock; the
/// caller refuses the launch rather than proceeding.
pub(super) fn open(path: &Path) -> io::Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_CLOEXEC)
        .open(path)?;
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(std::fs::TryLockError::WouldBlock) => Err(io::ErrorKind::WouldBlock.into()),
        Err(std::fs::TryLockError::Error(error)) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_that_holds_a_file_opens_read_write_and_one_that_does_not_is_refused() {
        let directory = tempfile::tempdir().expect("temp dir");
        let file = directory.path().join("volume.img");
        std::fs::write(&file, b"a volume").expect("write");
        let opened = open(&file).expect("opens");
        assert!(opened.metadata().expect("metadata").len() > 0);
        // Read-write: the machine writes through this descriptor, so a read-only open would be
        // a launch that cannot store anything.
        let mut reader = &opened;
        let mut contents = String::new();
        assert!(
            std::io::Read::read_to_string(&mut reader, &mut contents).is_ok(),
            "the file opens read-write"
        );
        assert_eq!(contents, "a volume");

        assert!(
            open(&directory.path().join("absent.img")).is_err(),
            "a missing volume refuses the launch rather than serving an empty disk"
        );
    }

    #[test]
    fn an_opened_volume_holds_the_store_lock_until_the_machine_lets_it_go() {
        let directory = tempfile::tempdir().expect("temp dir");
        let file = directory.path().join("volume.img");
        std::fs::write(&file, b"a volume").expect("write");
        let opened = open(&file).expect("opens");
        // The volume store's destroy, detach and repair each take this same lock first, and
        // refuse while a live machine holds it.
        let store = std::fs::File::open(&file).expect("store handle");
        assert!(
            matches!(store.try_lock(), Err(std::fs::TryLockError::WouldBlock)),
            "a running machine's volume is locked against the store"
        );
        assert!(
            open(&file).is_err(),
            "a second machine is refused a volume another one holds"
        );
        drop(opened);
        store
            .try_lock()
            .expect("the lock goes with the machine's descriptor");
    }
}
