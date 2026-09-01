use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{self, Read as _, Write as _},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use soma::{MAX_STATE_RECORD_BYTES, StateRecord, StateRevision, StateStoreFailure};

use super::{
    failure::{capacity_exceeded, conflict, corrupt, unavailable},
    filesystem::{
        reject_unsafe_existing_file, require_single_link, require_single_link_metadata,
        set_create_file_mode, set_file_permissions, sync_directory,
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
) -> Result<(), StateStoreFailure> {
    let target = revision_path_from_directory(directory, revision);
    let temp = create_temp(directory, record)?;
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
    sync_directory(directory)?;
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
                file.write_all(record.as_bytes())
                    .map_err(|_| unavailable())?;
                set_file_permissions(&file)?;
                // The record's bytes and its length are what a later read needs; the inode was
                // journalled with the exclusive create that made it, and its mode came from
                // that same create. `sync_all` would additionally commit the timestamps, which
                // nothing here reads.
                file.sync_data().map_err(|_| unavailable())?;
                return Ok(path);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(_) => return Err(unavailable()),
        }
    }
    Err(capacity_exceeded())
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
