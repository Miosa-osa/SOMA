//! Sharded head directories.
//!
//! Creating a head is an exclusive create and an unlink inside one directory, and both take
//! that directory's inode lock. A cohort of a hundred launches against one directory therefore
//! queues on it: measured at concurrency one hundred, `create` cost 1796 us and `unlink` 1038
//! us against one directory, and 50 us and 19 us against sixteen.
//!
//! A sharded root holds `shards` directories named `h00` upwards, and every caller takes one of
//! them. Selection starts at a per-process offset so that a cohort of single-launch processes
//! spreads over the shards rather than every process starting at `h00`, and advances within a
//! process so that a cohort launched from one process spreads as well.
//!
//! Sharding alone does not make a cohort faster. It removes one of the two objects a cohort
//! serializes on; the other is the overlay template, which the Linux-only `fan` module removes.

use std::fs::File;
use std::io;
use std::num::NonZeroUsize;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Shard count used when an operator names no other, measured as `d16`.
pub const DEFAULT_HEAD_SHARDS: usize = 16;

/// Highest shard count accepted, so a mistyped configuration cannot create a million
/// directories on the reflink volume.
pub const MAX_HEAD_SHARDS: usize = 1024;

/// The name of one shard directory inside the head root.
#[must_use]
pub fn shard_name(index: usize) -> String {
    format!("h{index:02}")
}

/// Creates every shard directory under `root`.
///
/// Idempotent, so an operator can run it before a cohort and a launcher can call it on a root
/// that is already prepared.
///
/// # Errors
///
/// Returns the first directory that could not be created.
pub fn create_shards(root: &Path, shards: NonZeroUsize) -> io::Result<()> {
    let shards = clamp(shards);
    for index in 0..shards {
        std::fs::create_dir_all(root.join(shard_name(index)))?;
    }
    Ok(())
}

/// Opens one shard of `root`, creating the shard set if it is not there yet.
///
/// The returned descriptor is the capability a head is created under, exactly as an unsharded
/// head directory descriptor was.
///
/// # Errors
///
/// Returns the first failure to create or open a shard directory.
pub fn open_shard(root: &Path, shards: NonZeroUsize) -> io::Result<File> {
    let shards = clamp(shards);
    let path = root.join(shard_name(next_index(shards)));
    match File::open(&path) {
        Ok(file) => Ok(file),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            create_shards(root, NonZeroUsize::new(shards).unwrap_or(NonZeroUsize::MIN))?;
            File::open(&path)
        }
        Err(error) => Err(error),
    }
}

fn clamp(shards: NonZeroUsize) -> usize {
    shards.get().min(MAX_HEAD_SHARDS)
}

/// The next shard index for this process.
///
/// A launch is usually its own process, so a counter that started at zero everywhere would put
/// every launch of a cohort on `h00` and shard nothing. The starting offset therefore comes
/// from the process identity and the clock, and the counter only spreads launches inside one
/// process.
fn next_index(shards: usize) -> usize {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    process_offset().wrapping_add(NEXT.fetch_add(1, Ordering::Relaxed)) % shards
}

/// A per-process starting offset for anything that spreads work over a fixed set of objects.
///
/// Derived once from the process identity and the clock, so two processes started in the same
/// second still start in different places.
pub(crate) fn process_offset() -> usize {
    static OFFSET: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *OFFSET.get_or_init(|| {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.subsec_nanos() as usize);
        (std::process::id() as usize).wrapping_mul(2_654_435_761) ^ nanos
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_every_shard_and_is_idempotent() {
        let root = tempfile::tempdir().expect("tempdir");
        let shards = NonZeroUsize::new(4).expect("nonzero");
        create_shards(root.path(), shards).expect("create");
        create_shards(root.path(), shards).expect("again");
        for index in 0..4 {
            assert!(root.path().join(shard_name(index)).is_dir());
        }
    }

    #[test]
    fn opens_a_shard_of_a_root_that_was_never_prepared() {
        let root = tempfile::tempdir().expect("tempdir");
        let shards = NonZeroUsize::new(3).expect("nonzero");
        let file = open_shard(&root.path().join("fresh"), shards).expect("open");
        assert!(file.metadata().expect("metadata").is_dir());
    }

    #[test]
    fn selection_spreads_over_every_shard_within_one_process() {
        let root = tempfile::tempdir().expect("tempdir");
        let shards = NonZeroUsize::new(8).expect("nonzero");
        create_shards(root.path(), shards).expect("create");
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..64 {
            seen.insert(next_index(8));
        }
        assert_eq!(seen.len(), 8);
    }

    #[test]
    fn a_shard_count_above_the_maximum_is_clamped() {
        let root = tempfile::tempdir().expect("tempdir");
        let shards = NonZeroUsize::new(MAX_HEAD_SHARDS + 7).expect("nonzero");
        create_shards(root.path(), shards).expect("create");
        assert!(root.path().join(shard_name(MAX_HEAD_SHARDS - 1)).is_dir());
        assert!(!root.path().join(shard_name(MAX_HEAD_SHARDS)).is_dir());
    }
}
