//! Taking an entry out of service and deleting it, with nothing half done at any step.
//!
//! An entry leaves in three steps. Its cached value leaves the cache and the entry is renamed into
//! the holding directory, under the cache's lock, so a lookup sees either the whole entry or
//! none of it. The value is then dropped, which closes this process's handles, and only an entry
//! no descriptor in this process still reaches is deleted. A crash after the rename leaves a whole
//! entry in the holding directory, which no lookup reads and the next applied prune deletes.

use std::{
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use super::super::{
    CacheKey,
    cache::{Cache, Release, Retained},
};
use super::{policy::Decision, survey::HOLDING};
use crate::PruneReason;

/// One entry moved out of the root, with the value that left the cache beside it.
pub(super) struct Moved<T> {
    key: CacheKey,
    from: PathBuf,
    to: PathBuf,
    value: Option<T>,
}

/// The holding directory, created when absent. A link in its place is refused, not followed.
pub(super) fn holding_directory(root: &Path) -> Result<PathBuf, String> {
    let holding = root.join(HOLDING);
    match std::fs::symlink_metadata(&holding) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            Err(format!("{} is not a directory", holding.display()))
        }
        Ok(_) => Ok(holding),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => std::fs::create_dir(&holding)
            .map(|()| holding.clone())
            .map_err(|error| format!("{}: {error}", holding.display())),
        Err(error) => Err(format!("{}: {error}", holding.display())),
    }
}

/// Releases the cache's handles to one entry and moves it into the holding directory.
pub(super) fn take_out<T: Clone + Retained>(
    cache: &Cache<T>,
    key: CacheKey,
    entry: &Path,
    holding: &Path,
) -> Result<Moved<T>, Decision> {
    let name = entry
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    // A name of its own, so an entry that was prepared, pruned, and prepared again under the same
    // name never lands on a leftover of its earlier self.
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos());
    let to = holding.join(format!("{name}.{nanos}"));
    match cache.release(&key, entry, &to) {
        Release::Moved(value) => Ok(Moved {
            key,
            from: entry.to_path_buf(),
            to,
            value,
        }),
        Release::Held => Err(held_elsewhere()),
        Release::Failed(error) => Err(Decision::refuse(
            PruneReason::MoveFailed,
            vec![error.to_string()],
        )),
    }
}

/// Moves one entry back into the root and caches its value again.
pub(super) fn put_back<T: Clone + Retained>(
    cache: &Cache<T>,
    moved: Moved<T>,
    mut refusal: Decision,
) -> Decision {
    let Moved {
        key,
        from,
        to,
        value,
    } = moved;
    if let Err(error) = cache.restore(key, value, &from, &to) {
        refusal.detail.push(stranded(&to, &error));
    }
    refusal
}

/// Drops the value, proves this process no longer reaches the entry, and deletes it.
pub(super) fn finish<T: Clone + Retained>(
    cache: &Cache<T>,
    moved: Moved<T>,
) -> Result<(), Decision> {
    let Moved {
        key,
        from,
        to,
        value,
    } = moved;
    // The only handles the cache held. Dropping them is what releases the entry.
    drop(value);
    let open = descriptors_under(&to);
    if open > 0 {
        let mut refusal = Decision::refuse(
            PruneReason::InUse,
            vec![format!(
                "{open} descriptors into it are still open in this process"
            )],
        );
        // The value is gone, so the next lookup verifies and admits the entry again.
        if let Err(error) = cache.restore(key, None, &from, &to) {
            refusal.detail.push(stranded(&to, &error));
        }
        return Err(refusal);
    }
    delete(&to)
}

/// Deletes one entry that is already out of the root.
pub(super) fn delete(path: &Path) -> Result<(), Decision> {
    std::fs::remove_dir_all(path).map_err(|error| {
        Decision::refuse(
            PruneReason::DeleteFailed,
            vec![format!(
                "{} is out of service but not deleted ({error}); a later applied prune deletes it",
                path.display()
            )],
        )
    })
}

pub(super) fn held_elsewhere() -> Decision {
    Decision::refuse(
        PruneReason::InUse,
        vec!["a launch, prepared machine, or prewarm plan holds its Generation".to_owned()],
    )
}

fn stranded(to: &Path, error: &std::io::Error) -> String {
    format!(
        "could not move it back ({error}); it is out of service at {}",
        to.display()
    )
}

/// How many of this process's descriptors reach anything under `path`.
///
/// A descriptor whose link cannot be read closed while the table was read (the table's own
/// descriptor is one), so it reaches nothing. A table that cannot be read at all counts as one
/// open descriptor, because nothing then shows the entry unreached.
fn descriptors_under(path: &Path) -> usize {
    let Ok(descriptors) = std::fs::read_dir("/proc/self/fd") else {
        return 1;
    };
    descriptors
        .filter(|descriptor| {
            descriptor
                .as_ref()
                .ok()
                .and_then(|descriptor| std::fs::read_link(descriptor.path()).ok())
                .is_some_and(|target| target.starts_with(path))
        })
        .count()
}
