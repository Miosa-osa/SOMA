use std::path::{Path, PathBuf};

use soma::{InstanceId, StateRevision, StateStoreFailure};

use super::failure::corrupt;

pub(super) const LOCK_DIRECTORY: &str = ".locks";
const REVISION_DIGITS: usize = 20;
const REVISION_SUFFIX: &str = ".state";
const TEMP_PID_DIGITS: usize = 10;
const TEMP_NONCE_DIGITS: usize = 20;

pub(super) fn parse_revision_name(name: &str) -> Result<StateRevision, StateStoreFailure> {
    if name.len() != REVISION_DIGITS + REVISION_SUFFIX.len() || !name.ends_with(REVISION_SUFFIX) {
        return Err(corrupt());
    }
    let digits = &name[..REVISION_DIGITS];
    if !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(corrupt());
    }
    let value = digits.parse::<u64>().map_err(|_| corrupt())?;
    StateRevision::new(value).map_err(|_| corrupt())
}

pub(super) fn is_valid_temp_name(name: &str) -> bool {
    const PREFIX: &str = ".tmp-";
    let expected_len = PREFIX.len() + TEMP_PID_DIGITS + 1 + TEMP_NONCE_DIGITS;
    if name.len() != expected_len || !name.starts_with(PREFIX) {
        return false;
    }
    let pid_start = PREFIX.len();
    let separator = pid_start + TEMP_PID_DIGITS;
    name.as_bytes().get(separator) == Some(&b'-')
        && name[pid_start..separator]
            .bytes()
            .all(|byte| byte.is_ascii_digit())
        && name[separator + 1..]
            .bytes()
            .all(|byte| byte.is_ascii_digit())
}

pub(super) fn revision_path_from_directory(directory: &Path, revision: StateRevision) -> PathBuf {
    directory.join(format!("{:020}{REVISION_SUFFIX}", revision.get()))
}

pub(super) fn shard_lock_path(locks: &Path, instance_id: &InstanceId) -> PathBuf {
    let shard = &instance_id.as_str()[..2];
    locks.join(format!("{shard}.lock"))
}

#[cfg(test)]
pub(super) fn instance_lock_path(root: &Path, instance_id: &InstanceId) -> PathBuf {
    shard_lock_path(&root.join(LOCK_DIRECTORY), instance_id)
}

#[cfg(test)]
pub(super) fn revision_path(
    root: &Path,
    instance_id: &InstanceId,
    revision: StateRevision,
) -> PathBuf {
    revision_path_from_directory(&root.join(instance_id.as_str()), revision)
}
