//! What a prepared root holds, read without admitting anything.
//!
//! A prune has to classify every entry, including ones a lookup would refuse, so this reads only
//! the small files that name an entry and never decodes or verifies its Generation. Nothing is
//! followed through a link and nothing is written.

use std::{
    io::ErrorKind,
    os::{fd::AsFd as _, unix::fs::OpenOptionsExt as _},
    path::{Path, PathBuf},
    time::SystemTime,
};

use soma::GenerationId;

use super::super::{
    CANDIDATE, GENERATION_ID, GENERATION_ID_BYTES, MAX_REFERENCE_BYTES, REFERENCE,
    entry::read_bounded,
};
use crate::PruneError;

/// The directory inside the root an entry is moved into before it is deleted.
///
/// It is a child of the root, so a move into it never crosses a filesystem, and an entry inside
/// it is one level too deep for a lookup to read its reference, so a lookup never sees it.
pub(super) const HOLDING: &str = ".pruning";

/// One entry as it was found.
pub(super) struct Surveyed {
    /// The directory name, or for an entry in the holding directory, `.pruning/<name>`.
    pub(super) name: String,
    pub(super) path: PathBuf,
    pub(super) reference: Option<String>,
    pub(super) state: State,
    pub(super) bytes: Option<ExtentBytes>,
}

/// Whether an entry is a ready Generation.
pub(super) enum State {
    /// `generation.id` names a Generation, published at `at`.
    Certified { id: GenerationId, at: SystemTime },
    /// A Candidate with no `generation.id` yet.
    Uncertified,
    /// The entry cannot be read well enough to say.
    Unreadable(String),
}

/// What an entry's files hold on the filesystem.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct ExtentBytes {
    /// Bytes in extents no other file shares.
    pub(super) unshared: u64,
    /// Bytes in every mapped extent.
    pub(super) mapped: u64,
}

/// Every entry in `root`, and every entry an earlier prune left in its holding directory.
pub(super) fn survey(root: &Path) -> Result<(Vec<Surveyed>, Vec<Surveyed>), PruneError> {
    let unavailable = |reason: &str| PruneError::StoreUnavailable(reason.to_owned());
    match std::fs::symlink_metadata(root) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(unavailable("the prepared root is a link"));
        }
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => return Err(unavailable("the prepared root is not a directory")),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Err(unavailable("the prepared root does not exist"));
        }
        Err(_) => return Err(unavailable("the prepared root cannot be described")),
    }
    let entries = children(root, false)
        .map_err(|_| unavailable("the prepared root cannot be read"))?
        .into_iter()
        .filter(|(name, _)| name != HOLDING)
        .map(|(name, path)| read(name, path))
        .collect();
    let holding = root.join(HOLDING);
    let leftovers = match std::fs::symlink_metadata(&holding) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(unavailable("the holding directory is a link"));
        }
        Ok(_) => children(&holding, true)
            .map_err(|_| unavailable("the holding directory cannot be read"))?
            .into_iter()
            .map(|(name, path)| read(format!("{HOLDING}/{name}"), path))
            .collect(),
        Err(error) if error.kind() == ErrorKind::NotFound => Vec::new(),
        Err(_) => return Err(unavailable("the holding directory cannot be described")),
    };
    Ok((entries, leftovers))
}

/// The directories and links directly under `directory`, by name.
///
/// A plain file is not an entry. A link is kept so it can be reported, because a lookup refuses
/// a linked entry and an operator should hear about one. With `directories_only`, links are
/// dropped too: the holding directory only ever receives directories.
fn children(directory: &Path, directories_only: bool) -> std::io::Result<Vec<(String, PathBuf)>> {
    let mut found = Vec::new();
    for child in std::fs::read_dir(directory)? {
        let child = child?;
        let kind = child.file_type()?;
        if kind.is_dir() || (kind.is_symlink() && !directories_only) {
            let name = child.file_name().to_string_lossy().into_owned();
            found.push((name, child.path()));
        }
    }
    // Directory order is whatever the filesystem returns; the plan is ordered by name instead.
    found.sort();
    Ok(found)
}

fn read(name: String, path: PathBuf) -> Surveyed {
    let linked =
        std::fs::symlink_metadata(&path).map_or(true, |metadata| metadata.file_type().is_symlink());
    if linked {
        return Surveyed {
            name,
            path,
            reference: None,
            state: State::Unreadable("the entry is a link".to_owned()),
            bytes: None,
        };
    }
    let (reference, state) = classify(&path);
    let bytes = measure(&path);
    Surveyed {
        name,
        path,
        reference,
        state,
        bytes,
    }
}

fn classify(entry: &Path) -> (Option<String>, State) {
    let unreadable = |reason: &str| State::Unreadable(reason.to_owned());
    let reference = match small_text(&entry.join(REFERENCE), MAX_REFERENCE_BYTES) {
        Ok(Some(text)) if !text.is_empty() => text,
        Ok(_) => return (None, unreadable("the entry names no reference")),
        Err(reason) => return (None, unreadable(&format!("reference: {reason}"))),
    };
    let path = entry.join(GENERATION_ID);
    let state = match std::fs::symlink_metadata(&path) {
        Err(error) if error.kind() == ErrorKind::NotFound => {
            if entry.join(CANDIDATE).is_file() {
                State::Uncertified
            } else {
                unreadable("the entry holds neither a generation.id nor a Candidate")
            }
        }
        Err(_) => unreadable("generation.id cannot be described"),
        Ok(metadata) if metadata.file_type().is_symlink() => unreadable("generation.id is a link"),
        Ok(metadata) => match (small_text(&path, GENERATION_ID_BYTES), metadata.modified()) {
            (Ok(Some(text)), Ok(at)) => GenerationId::new(text).map_or_else(
                |_| unreadable("generation.id does not name a Generation"),
                |id| State::Certified { id, at },
            ),
            _ => unreadable("generation.id cannot be read"),
        },
    };
    (Some(reference), state)
}

/// The trimmed text of one small file, nothing when it is absent, or why it cannot be read.
fn small_text(path: &Path, limit: u64) -> Result<Option<String>, &'static str> {
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("cannot be described"),
        Ok(metadata) if metadata.file_type().is_symlink() => return Err("is a link"),
        Ok(_) => {}
    }
    let bytes = read_bounded(path, limit).ok_or("is unreadable or oversized")?;
    let text = String::from_utf8(bytes).map_err(|_| "is not text")?;
    Ok(Some(text.trim().to_owned()))
}

/// Sums the extents of every regular file under `entry`, or nothing when one cannot be mapped.
///
/// A partial sum would understate what a removal frees and look like a measurement, so one file
/// that cannot be mapped makes the whole entry unmeasured.
fn measure(entry: &Path) -> Option<ExtentBytes> {
    let mut total = ExtentBytes::default();
    let mut pending = vec![entry.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for child in std::fs::read_dir(&directory).ok()? {
            let child = child.ok()?;
            let kind = child.file_type().ok()?;
            if kind.is_dir() {
                pending.push(child.path());
            } else if kind.is_file() {
                let file = std::fs::OpenOptions::new()
                    .read(true)
                    .custom_flags(libc::O_NOFOLLOW)
                    .open(child.path())
                    .ok()?;
                let summary = soma_storage::fiemap::summarize(file.as_fd()).ok()?;
                total.unshared = total.unshared.saturating_add(summary.unshared_bytes());
                total.mapped = total.mapped.saturating_add(summary.mapped_bytes);
            }
            // A link or a special file holds no extents of its own.
        }
    }
    Some(total)
}
