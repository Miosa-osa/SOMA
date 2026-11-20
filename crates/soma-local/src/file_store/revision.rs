use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{self, Read as _, Write as _},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use soma::{MAX_STATE_RECORD_BYTES, StateRecord, StateRevision, StateStoreFailure};

use super::{
    barrier::SyncBarrier,
    failure::{capacity_exceeded, conflict, corrupt, unavailable},
    filesystem::{
        reject_unsafe_existing_file, require_single_link, require_single_link_metadata,
        set_create_file_mode, set_file_permissions, sync_record, sync_root,
    },
    layout::{is_valid_temp_name, parse_revision_name, revision_path_from_directory},
};

const MAX_DIRECTORY_ENTRIES: usize = 1_024;
static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

pub(super) fn scan_revisions(
    directory: &Path,
) -> Result<BTreeMap<StateRevision, PathBuf>, StateStoreFailure> {
    let mut revision_paths = Vec::new();
    let mut temporary_paths = Vec::new();
    let mut entries = 0_usize;
    for entry in fs::read_dir(directory).map_err(|_| unavailable())? {
        entries = entries.checked_add(1).ok_or_else(capacity_exceeded)?;
        if entries > MAX_DIRECTORY_ENTRIES {
            return Err(capacity_exceeded());
        }
        let entry = entry.map_err(|_| unavailable())?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            return Err(corrupt());
        };
        let file_type = entry.file_type().map_err(|_| unavailable())?;
        if is_valid_temp_name(name) {
            if file_type.is_dir() {
                return Err(corrupt());
            }
            temporary_paths.push(entry.path());
            continue;
        }
        let revision = parse_revision_name(name)?;
        if file_type.is_symlink() || !file_type.is_file() {
            return Err(corrupt());
        }
        revision_paths.push((revision, entry.path()));
    }
    for path in temporary_paths {
        fs::remove_file(path).map_err(|_| unavailable())?;
    }
    let mut revisions = BTreeMap::new();
    for (revision, path) in revision_paths {
        require_single_link(&path)?;
        if revisions.insert(revision, path).is_some() {
            return Err(corrupt());
        }
    }
    Ok(revisions)
}

pub(super) fn read_record(path: &Path) -> Result<StateRecord, StateStoreFailure> {
    reject_unsafe_existing_file(path)?;
    let file = File::open(path).map_err(|_| unavailable())?;
    let metadata = file.metadata().map_err(|_| unavailable())?;
    require_single_link_metadata(&metadata)?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_STATE_RECORD_BYTES as u64
    {
        return Err(corrupt());
    }
    let expected_len = usize::try_from(metadata.len()).map_err(|_| corrupt())?;
    let mut bytes = Vec::with_capacity(expected_len);
    file.take(MAX_STATE_RECORD_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| unavailable())?;
    if bytes.len() != expected_len || bytes.len() > MAX_STATE_RECORD_BYTES {
        return Err(corrupt());
    }
    StateRecord::from_bytes(bytes).map_err(|_| corrupt())
}

pub(super) fn commit_revision(
    directory: &Path,
    revision: StateRevision,
    record: &StateRecord,
    barrier: &SyncBarrier,
) -> Result<(), StateStoreFailure> {
    let target = revision_path_from_directory(directory, revision);
    let temp = create_temp(directory, record)?;
    // The record's bytes must be durable before anything can publish them, so a
    // crash cannot leave a link pointing at a record that was never written.
    // Group commit puts one sync behind every writer that arrived with this one.
    if let Err(failure) = barrier.commit(|| sync_record(directory, &temp)) {
        let _ = fs::remove_file(&temp);
        return Err(failure);
    }
    match fs::hard_link(&temp, &target) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            let _ = fs::remove_file(&temp);
            return Err(conflict());
        }
        Err(_) => {
            let _ = fs::remove_file(&temp);
            return Err(unavailable());
        }
    }
    // The temp name is retired before the directory is made durable, so one commit publishes
    // the link and removes the name. A crash before that commit leaves either nothing, or the
    // link with its temp still present, and the temp sweep in `scan_revisions` recovers the
    // second case on the next read: `publication_recovers_when_crash_leaves_the_committed_temp_link`
    // proves it. A second directory commit would therefore buy no state that is not already
    // recovered, and it is the most expensive syscall on this path.
    fs::remove_file(&temp).map_err(|_| unavailable())?;
    // The link is what publishes the record, so this write is not durable until
    // that link is. One sync per batch covers every writer that arrived with it.
    barrier.commit(|| sync_root(directory))?;
    require_single_link(&target)?;
    Ok(())
}

fn create_temp(directory: &Path, record: &StateRecord) -> Result<PathBuf, StateStoreFailure> {
    for _ in 0..32 {
        let nonce = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let name = format!(".tmp-{:010}-{nonce:020}", std::process::id());
        let path = directory.join(name);
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        set_create_file_mode(&mut options);
        match options.open(&path) {
            Ok(mut file) => {
                let written = file
                    .write_all(record.as_bytes())
                    .map_err(|_| unavailable())
                    .and_then(|()| set_file_permissions(&file));
                // A full filesystem fails the write after the name already exists, so the name
                // goes with the failure: left behind, it is an empty file in a directory that
                // names an Instance with no record (SOMA#127).
                if let Err(failure) = written {
                    drop(file);
                    let _ = fs::remove_file(&path);
                    return Err(failure);
                }
                // No sync here: `commit_revision` makes these bytes durable through the
                // barrier before the link can publish them, which is a stronger place for it
                // because one sync then covers every writer in the batch.
                return Ok(path);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(_) => return Err(unavailable()),
        }
    }
    Err(capacity_exceeded())
}

/// Removes every revision of one Instance, the current one last, and makes the removal durable.
///
/// Older revisions go first because the newest name is the record: removing it first would let
/// a crash part way leave an older revision as the current one, and an older revision of a
/// terminal record is a machine that was still running. Superseded revisions normally do not
/// exist, since every replacement prunes them, so the extra sync is paid only after a crash.
///
/// The removal is synced before it is reported because a released identity may be created
/// again, and a terminal record that came back after a crash would then hide the new one.
pub(super) fn remove_revisions(
    directory: &Path,
    revisions: &BTreeMap<StateRevision, PathBuf>,
    barrier: &SyncBarrier,
) -> Result<(), StateStoreFailure> {
    let Some((_, current)) = revisions.last_key_value() else {
        return Ok(());
    };
    if revisions.len() > 1 {
        for path in revisions.values().filter(|path| *path != current) {
            fs::remove_file(path).map_err(|_| unavailable())?;
        }
        barrier.commit(|| sync_root(directory))?;
    }
    fs::remove_file(current).map_err(|_| unavailable())?;
    barrier.commit(|| sync_root(directory))
}

pub(super) fn prune_superseded(
    revisions: &BTreeMap<StateRevision, PathBuf>,
    committed: StateRevision,
) {
    for (revision, path) in revisions {
        if *revision != committed {
            let _ = fs::remove_file(path);
        }
    }
}
