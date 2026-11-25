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
use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _};
use std::path::Path;
use std::time::{Duration, Instant};

/// How long a launch waits for the volume store to let go of the file's lock.
///
/// The store takes the lock only for the length of one repair, detach or destroy, each done under
/// its own mutex, so a lock still held after this is a live machine's, and the launch is refused.
/// Two creates of one sandbox at once made the first one's machine fail on the second one's
/// repair, so neither won (SOMA#234).
pub const VOLUME_LOCK_WAIT: Duration = Duration::from_secs(10);

/// Opens one volume file read-write, by the path the assignment carried, and takes its exclusive
/// lock for as long as the machine holds the descriptor.
///
/// The lock is the volume store's proof that a live machine owns the file: its destroy, detach and
/// repair each try it first and refuse while it is held. The sealed launch path locks the same
/// way before it hands the descriptor over.
///
/// # Errors
///
/// Returns the error of the failed open, or the refusal of [`lock_volume`]; the caller refuses
/// the launch rather than proceeding.
pub(super) fn open(path: &Path) -> io::Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_CLOEXEC)
        .open(path)?;
    lock_volume(&file, path, VOLUME_LOCK_WAIT)?;
    Ok(file)
}

/// Takes the exclusive lock of an opened volume file, waiting up to `wait` for a store operation
/// that holds it to finish, and proves the path still names the file that was opened.
///
/// # Errors
///
/// Returns `WouldBlock` when the lock is still held after `wait`, which is a live machine's, and
/// `NotFound` when the file was removed or replaced while the launch waited.
pub fn lock_volume(file: &File, path: &Path, wait: Duration) -> io::Result<()> {
    let deadline = Instant::now() + wait;
    loop {
        match file.try_lock() {
            Ok(()) => break,
            Err(std::fs::TryLockError::WouldBlock) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(std::fs::TryLockError::WouldBlock) => return Err(io::ErrorKind::WouldBlock.into()),
            Err(std::fs::TryLockError::Error(error)) => return Err(error),
        }
    }
    // A destroy holds the same lock while it removes the file, so the lock a launch waited for may
    // belong to a disk that no longer exists.
    let held = file.metadata()?;
    match std::fs::metadata(path) {
        Ok(named) if named.dev() == held.dev() && named.ino() == held.ino() => Ok(()),
        Ok(_) => Err(io::ErrorKind::NotFound.into()),
        Err(error) => Err(error),
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
        let second = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&file)
            .expect("second handle");
        assert_eq!(
            lock_volume(&second, &file, Duration::from_millis(100))
                .expect_err("a second machine is refused a volume another one holds")
                .kind(),
            io::ErrorKind::WouldBlock
        );
        drop(opened);
        store
            .try_lock()
            .expect("the lock goes with the machine's descriptor");
    }

    /// SOMA#234: the store holds the lock for one repair; the launch waits it out.
    #[test]
    fn a_store_operation_that_holds_the_lock_for_a_moment_does_not_fail_the_launch() {
        let directory = tempfile::tempdir().expect("temp dir");
        let file = directory.path().join("volume.img");
        std::fs::write(&file, b"a volume").expect("write");
        let store = std::fs::File::open(&file).expect("store handle");
        store.try_lock().expect("the store's repair holds the lock");
        let release = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(300));
            drop(store);
        });
        let opened = open(&file);
        release.join().expect("release");
        opened.expect("the launch opens the volume once the repair is done");
    }

    #[test]
    fn a_volume_removed_while_the_launch_waited_is_refused() {
        let directory = tempfile::tempdir().expect("temp dir");
        let file = directory.path().join("volume.img");
        std::fs::write(&file, b"a volume").expect("write");
        let launch = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&file)
            .expect("open");
        let store = std::fs::File::open(&file).expect("store handle");
        store
            .try_lock()
            .expect("the store's destroy holds the lock");
        let path = file.clone();
        let destroy = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(200));
            std::fs::remove_file(&path).expect("destroy removes the file");
            drop(store);
        });
        let locked = lock_volume(&launch, &file, Duration::from_secs(5));
        destroy.join().expect("destroy");
        assert_eq!(
            locked.expect_err("a removed volume is never served").kind(),
            io::ErrorKind::NotFound
        );
    }
}
