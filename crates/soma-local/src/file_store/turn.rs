//! The one turn at addressing an Instance's machine, shared by every process on this root.
//!
//! The turn is an advisory lock on a file named for the Instance. The kernel drops it when its
//! holder dies, which is what still lets an Executing record a dead process left be recovered,
//! while a second command from a live one waits instead of taking the first for a crash
//! (SOMA#243).
//!
//! The file exists only while someone holds or waits for the turn: the holder removes the name
//! before it unlocks. A waiter that then wins the lock on the removed file sees the name no
//! longer leads to it, and opens the name again, so exactly one holder has the file the name
//! leads to.

use std::{
    fs::{self, File, OpenOptions},
    io,
    path::{Path, PathBuf},
};

use soma::{InstanceId, InstanceTurn, StateStoreFailure};

use super::{
    failure::{corrupt, unavailable},
    filesystem::{reject_unsafe_existing_file, set_create_file_mode, set_file_permissions},
};

pub(super) fn take(
    locks: &Path,
    instance_id: &InstanceId,
) -> Result<InstanceTurn, StateStoreFailure> {
    let path = turn_path(locks, instance_id);
    loop {
        reject_unsafe_existing_file(&path)?;
        let mut options = OpenOptions::new();
        options.create(true).read(true).write(true);
        set_create_file_mode(&mut options);
        let file = options.open(&path).map_err(|_| unavailable())?;
        set_file_permissions(&file)?;
        file.lock().map_err(|_| unavailable())?;
        if leads_to(&path, &file)? {
            return Ok(InstanceTurn::new(HeldTurn { file, path }));
        }
    }
}

pub(super) fn turn_path(locks: &Path, instance_id: &InstanceId) -> PathBuf {
    locks.join(format!("turn-{}.lock", instance_id.as_str()))
}

struct HeldTurn {
    file: File,
    path: PathBuf,
}

impl Drop for HeldTurn {
    fn drop(&mut self) {
        // Windows cannot remove a name another handle has open; there the file stays, and the
        // next holder reuses it.
        if cfg!(unix) {
            let _ = fs::remove_file(&self.path);
        }
        let _ = self.file.unlock();
    }
}

/// Whether `path` still names the file this caller holds the lock on.
fn leads_to(path: &Path, file: &File) -> Result<bool, StateStoreFailure> {
    let named = match fs::symlink_metadata(path) {
        Ok(named) => named,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(_) => return Err(unavailable()),
    };
    if named.file_type().is_symlink() || !named.is_file() {
        return Err(corrupt());
    }
    let held = file.metadata().map_err(|_| unavailable())?;
    Ok(same_file(&named, &held))
}

#[cfg(unix)]
fn same_file(named: &fs::Metadata, held: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt as _;

    named.dev() == held.dev() && named.ino() == held.ino()
}

/// The name is never removed where [`HeldTurn`] cannot remove it, so it always leads to the file.
#[cfg(not(unix))]
const fn same_file(_named: &fs::Metadata, _held: &fs::Metadata) -> bool {
    true
}
