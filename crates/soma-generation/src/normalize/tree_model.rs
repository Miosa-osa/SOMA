use std::collections::{BTreeMap, BTreeSet};

use soma::OciDigest;

use super::entry::{GuestPath, Metadata};
use crate::{NormalizeError, NormalizeErrorKind, NormalizePhase};

#[derive(Clone, Debug)]
pub(super) struct FileNode {
    pub(super) inode: u64,
    pub(super) metadata: Metadata,
    pub(super) digest: OciDigest,
    pub(super) size: u64,
}

#[derive(Clone, Debug)]
pub(super) enum Node {
    Directory(Metadata),
    Regular(FileNode),
    Symlink { metadata: Metadata, target: Vec<u8> },
    Fifo(Metadata),
}

pub(super) struct TreeStats {
    pub(super) entry_count: u32,
    pub(super) logical_file_bytes: u64,
    pub(super) content_blob_count: u32,
    pub(super) content_blob_bytes: u64,
}

pub(super) fn stats(entries: &BTreeMap<GuestPath, Node>) -> Result<TreeStats, NormalizeError> {
    let entry_count = u32::try_from(entries.len()).map_err(|_| limit())?;
    let mut inodes = BTreeSet::new();
    let mut logical_file_bytes = 0_u64;
    let mut contents = BTreeMap::<String, u64>::new();
    for node in entries.values() {
        if let Node::Regular(file) = node {
            if inodes.insert(file.inode) {
                logical_file_bytes = logical_file_bytes
                    .checked_add(file.size)
                    .ok_or_else(limit)?;
            }
            match contents.insert(file.digest.as_str().to_owned(), file.size) {
                Some(size) if size != file.size => return Err(integrity()),
                _ => {}
            }
        }
    }
    let content_blob_count = u32::try_from(contents.len()).map_err(|_| limit())?;
    let content_blob_bytes = contents
        .values()
        .try_fold(0_u64, |sum, size| sum.checked_add(*size))
        .ok_or_else(limit)?;
    Ok(TreeStats {
        entry_count,
        logical_file_bytes,
        content_blob_count,
        content_blob_bytes,
    })
}

const fn integrity() -> NormalizeError {
    NormalizeError::new(NormalizePhase::ApplyLayer, NormalizeErrorKind::Integrity)
}

const fn limit() -> NormalizeError {
    NormalizeError::new(
        NormalizePhase::ApplyLayer,
        NormalizeErrorKind::LimitExceeded,
    )
}
